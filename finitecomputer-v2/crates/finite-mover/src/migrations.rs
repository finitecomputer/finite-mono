//! The mover's calls to finite.chat's `Migrations`, each signed with NIP-98
//! by the mover's own key (fragment `docs/migrations.md`; the deployment's
//! config holds only its npub):
//!
//! ```text
//! PUT  /api/migrations/mover/roster                 {roster}  → {bound, withdrawn, replay}
//! GET  /api/migrations/mover/mailbox                          → {commands}
//! POST /api/migrations/mover/agents/{old}/report    {report}  → {state, attempt, taken}
//! PUT  /api/migrations/mover/stages/{stage}/parts/{n}?sha256=<hex>  body: the part
//! GET  /api/migrations/mover/stages/{stage}/parts                   → {parts: [{n, bytes, sha256, etag}]}
//! POST /api/migrations/mover/stages/{stage}/complete  {manifest}    → {manifest_sha256, zip_bytes}
//! ```
//!
//! The key is held in memory only and never printed; errors keep the
//! status, never a body.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use finite_nostr::{HttpAuthEventRequest, sign_http_auth_header_with_secret};
use reqwest::blocking::{Client, Response};
use reqwest::{StatusCode, Url};
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

use crate::contract::{Mailbox, Report, Reported, Roster, RosterTaken};
use crate::sink::{Complete, Completed, Part, Parts, SinkError, StageSink};
use crate::stage::Manifest;

pub struct MigrationsClient {
    /// finite.chat's origin, such as `https://finite.chat`.
    base: Url,
    key: Zeroizing<[u8; 32]>,
    http: Client,
}

impl MigrationsClient {
    pub fn new(base: Url, key: Zeroizing<[u8; 32]>) -> Result<Self, SinkError> {
        // No redirect: a stage's bytes go to finite.chat's own routes alone.
        let http = Client::builder()
            .timeout(Duration::from_secs(600))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| unreachable_error("setup", &error))?;
        Ok(Self { base, key, http })
    }

    fn url(&self, stage: &str, rest: &str) -> String {
        self.mover_url(&format!("stages/{stage}/{rest}"))
    }

    fn mover_url(&self, rest: &str) -> String {
        format!(
            "{}/api/migrations/mover/{rest}",
            self.base.as_str().trim_end_matches('/')
        )
    }

    fn json_body(
        operation: &'static str,
        value: &impl serde::Serialize,
    ) -> Result<Vec<u8>, SinkError> {
        serde_json::to_vec(value).map_err(|_| SinkError::Unreadable { operation })
    }

    /// `PUT …/roster`: the whole roster, replacing the last.
    pub fn push_roster(&self, roster: &Roster) -> Result<RosterTaken, SinkError> {
        let body = Self::json_body("roster", roster)?;
        let response = self.send(
            "roster",
            reqwest::Method::PUT,
            self.mover_url("roster"),
            Some(body),
            "application/json",
        )?;
        Self::answer("roster", response)
    }

    /// `GET …/mailbox`: what the mover is told to do.
    pub fn mailbox(&self) -> Result<Mailbox, SinkError> {
        let response = self.send(
            "mailbox",
            reqwest::Method::GET,
            self.mover_url("mailbox"),
            None,
            "",
        )?;
        Self::answer("mailbox", response)
    }

    /// `POST …/agents/{old}/report`. A 409 (`NotStaging` here) means the
    /// command is no longer this old agent's current one.
    pub fn report(&self, old: &str, report: &Report) -> Result<Reported, SinkError> {
        let body = Self::json_body("report", report)?;
        let response = self.send(
            "report",
            reqwest::Method::POST,
            self.mover_url(&format!("agents/{old}/report")),
            Some(body),
            "application/json",
        )?;
        Self::answer("report", response)
    }

    fn send(
        &self,
        operation: &'static str,
        method: reqwest::Method,
        url: String,
        body: Option<Vec<u8>>,
        content_type: &str,
    ) -> Result<Response, SinkError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let mut signed = HttpAuthEventRequest::new(method.as_str(), url.clone(), now);
        if let Some(body) = &body {
            signed = signed.with_body(body.clone());
        }
        let authorization =
            sign_http_auth_header_with_secret(&self.key, &signed).map_err(|_| {
                SinkError::Unreadable {
                    operation: "signing",
                }
            })?;
        let mut request = self
            .http
            .request(method, url)
            .header("authorization", authorization);
        if let Some(body) = body {
            request = request.header("content-type", content_type).body(body);
        }
        request
            .send()
            .map_err(|error| unreachable_error(operation, &error))
    }

    fn answer<T: DeserializeOwned>(
        operation: &'static str,
        response: Response,
    ) -> Result<T, SinkError> {
        match response.status() {
            StatusCode::FORBIDDEN => return Err(SinkError::NotCommanded),
            StatusCode::CONFLICT => return Err(SinkError::NotStaging),
            status if !status.is_success() => {
                return Err(SinkError::Refused {
                    operation,
                    status: status.as_u16(),
                });
            }
            _ => {}
        }
        response
            .json()
            .map_err(|_| SinkError::Unreadable { operation })
    }
}

impl StageSink for MigrationsClient {
    fn parts(&self, stage: &str) -> Result<Vec<Part>, SinkError> {
        let response = self.send(
            "parts",
            reqwest::Method::GET,
            self.url(stage, "parts"),
            None,
            "",
        )?;
        Self::answer::<Parts>("parts", response).map(|parts| parts.parts)
    }

    fn put_part(&self, stage: &str, n: u32, body: &[u8], sha256: &str) -> Result<Part, SinkError> {
        let url = self.url(stage, &format!("parts/{n}?sha256={sha256}"));
        let response = self.send(
            "put part",
            reqwest::Method::PUT,
            url,
            Some(body.to_vec()),
            "application/octet-stream",
        )?;
        Self::answer("put part", response)
    }

    fn complete(&self, stage: &str, manifest: &Manifest) -> Result<Completed, SinkError> {
        let body = serde_json::to_vec(&Complete {
            manifest: manifest.clone(),
        })
        .map_err(|_| SinkError::Unreadable {
            operation: "complete",
        })?;
        let response = self.send(
            "complete",
            reqwest::Method::POST,
            self.url(stage, "complete"),
            Some(body),
            "application/json",
        )?;
        Self::answer("complete", response)
    }
}

fn unreachable_error(operation: &'static str, error: &reqwest::Error) -> SinkError {
    // reqwest's message names the URL, never a header or body.
    SinkError::Unreachable {
        operation,
        reason: error.to_string(),
    }
}
