//! finite-identity Authority HTTP helpers used by fbrain auth email flows.

use serde::{Deserialize, Serialize};

use crate::{CliEnvironment, CliError, load_or_generate_identity, unix_timestamp};

#[derive(Debug, Clone)]
pub(crate) struct IdentityAuthorityClient {
    base_url: String,
    client: finite_identity::client::IdentityClient,
}

impl IdentityAuthorityClient {
    pub(crate) fn from_environment(env: &CliEnvironment) -> Result<Self, CliError> {
        let base_url = env
            .identity_authority_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                CliError::Unsupported(
                    "email auth requires FINITE_IDENTITY_AUTHORITY to point at finite-identity"
                        .to_owned(),
                )
            })?
            .trim_end_matches('/')
            .to_owned();
        Ok(Self {
            client: finite_identity::client::IdentityClient::new(&base_url),
            base_url,
        })
    }

    pub(crate) fn request_email_challenge(
        &self,
        email: &str,
    ) -> Result<EmailChallengeReport, CliError> {
        let body = self
            .client
            .email_challenge_body(email)
            .map_err(|error| CliError::Identity(error.to_string()))?;
        let body = serde_json::to_vec(&body)?;
        let url = format!("{}/api/v1/email-challenges", self.base_url);
        let response = ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_bytes(&body);
        identity_response_json(response, "POST", "/api/v1/email-challenges")
    }

    pub(crate) fn redeem_email(
        &self,
        env: &CliEnvironment,
        email: &str,
        token: &str,
    ) -> Result<EmailRedeemReport, CliError> {
        let identity = load_or_generate_identity(env)?;
        let key = finite_identity::client::LocalIdentityKey::from_identity(&identity);
        if is_finite_vip_email(email) {
            let request = self
                .client
                .vip_email_binding_redeem(&key, email, token, unix_timestamp())
                .map_err(|error| CliError::Identity(error.to_string()))?;
            let response: VipEmailRedeemResponse = self.send_signed_json(request)?;
            Ok(EmailRedeemReport {
                email: response.email,
                pubkey: response.pubkey,
                principal_kind: "native".to_owned(),
                nip05: Some(response.nip05),
                limitation: None,
            })
        } else {
            let request = self
                .client
                .email_only_redeem(&key, email, token, unix_timestamp())
                .map_err(|error| CliError::Identity(error.to_string()))?;
            let response: EmailOnlyRedeemResponse = self.send_signed_json(request)?;
            Ok(EmailRedeemReport {
                email: response.email,
                pubkey: response.pubkey,
                principal_kind: "email_only".to_owned(),
                nip05: None,
                limitation: Some(
                    "FiniteBrain direct permission grants still require an npub recipient; email-targeted Vault Invitations can be claimed after email proof."
                        .to_owned(),
                ),
            })
        }
    }

    fn send_signed_json<T: serde::de::DeserializeOwned>(
        &self,
        signed: finite_identity::client::SignedJsonRequest,
    ) -> Result<T, CliError> {
        let url = format!("{}{}", self.base_url, signed.path);
        let response = ureq::request(&signed.method, &url)
            .set("Content-Type", "application/json")
            .set("Authorization", &signed.authorization)
            .send_bytes(&signed.body);
        identity_response_json(response, &signed.method, &signed.path)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmailChallengeReport {
    pub(crate) email: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmailRedeemReport {
    pub(crate) email: String,
    pub(crate) pubkey: String,
    pub(crate) principal_kind: String,
    pub(crate) nip05: Option<String>,
    pub(crate) limitation: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VipEmailRedeemResponse {
    email: String,
    pubkey: String,
    nip05: String,
}

#[derive(Debug, Deserialize)]
struct EmailOnlyRedeemResponse {
    email: String,
    pubkey: String,
}

fn identity_response_json<T: serde::de::DeserializeOwned>(
    response: Result<ureq::Response, ureq::Error>,
    method: &str,
    path: &str,
) -> Result<T, CliError> {
    match response {
        Ok(response) => {
            let body = response.into_string().map_err(|error| {
                CliError::Http(format!(
                    "could not read Identity Authority response for {method} {path}: {error}"
                ))
            })?;
            serde_json::from_str::<T>(&body).map_err(|error| {
                CliError::Http(format!(
                    "invalid Identity Authority response for {method} {path}: {error}"
                ))
            })
        }
        Err(ureq::Error::Status(status, response)) => {
            let code = response
                .into_string()
                .ok()
                .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
                .and_then(|body: serde_json::Value| {
                    body.get("error")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .unwrap_or_else(|| "no error details".to_owned());
            let classified =
                finite_identity::client::IdentityClient::classify_authority_error(status, &code)
                    .map(|error| format!(" ({:?})", error.kind()))
                    .unwrap_or_default();
            Err(CliError::Http(format!(
                "Identity Authority {method} {path} returned {status}: {code}{classified}"
            )))
        }
        Err(error) => Err(CliError::Http(format!(
            "Identity Authority {method} {path} failed: {error}"
        ))),
    }
}

fn is_finite_vip_email(email: &str) -> bool {
    email.trim().to_ascii_lowercase().ends_with("@finite.vip")
}
