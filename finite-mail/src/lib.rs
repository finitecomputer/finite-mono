//! Shared outbound email transport for Finite services.
//!
//! One implementation per delivery mode, used by every service that sends
//! mail (Sites, Identity, Brain) so provider payloads, timeouts, and dev
//! outbox behavior cannot drift apart:
//!
//! - [`ResendMailer`]: sends through the Resend JSON API. The API key comes
//!   from the `RESEND_API_KEY` environment variable so secrets stay in the
//!   service env file, never in argv.
//! - [`FileOutboxMailer`]: writes each email to a file under an outbox
//!   directory and logs it. Local development and tests only.
//!
//! Services keep their own message-text formatting and typed mailer traits;
//! this crate owns only the transport.

use std::path::PathBuf;
use std::time::Duration;

/// Environment variable holding the Resend API key. Referenced by infra env
/// files and NixOS modules; the name is a deployment contract.
pub const RESEND_API_KEY_ENV_VAR: &str = "RESEND_API_KEY";

const RESEND_ENDPOINT: &str = "https://api.resend.com/emails";

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("mail io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("mail send failed: {0}")]
    Send(String),
}

/// One outbound plain-text email. Subject and body are owned by the calling
/// service; `from` is owned by the transport.
pub struct TextEmail<'a> {
    pub to: &'a str,
    pub subject: &'a str,
    pub text: &'a str,
}

/// Transport for one outbound plain-text email.
pub trait MailTransport: Send + Sync {
    fn send_text_email(&self, email: &TextEmail<'_>) -> Result<(), MailError>;

    /// Send with a delivery idempotency key when the provider supports one.
    /// The default ignores the key; provider transports override this to set
    /// their idempotency header. Callers still own durable dedup — the key
    /// only narrows the crash window between delivery and the durable mark.
    fn send_text_email_with_idempotency_key(
        &self,
        idempotency_key: &str,
        email: &TextEmail<'_>,
    ) -> Result<(), MailError> {
        let _ = idempotency_key;
        self.send_text_email(email)
    }
}

// ---- Resend HTTP transport ---------------------------------------------------

pub struct ResendMailer {
    api_key: String,
    from_address: String,
    agent: ureq::Agent,
}

impl ResendMailer {
    pub fn new(api_key: String, from_address: String) -> ResendMailer {
        assert!(!api_key.is_empty() && from_address.contains('@'));
        ResendMailer {
            api_key,
            from_address,
            // Login mail is latency-sensitive; fail fast and let the caller
            // retry rather than hanging the request.
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .build(),
        }
    }

    fn deliver(
        &self,
        idempotency_key: Option<&str>,
        email: &TextEmail<'_>,
    ) -> Result<(), MailError> {
        let payload = resend_payload(&self.from_address, email);
        let mut request = self
            .agent
            .post(RESEND_ENDPOINT)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Accept", "application/json");
        if let Some(idempotency_key) = idempotency_key {
            request = request.set("Idempotency-Key", idempotency_key);
        }
        match request.send_json(payload) {
            Ok(_response) => Ok(()),
            Err(ureq::Error::Status(code, response)) => {
                // Provider error bodies are short JSON; bound the read and
                // log enough to debug deliverability without the API key.
                let body = response
                    .into_string()
                    .unwrap_or_else(|_| "unreadable body".to_string());
                let truncated: String = body.chars().take(500).collect();
                Err(MailError::Send(format!(
                    "provider returned {code}: {truncated}"
                )))
            }
            Err(transport) => Err(MailError::Send(format!("transport error: {transport}"))),
        }
    }
}

/// Build the Resend JSON payload. Split out for tests.
fn resend_payload(from_address: &str, email: &TextEmail<'_>) -> serde_json::Value {
    serde_json::json!({
        "from": from_address,
        "to": [email.to],
        "subject": email.subject,
        "text": email.text,
    })
}

impl MailTransport for ResendMailer {
    fn send_text_email(&self, email: &TextEmail<'_>) -> Result<(), MailError> {
        self.deliver(None, email)
    }

    fn send_text_email_with_idempotency_key(
        &self,
        idempotency_key: &str,
        email: &TextEmail<'_>,
    ) -> Result<(), MailError> {
        self.deliver(Some(idempotency_key), email)
    }
}

// ---- dev file-outbox transport ------------------------------------------------

pub struct FileOutboxMailer {
    outbox_dir: PathBuf,
}

impl FileOutboxMailer {
    pub fn new(outbox_dir: PathBuf) -> Result<FileOutboxMailer, MailError> {
        std::fs::create_dir_all(&outbox_dir)?;
        Ok(FileOutboxMailer { outbox_dir })
    }

    /// Write one email as `To:`/`Subject:`/blank-line/body under the outbox
    /// directory, with a random nonce filename so retries never overwrite.
    /// Returns the written path so callers can log it. An empty `suffix`
    /// yields `<nonce>-<safe-email>.txt`; otherwise
    /// `<nonce>-<safe-email>-<suffix>.txt`.
    pub fn write(&self, email: &TextEmail<'_>, suffix: &str) -> Result<PathBuf, MailError> {
        use std::io::Write as _;

        let nonce = outbox_nonce()?;
        let safe_email: String = email
            .to
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let file_name = if suffix.is_empty() {
            format!("{nonce}-{safe_email}.txt")
        } else {
            format!("{nonce}-{safe_email}-{suffix}.txt")
        };
        let path = self.outbox_dir.join(file_name);
        let mut file = std::fs::File::create(&path)?;
        writeln!(file, "To: {}", email.to)?;
        writeln!(file, "Subject: {}", email.subject)?;
        writeln!(file)?;
        write!(file, "{}", email.text)?;
        eprintln!(
            "dev-mail: email for {} (written to {})",
            email.to,
            path.display()
        );
        Ok(path)
    }
}

fn outbox_nonce() -> Result<String, MailError> {
    let mut bytes = [0u8; 4];
    getrandom::fill(&mut bytes)
        .map_err(|error| MailError::Send(format!("outbox nonce generation failed: {error}")))?;
    Ok(hex::encode(bytes))
}

impl MailTransport for FileOutboxMailer {
    fn send_text_email(&self, email: &TextEmail<'_>) -> Result<(), MailError> {
        self.write(email, "email").map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resend_payload_matches_provider_shape() {
        let payload = resend_payload(
            "Finite Sites <sites@finite.chat>",
            &TextEmail {
                to: "friend@example.com",
                subject: "Your link to hello",
                text: "https://hello.finite.chat/_finite/auth?token=abc",
            },
        );
        assert_eq!(payload["from"], "Finite Sites <sites@finite.chat>");
        assert_eq!(payload["to"][0], "friend@example.com");
        assert_eq!(payload["subject"], "Your link to hello");
        assert!(payload["text"].as_str().unwrap().contains("token=abc"));
    }

    #[test]
    fn resend_api_key_env_var_name_is_stable() {
        assert_eq!(RESEND_API_KEY_ENV_VAR, "RESEND_API_KEY");
    }

    #[test]
    fn file_outbox_writes_envelope_and_body() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = FileOutboxMailer::new(dir.path().to_path_buf()).unwrap();
        let path = outbox
            .write(
                &TextEmail {
                    to: "friend@example.com",
                    subject: "Your link to hello",
                    text: "open me",
                },
                "login-link",
            )
            .unwrap();
        let file_name = path.file_name().unwrap().to_str().unwrap().to_owned();
        assert!(file_name.ends_with("-friend_example_com-login-link.txt"));
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            contents,
            "To: friend@example.com\nSubject: Your link to hello\n\nopen me"
        );
    }

    #[test]
    fn file_outbox_transport_trait_writes_email_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = FileOutboxMailer::new(dir.path().to_path_buf()).unwrap();
        outbox
            .send_text_email(&TextEmail {
                to: "friend@example.com",
                subject: "s",
                text: "t",
            })
            .unwrap();
        let mut entries = std::fs::read_dir(dir.path()).unwrap();
        let name = entries
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .into_string()
            .unwrap();
        assert!(name.ends_with("-friend_example_com-email.txt"));
    }
}
