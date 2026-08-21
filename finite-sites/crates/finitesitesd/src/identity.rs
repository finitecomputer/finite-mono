//! Identity Directory client for the one cross-service question that remains:
//! NIP-05 name resolution (and Core account lookup for reconciliation).
//! Authorization is never delegated; see docs/auth-kernel.md.

use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct IdentityAuthority {
    base_url: String,
}

#[derive(Debug, Clone)]
pub struct CoreAccountAuthority {
    base_url: String,
    service_token: String,
}

impl IdentityAuthority {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    pub fn resolve_nip05(&self, name: &str) -> Result<Option<Nip05Resolution>, String> {
        let url = format!("{}/api/v1/nip05-resolution", self.base_url);
        let response = ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_json(serde_json::json!({ "name": name }));
        match response {
            // Reconciliation intentionally offers every legacy email grant to
            // the Finite NIP-05 resolver. Ordinary mailbox addresses are valid
            // Sites grants but outside that resolver's domain, which the
            // Authority reports as 400; a missing Finite name is 404. Neither
            // is a reconciliation failure or authority to change the grant.
            Err(ureq::Error::Status(400 | 404, _)) => Ok(None),
            Err(error) => Err(format!("identity authority request failed: {error}")),
            Ok(response) => response
                .into_json()
                .map(Some)
                .map_err(|error| format!("identity authority returned invalid json: {error}")),
        }
    }
}

impl CoreAccountAuthority {
    pub fn new(base_url: impl Into<String>, service_token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            service_token: service_token.into(),
        }
    }

    pub fn managed_agent_account(
        &self,
        managed_agent_email: &str,
    ) -> Result<Option<ManagedAgentAccount>, String> {
        let url = format!("{}/api/core/v1/brain/agent-account", self.base_url);
        let response = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.service_token))
            .set("Content-Type", "application/json")
            .send_json(serde_json::json!({
                "managedAgentEmail": managed_agent_email,
            }));
        match response {
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(error) => Err(format!("Core account authority request failed: {error}")),
            Ok(response) => {
                let account: ManagedAgentAccount = response.into_json().map_err(|error| {
                    format!("Core account authority returned invalid json: {error}")
                })?;
                if account.status != "active"
                    || account.managed_agent_email != managed_agent_email
                    || account.workos_user_id.trim().is_empty()
                {
                    return Err(
                        "Core account authority returned an inactive or mismatched Managed Agent"
                            .to_string(),
                    );
                }
                Ok(Some(account))
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Nip05Resolution {
    pub pubkey: String,
    pub kind: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentAccount {
    pub workos_user_id: String,
    pub managed_agent_email: String,
    pub verified_email: String,
    pub status: String,
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn managed_agent_account_uses_core_service_auth_and_exact_agent_email() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            assert_eq!(
                request_line.trim_end(),
                "POST /api/core/v1/brain/agent-account HTTP/1.1"
            );

            let mut content_length = None;
            let mut authorization = None;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                let lower = trimmed.to_ascii_lowercase();
                if let Some(value) = lower.strip_prefix("content-length: ") {
                    content_length = Some(value.parse::<usize>().unwrap());
                }
                if lower.starts_with("authorization: ") {
                    authorization = trimmed.split_once(": ").map(|(_, value)| value.to_string());
                }
            }
            assert_eq!(authorization.as_deref(), Some("Bearer core-secret"));
            let mut body = vec![0; content_length.expect("content-length")];
            reader.read_exact(&mut body).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["managedAgentEmail"], "clanky-123@finite.vip");

            let response = br#"{"workosUserId":"user_123","managedAgentEmail":"clanky-123@finite.vip","verifiedEmail":"paul@finite.vip","status":"active"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                response.len()
            )
            .unwrap();
            stream.write_all(response).unwrap();
        });

        let authority = CoreAccountAuthority::new(format!("http://{address}"), "core-secret");
        let account = authority
            .managed_agent_account("clanky-123@finite.vip")
            .unwrap()
            .unwrap();
        assert_eq!(account.verified_email, "paul@finite.vip");
        server.join().unwrap();
    }

    #[test]
    fn non_finite_mailbox_is_not_a_nip05_reconciliation_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            assert_eq!(
                request_line.trim_end(),
                "POST /api/v1/nip05-resolution HTTP/1.1"
            );

            let mut content_length = None;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                if let Some(value) = trimmed
                    .to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                {
                    content_length = Some(value.parse::<usize>().unwrap());
                }
            }
            let mut body = vec![0; content_length.expect("content-length")];
            reader.read_exact(&mut body).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["name"], "person@example.com");

            let response = br#"{"error":"invalid_finite_nip05_name"}"#;
            write!(
                stream,
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                response.len()
            )
            .unwrap();
            stream.write_all(response).unwrap();
        });

        let authority = IdentityAuthority::new(format!("http://{address}"));
        assert!(
            authority
                .resolve_nip05("person@example.com")
                .unwrap()
                .is_none()
        );
        server.join().unwrap();
    }
}
