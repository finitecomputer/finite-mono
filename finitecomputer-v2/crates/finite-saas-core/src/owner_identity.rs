//! The owner's sign-in identity for operator-created Agents.
//!
//! Before an operator creates an Agent for a person who has not signed in,
//! Core finds or creates the WorkOS user for their email (no mailbox access
//! is needed: the user starts unverified) and reads that user's hosted-chat
//! account, which the hosted device mints on first contact. The Agent is
//! then created locked to that chat account, so it admits only its owner.
//!
//! Why the person's later sign-in lands on the same WorkOS user: WorkOS
//! keeps one user per email in an environment, and a new credential for an
//! existing email is attached to that user once WorkOS has verified the
//! inbox; otherwise the sign-in does not complete. Core links a pending
//! account only to a verified WorkOS user. The hosted device keys each chat
//! identity by WorkOS user id, so the chat account is the same one too.

use crate::store::CoreStore;
use crate::{
    CoreError, CoreResult, OperatorCreateAgentInput, OperatorCreateAgentResult, OperatorIdentity,
    OwnerSignInIdentity, normalize_owner_email,
};
use futures_util::future::BoxFuture;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

mod http;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

pub use http::HttpOwnerIdentityBackend;

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// One WorkOS user as the user-management API reports it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkosUserRecord {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub email_verified: bool,
}

/// The external calls the operator path makes. Production uses
/// [`HttpOwnerIdentityBackend`]; tests substitute their own.
pub trait OwnerIdentityBackend: Send + Sync {
    fn workos_users_by_email<'a>(
        &'a self,
        email: &'a str,
    ) -> BoxFuture<'a, CoreResult<Vec<WorkosUserRecord>>>;
    /// Create an unverified WorkOS user for `email`.
    fn create_workos_user<'a>(
        &'a self,
        email: &'a str,
    ) -> BoxFuture<'a, CoreResult<WorkosUserRecord>>;
    fn workos_user<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, CoreResult<Option<WorkosUserRecord>>>;
    /// True when the user has an active membership in the operator org.
    fn operator_membership_active<'a>(
        &'a self,
        workos_user_id: &'a str,
    ) -> BoxFuture<'a, CoreResult<bool>>;
    /// The hosted-chat account id (64 lowercase hex) of this WorkOS user,
    /// minted by the hosted device on first contact.
    fn hosted_chat_account_id<'a>(
        &'a self,
        workos_user_id: &'a str,
    ) -> BoxFuture<'a, CoreResult<String>>;
}

/// What a preview found, without creating anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerIdentityPreview {
    /// The WorkOS user the execute run would use, when one exists already.
    pub existing_workos_user_id: Option<String>,
    pub would_create_workos_user: bool,
    pub would_read_or_mint_chat_identity: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorCreateAgentOutcome {
    pub result: OperatorCreateAgentResult,
    /// Set only for a preview.
    pub owner_identity_preview: Option<OwnerIdentityPreview>,
}

/// Where the owner's sign-in identity comes from, as Core's rows say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerIdentityPlan {
    /// The idempotency key already names a creation: reuse its identity.
    Reuse(OwnerSignInIdentity),
    /// The owner has signed in: their linked WorkOS user is the identity.
    Linked { workos_user_id: String },
    /// Pending or unknown: find or create the WorkOS user for the email.
    Unlinked,
}

#[derive(Clone)]
pub struct OwnerIdentityProvisioner {
    backend: Arc<dyn OwnerIdentityBackend>,
}

impl OwnerIdentityProvisioner {
    pub fn new(backend: Arc<dyn OwnerIdentityBackend>) -> Self {
        Self { backend }
    }

    /// `None` when the hosted device is not configured
    /// (`FC_HOSTED_WEB_DEVICE_URL` and `FINITECHAT_HOSTED_API_TOKEN` both
    /// unset). Operator creation then refuses.
    pub fn from_env() -> CoreResult<Option<Self>> {
        Ok(HttpOwnerIdentityBackend::from_env()?
            .map(|backend| Self::new(Arc::new(backend) as Arc<dyn OwnerIdentityBackend>)))
    }

    /// The single WorkOS user with exactly this email, if any.
    async fn find_user(&self, email: &str) -> CoreResult<Option<String>> {
        let matching: Vec<_> = self
            .backend
            .workos_users_by_email(email)
            .await?
            .into_iter()
            .filter(|user| normalize_owner_email(Some(&user.email)).as_deref() == Some(email))
            .collect();
        match matching.as_slice() {
            [] => Ok(None),
            [user] => Ok(Some(user.id.clone())),
            _ => Err(CoreError::OperatorAgentCreationRefused(
                "WorkOS reports more than one user for the owner email",
            )),
        }
    }

    async fn ensure_user(&self, email: &str) -> CoreResult<String> {
        if let Some(id) = self.find_user(email).await? {
            return Ok(id);
        }
        match self.backend.create_workos_user(email).await {
            Ok(user) if normalize_owner_email(Some(&user.email)).as_deref() == Some(email) => {
                Ok(user.id)
            }
            Ok(_) => Err(CoreError::OwnerIdentityUnavailable(
                "WorkOS created a user for a different email".into(),
            )),
            // Another caller may have created it first: emails are unique.
            Err(error) => self.find_user(email).await?.ok_or(error),
        }
    }

    /// The CLI's admin must be a verified WorkOS user with this email and an
    /// active operator-org membership, not just text on the command line.
    pub async fn verify_operator(&self, admin: &OperatorIdentity) -> CoreResult<()> {
        let email =
            normalize_owner_email(Some(&admin.email)).ok_or(CoreError::MissingVerifiedEmail)?;
        let user = self
            .backend
            .workos_user(admin.workos_user_id.trim())
            .await?
            .ok_or(CoreError::OperatorIdentityMismatch)?;
        if user.id != admin.workos_user_id.trim()
            || !user.email_verified
            || normalize_owner_email(Some(&user.email)).as_deref() != Some(email.as_str())
            || !self.backend.operator_membership_active(&user.id).await?
        {
            return Err(CoreError::OperatorIdentityMismatch);
        }
        Ok(())
    }

    async fn chat_account(&self, workos_user_id: &str) -> CoreResult<String> {
        let id = self.backend.hosted_chat_account_id(workos_user_id).await?;
        crate::normalize_owner_chat_account_id(Some(&id))?.ok_or_else(|| {
            CoreError::OwnerIdentityUnavailable("hosted chat returned no account id".into())
        })
    }
}

/// Create an Agent for `input.owner_email` the way both entry points do:
/// resolve the owner's sign-in identity first, then run the store's single
/// transaction with it. A preview store creates nothing outside Core either.
pub async fn operator_create_agent(
    store: &CoreStore,
    provisioner: &OwnerIdentityProvisioner,
    mut input: OperatorCreateAgentInput,
) -> CoreResult<OperatorCreateAgentOutcome> {
    let preview = store.is_dry_run();
    input.owner_identity = None;
    let plan = store.operator_owner_identity_plan(&input).await?;
    let owner_email = normalize_owner_email(Some(&input.owner_email))
        .ok_or(CoreError::InvalidOperatorOwnerEmail)?;
    let mut owner_identity_preview = None;
    input.owner_identity = match plan {
        OwnerIdentityPlan::Reuse(identity) => Some(identity),
        _ if preview => {
            let existing = match &plan {
                OwnerIdentityPlan::Linked { workos_user_id } => Some(workos_user_id.clone()),
                _ => provisioner.find_user(&owner_email).await?,
            };
            owner_identity_preview = Some(OwnerIdentityPreview {
                would_create_workos_user: existing.is_none(),
                existing_workos_user_id: existing,
                would_read_or_mint_chat_identity: true,
            });
            None
        }
        OwnerIdentityPlan::Linked { workos_user_id } => Some(OwnerSignInIdentity {
            chat_account_id: provisioner.chat_account(&workos_user_id).await?,
            workos_user_id,
        }),
        OwnerIdentityPlan::Unlinked => {
            let workos_user_id = provisioner.ensure_user(&owner_email).await?;
            Some(OwnerSignInIdentity {
                chat_account_id: provisioner.chat_account(&workos_user_id).await?,
                workos_user_id,
            })
        }
    };
    let result = store.admin_create_agent_for_owner(input).await?;
    Ok(OperatorCreateAgentOutcome {
        result,
        owner_identity_preview,
    })
}

fn unavailable(what: &str, status: Option<StatusCode>) -> CoreError {
    CoreError::OwnerIdentityUnavailable(match status {
        Some(status) => format!("{what} returned HTTP {}", status.as_u16()),
        None => format!("{what} is unreachable"),
    })
}

fn http_client() -> CoreResult<Client> {
    Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|_| CoreError::OwnerIdentityUnavailable("cannot build HTTP client".into()))
}

fn parse_base_url(name: &str, value: &str) -> CoreResult<Url> {
    let url = Url::parse(value.trim().trim_end_matches('/'))
        .map_err(|_| CoreError::OwnerIdentityUnavailable(format!("{name} is not a URL")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(CoreError::OwnerIdentityUnavailable(format!(
            "{name} must use http or https"
        )));
    }
    Ok(url)
}
