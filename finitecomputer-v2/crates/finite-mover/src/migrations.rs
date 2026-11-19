//! The mover's calls to finite.chat's `Migrations`, each signed with NIP-98
//! by the mover's own key (fragment `docs/migrations.md`; the deployment's
//! config holds only its npub):
//!
//! ```text
//! PUT  /api/migrations/mover/roster                 {roster}  → {bound, withdrawn, replay}
//! GET  /api/migrations/mover/mailbox                          → {commands}
//! POST /api/migrations/mover/agents/{old}/report    {report}  → {state, attempt, taken}
//! POST /api/migrations/mover/agents/{old}/secrets   {command, items}  → {handed}
//! PUT  /api/migrations/mover/stages/{stage}/parts/{n}?sha256=<hex>  body: the part
//! GET  /api/migrations/mover/stages/{stage}/parts                   → {parts: [{n, bytes, sha256, etag}]}
//! POST /api/migrations/mover/stages/{stage}/complete  {manifest}    → {manifest_sha256, zip_bytes}
//! ```
//!
//! The key is held in memory only and never printed; errors keep the
//! status, never a body. A part that fails in a way that passes in
//! moments is sent again, signed afresh (`PART_WAITS`).

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use finite_nostr::{HttpAuthEventRequest, sign_http_auth_header_with_secret};
use reqwest::blocking::{Client, Response};
use reqwest::{StatusCode, Url};
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

use crate::contract::{
    HandedOff, Handoff, HandoffItem, Mailbox, Report, Reported, Roster, RosterTaken,
};
use crate::sink::{Complete, Completed, Part, Parts, SinkError, StageSink};
use crate::stage::Manifest;

/// The waits between a part's tries: a failed one is sent again, signed
/// afresh, after each in turn (finite.chat takes the same part again in
/// place). Its tries are `PART_TRIES`. Each of these passes in moments, and
/// each once ended the run, whose resume reads the old agent's whole
/// stream again to skip what was sent: R2's own error (500), a signature
/// more than finite.chat's 60 s old when it arrived (401), a dropped
/// connection.
pub const PART_WAITS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(4),
    Duration::from_secs(16),
];

/// A part's tries at most: the first, then one after each `PART_WAITS`.
pub const PART_TRIES: usize = PART_WAITS.len() + 1;

/// No try of a part starts this long after its first. Each may take the
/// client's whole 600 s timeout, and a stage that says nothing for 30
/// minutes is held by finite.chat (`PROGRESS_EVERY` reports only between
/// parts): a part that slow ends the run as before.
pub const PART_TRY_WINDOW: Duration = Duration::from_secs(5 * 60);

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

    /// `POST …/agents/{old}/secrets`: the old agent's secrets for
    /// `command`, each into its person's Vault on finite.chat; a refusal
    /// keeps its status alone. The body is held in memory alone, never on
    /// disk or in a log; this copy of it is cleared once sent, but the
    /// copies the signer and reqwest make are ordinary memory, freed
    /// uncleared.
    pub fn hand_off(
        &self,
        old: &str,
        command: &str,
        items: &[HandoffItem],
    ) -> Result<HandedOff, SinkError> {
        let body = Zeroizing::new(Self::json_body("secrets", &Handoff { command, items })?);
        let response = self.send(
            "secrets",
            reqwest::Method::POST,
            self.mover_url(&format!("agents/{old}/secrets")),
            Some(body.to_vec()),
            "application/json",
        )?;
        Self::answer("secrets", response)
    }

    fn send(
        &self,
        operation: &'static str,
        method: reqwest::Method,
        url: String,
        body: Option<Vec<u8>>,
        content_type: &str,
    ) -> Result<Response, SinkError> {
        // The signer's copy of the body is made before the clock is read,
        // so the signature's 60 s start as near its send as this signer
        // allows (it hashes the body itself).
        let signed_body = body.clone();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let mut signed = HttpAuthEventRequest::new(method.as_str(), url.clone(), now);
        if let Some(body) = signed_body {
            signed = signed.with_body(body);
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

    /// Each try is signed by `send` as it goes: a retry never carries an
    /// earlier try's signature, which may be what aged past 60 s.
    fn put_part(&self, stage: &str, n: u32, body: &[u8], sha256: &str) -> Result<Part, SinkError> {
        let url = self.url(stage, &format!("parts/{n}?sha256={sha256}"));
        let first = Instant::now();
        let mut tried = 0;
        loop {
            tried += 1;
            let error = match self
                .send(
                    "put part",
                    reqwest::Method::PUT,
                    url.clone(),
                    Some(body.to_vec()),
                    "application/octet-stream",
                )
                .and_then(|response| Self::answer("put part", response))
            {
                Ok(part) => return Ok(part),
                Err(error) => error,
            };
            let Some(wait) = part_wait(&error, tried, first.elapsed()) else {
                return Err(error);
            };
            eprintln!("finite-mover: part {n} of {stage}: {error}; trying again in {wait:?}");
            std::thread::sleep(wait);
        }
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

/// The wait before a part's next try, after `tried` tries the first of
/// which began `since_first` ago, or none: `error` would only come again
/// (a refusal finite.chat means, such as 403, 409 or 400), the tries are
/// spent, or the next would begin past `PART_TRY_WINDOW`.
fn part_wait(error: &SinkError, tried: usize, since_first: Duration) -> Option<Duration> {
    let passes = matches!(
        error,
        SinkError::Unreachable { .. }
            | SinkError::Refused {
                status: 401 | 500..=599,
                ..
            }
    );
    let wait = *PART_WAITS.get(tried.checked_sub(1)?)?;
    (passes && since_first + wait < PART_TRY_WINDOW).then_some(wait)
}

fn unreachable_error(operation: &'static str, error: &reqwest::Error) -> SinkError {
    // reqwest's message names the URL, never a header or body.
    SinkError::Unreachable {
        operation,
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(status: u16) -> SinkError {
        SinkError::Refused {
            operation: "put part",
            status,
        }
    }

    #[test]
    fn a_part_is_tried_again_only_for_what_passes() {
        let cut = SinkError::Unreachable {
            operation: "put part",
            reason: "connection reset".into(),
        };
        for error in [refused(500), refused(502), refused(503), refused(401), cut] {
            assert_eq!(
                part_wait(&error, 1, Duration::ZERO),
                Some(PART_WAITS[0]),
                "{error}"
            );
        }
        for error in [
            refused(400),
            refused(404),
            refused(413),
            refused(429),
            SinkError::NotCommanded,
            SinkError::NotStaging,
            SinkError::Unreadable {
                operation: "put part",
            },
        ] {
            assert_eq!(part_wait(&error, 1, Duration::ZERO), None, "{error}");
        }
    }

    #[test]
    fn a_parts_tries_are_bounded_in_number_and_time() {
        let waits: Vec<_> = (1..=PART_TRIES)
            .map(|tried| part_wait(&refused(500), tried, Duration::ZERO))
            .collect();
        assert_eq!(
            waits,
            [
                Some(PART_WAITS[0]),
                Some(PART_WAITS[1]),
                Some(PART_WAITS[2]),
                None
            ]
        );
        assert_eq!(part_wait(&refused(500), 0, Duration::ZERO), None);
        // The next try would begin at the window's end: none.
        let late = PART_TRY_WINDOW - PART_WAITS[0];
        assert_eq!(part_wait(&refused(500), 1, late), None);
        assert_eq!(
            part_wait(&refused(500), 1, late - Duration::from_millis(1)),
            Some(PART_WAITS[0])
        );
    }
}
