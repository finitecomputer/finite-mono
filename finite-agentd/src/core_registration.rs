//! Outbound registration of a networking process's ephemeral Iroh endpoint.
//! The endpoint owner constructs this once and retains it for retries. A new
//! process allocates a new durable generation without persisting its Iroh key.
use crate::{AgentdError, Ledger};
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointRegistrationOutcome {
    Accepted,
    /// Wait for launch binding/lease recovery, or stop if access was revoked.
    Unauthorized,
    /// A newer process has registered. This process must stop advertising.
    Superseded,
    Retry,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegistrationBody {
    generation: i64,
    endpoint_id: String,
    relay_url: String,
}

// Deliberately no Debug/Serialize: contains a runtime credential.
pub struct CoreEndpointRegistration {
    client: reqwest::Client,
    url: String,
    credential: String,
    body: RegistrationBody,
}
impl CoreEndpointRegistration {
    pub fn new(
        core_url: &str,
        credential: String,
        ledger: &Ledger,
        endpoint_id: String,
        relay_url: String,
    ) -> Result<Self, AgentdError> {
        let core = reqwest::Url::parse(core_url)
            .map_err(|_| AgentdError::Config("invalid Core origin".into()))?;
        let local = matches!(core.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if (core.scheme() != "https" && !(core.scheme() == "http" && local))
            || core.host_str().is_none()
            || !core.username().is_empty()
            || core.password().is_some()
            || core.query().is_some()
            || core.fragment().is_some()
            || core.path() != "/"
        {
            return Err(AgentdError::Config(
                "Core must be an HTTPS origin (HTTP loopback allowed locally)".into(),
            ));
        }
        let valid_hex = |s: &str| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        let relay = reqwest::Url::parse(&relay_url)
            .map_err(|_| AgentdError::Config("invalid Iroh relay URL".into()))?;
        if !valid_hex(&credential)
            || !valid_hex(&endpoint_id)
            || relay_url.len() > 2048
            || relay.scheme() != "https"
            || relay.host_str().is_none()
            || !relay.username().is_empty()
            || relay.password().is_some()
            || relay.query().is_some()
            || relay.fragment().is_some()
        {
            return Err(AgentdError::Config(
                "invalid runtime endpoint registration".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let generation = ledger.next_endpoint_generation(&credential)?;
        Ok(Self {
            client,
            url: format!(
                "{}/api/core/v1/runtime/iroh-endpoint",
                core.as_str().trim_end_matches('/')
            ),
            credential,
            body: RegistrationBody {
                generation,
                endpoint_id,
                relay_url,
            },
        })
    }

    pub async fn register(&self) -> Result<EndpointRegistrationOutcome, AgentdError> {
        let response = self
            .client
            .post(&self.url)
            .bearer_auth(&self.credential)
            .json(&self.body)
            .send()
            .await?;
        Ok(match response.status().as_u16() {
            204 => EndpointRegistrationOutcome::Accepted,
            401 => EndpointRegistrationOutcome::Unauthorized,
            409 => EndpointRegistrationOutcome::Superseded,
            429 | 500..=599 => EndpointRegistrationOutcome::Retry,
            _ => {
                return Err(AgentdError::Transport(
                    "Core rejected endpoint registration".into(),
                ));
            }
        })
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdmittedPeer {
    pub peer_id: String,
    pub expires_in_seconds: u64,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeAdmissions {
    pub generation: i64,
    pub endpoint_id: String,
    pub service: String,
    pub peers: Vec<AdmittedPeer>,
}
pub(crate) enum AdmissionRefresh {
    Snapshot(RuntimeAdmissions),
    Unauthorized,
    Superseded,
}
impl CoreEndpointRegistration {
    pub(crate) async fn admissions(&self) -> Result<AdmissionRefresh, AgentdError> {
        let response = self
            .client
            .get(self.url.replace("/iroh-endpoint", "/iroh-admissions"))
            .bearer_auth(&self.credential)
            .query(&[
                ("generation", self.body.generation.to_string()),
                ("endpointId", self.body.endpoint_id.clone()),
            ])
            .send()
            .await?;
        match response.status().as_u16() {
            401 => return Ok(AdmissionRefresh::Unauthorized),
            409 => return Ok(AdmissionRefresh::Superseded),
            _ => {}
        }
        let mut response = response.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 32 * 1024 {
                return Err(AgentdError::Transport(
                    "Core admission response exceeded limit".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let snapshot: RuntimeAdmissions = serde_json::from_slice(&bytes)?;
        if snapshot.generation != self.body.generation
            || snapshot.endpoint_id != self.body.endpoint_id
            || snapshot.service != "hermes"
            || snapshot.peers.len() > 64
        {
            return Err(AgentdError::Transport(
                "Core admission binding mismatch".into(),
            ));
        }
        Ok(AdmissionRefresh::Snapshot(snapshot))
    }
}
