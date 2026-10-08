//! Account support reports are a separate email outbox, never chat history.
use crate::{CoreResult, store::CoreStore};
use finite_mail::{FileOutboxMailer, MailTransport, ResendMailer, TextEmail};
use serde::{Deserialize, Serialize};
use std::{env, sync::Arc, time::Duration};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupportInput {
    pub idempotency_key: String,
    pub project_id: Option<String>,
    pub message: String,
    // These bind submission to the addresses the user reviewed. Neither grants authority.
    pub support_email: String,
    pub reply_to: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportReceipt {
    pub id: String,
    pub status: String,
}

pub(crate) enum SubmitSupportResult {
    Receipt(SupportReceipt),
    Conflict,
    RateLimited,
}

pub(crate) struct SupportDelivery {
    pub id: String,
    pub recipient: String,
    pub reply_to: String,
    pub project_id: Option<String>,
    pub message: String,
    pub lease_token: String,
}

#[derive(Clone)]
pub struct SupportService {
    pub email: String,
    pub mailer: Arc<dyn MailTransport>,
}

/// Deliberately no built-in Finite address: self-hosted installs must opt in.
impl SupportService {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(email) = env::var("FINITE_SUPPORT_EMAIL")
            .ok()
            .filter(|s| !s.trim().is_empty())
        else {
            return Ok(None);
        };
        anyhow::ensure!(
            valid_email(&email),
            "FINITE_SUPPORT_EMAIL must be a single email address"
        );
        let mailer: Arc<dyn MailTransport> = match env::var("FINITE_SUPPORT_MAILER").as_deref() {
            Ok("resend") => {
                let from = env::var("FINITE_SUPPORT_MAIL_FROM")?;
                anyhow::ensure!(
                    valid_email(&from),
                    "FINITE_SUPPORT_MAIL_FROM must be a single email address"
                );
                let key = env::var(finite_mail::RESEND_API_KEY_ENV_VAR)?;
                anyhow::ensure!(!key.trim().is_empty(), "support mail API key is empty");
                Arc::new(ResendMailer::new(key, from))
            }
            Ok("dev") => Arc::new(FileOutboxMailer::new(
                env::var("FINITE_SUPPORT_OUTBOX_DIR")?.into(),
            )?),
            // The contact can be displayed before server-side sending is enabled.
            Err(env::VarError::NotPresent) => return Ok(None),
            _ => anyhow::bail!("FINITE_SUPPORT_MAILER must be resend or dev"),
        };
        Ok(Some(Self { email, mailer }))
    }

    pub fn start_worker(&self, store: CoreStore) -> tokio::task::JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                for _ in 0..20 {
                    match service.deliver_next(&store).await {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(error) => {
                            tracing::error!(%error, "support outbox worker failed");
                            break;
                        }
                    }
                }
            }
        })
    }

    pub(crate) async fn deliver_next(&self, store: &CoreStore) -> CoreResult<bool> {
        let Some(delivery) = store.claim_support_delivery().await? else {
            return Ok(false);
        };
        let id = delivery.id.clone();
        let lease = delivery.lease_token.clone();
        let mailer = self.mailer.clone();
        let result = tokio::task::spawn_blocking(move || {
            let subject = format!("[Finite support {}] Help requested", delivery.id);
            let text = format!(
                "Reply email: {}\nAgent project: {}\n\n{}",
                delivery.reply_to,
                delivery.project_id.as_deref().unwrap_or("Not specified"),
                delivery.message
            );
            mailer.send_text_email_with_reply_to(
                &delivery.id,
                &TextEmail {
                    to: &delivery.recipient,
                    subject: &subject,
                    text: &text,
                },
                &delivery.reply_to,
            )
        })
        .await;
        let sent = matches!(result, Ok(Ok(())));
        if !sent {
            // Provider errors can echo report text or addresses; do not log them.
            tracing::warn!(request_id = %id, "support email delivery failed; retained for retry");
        }
        store.finish_support_delivery(&id, &lease, sent).await?;
        Ok(true)
    }
}

pub fn valid_email(value: &str) -> bool {
    if value.len() > 254 || !value.is_ascii() || value.bytes().any(|c| c <= b' ' || c >= 127) {
        return false;
    }
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&c))
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}
