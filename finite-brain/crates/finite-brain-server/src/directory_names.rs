//! Optional Identity Directory exact-key name lookup used only by the access
//! report. The Directory answers which active published names are bound to
//! an exact key; it never decides access. Deployments without the lookup
//! configured, or with an older Directory, degrade to explicit name states.

use std::collections::BTreeSet;
use std::io::Read;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

/// Loopback Directory route; must match `finite_identity::authority::NAME_LOOKUP_PATH`.
const NAME_LOOKUP_PATH: &str = "/api/v1/name-lookup/by-key";
/// Credential header; must match the Directory's `NAME_LOOKUP_TOKEN_HEADER`.
const NAME_LOOKUP_TOKEN_HEADER: &str = "x-finite-name-lookup-token";
/// Keys sent per Directory request; matches the Directory batch ceiling.
pub(crate) const DIRECTORY_BATCH_KEYS: usize = 64;
const DIRECTORY_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const DIRECTORY_READ_TIMEOUT: Duration = Duration::from_secs(3);
/// Whole-request ceiling, shorter than the caller's async timeout, so an
/// abandoned blocking lookup still finishes and releases its shared permit.
pub(crate) const DIRECTORY_TOTAL_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_DIRECTORY_RESPONSE_BYTES: u64 = 256 * 1024;
const MAX_NAMES_PER_KEY: usize = 8;
const MAX_NAME_BYTES: usize = 254;
const MAX_SOURCE_BYTES: usize = 64;

/// Accept only a literal loopback base URL: `http(s)://127.x.x.x[:port]` or
/// `http(s)://[::1][:port]`, no user info, path, query, or fragment. A host
/// name is refused so DNS can never redirect the credential and report keys.
pub(crate) fn validate_loopback_base_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or("directory name lookup URL must be http(s)")?;
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty() || authority.contains(['/', '?', '#', '@', '\\']) {
        return Err(
            "directory name lookup URL must be scheme://loopback-ip[:port] with no user info, path, query, or fragment"
                .to_owned(),
        );
    }
    let (host_is_loopback, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, after) = bracketed
            .split_once(']')
            .ok_or("directory name lookup URL has a malformed IPv6 host")?;
        let loopback = host.parse::<Ipv6Addr>().is_ok_and(|ip| ip.is_loopback());
        let port = match after {
            "" => None,
            port => Some(
                port.strip_prefix(':')
                    .ok_or("directory name lookup URL has a malformed port")?,
            ),
        };
        (loopback, port)
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        (
            host.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback()),
            port,
        )
    };
    if !host_is_loopback {
        return Err(
            "directory name lookup URL host must be a literal loopback IP (127.0.0.0/8 or [::1])"
                .to_owned(),
        );
    }
    if let Some(port) = port
        && !(port.bytes().all(|byte| byte.is_ascii_digit())
            && port.parse::<u16>().is_ok_and(|port| port > 0))
    {
        return Err("directory name lookup URL has an invalid port".to_owned());
    }
    Ok(url.trim_end_matches('/').to_owned())
}

/// Why a Directory batch could not be used.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum DirectoryLookupFailure {
    /// The Directory does not serve the route (older deployment).
    Unsupported,
    /// The Directory rejected the configured credential.
    Unauthorized,
    /// Transport, timeout, or a response that did not match the request.
    Unavailable(String),
}

impl DirectoryLookupFailure {
    pub(crate) fn state(&self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Unauthorized | Self::Unavailable(_) => "unavailable",
        }
    }

    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Unsupported => {
                "the Identity Directory does not offer exact-key name lookup".to_owned()
            }
            Self::Unauthorized => {
                "the Identity Directory rejected the name-lookup credential".to_owned()
            }
            Self::Unavailable(reason) => {
                format!("the Identity Directory was unavailable: {reason}")
            }
        }
    }
}

/// One active name the Directory reports for an exact key.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub(crate) struct DirectoryName {
    pub(crate) name: String,
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) source: Option<String>,
    #[serde(default)]
    pub(crate) bound_at: Option<u64>,
}

/// Directory answer for one requested key.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub(crate) struct DirectoryKeyNames {
    pub(crate) pubkey: String,
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) names: Vec<DirectoryName>,
    #[serde(default)]
    pub(crate) more_names: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub(crate) struct DirectoryLookupResponse {
    #[serde(default)]
    pub(crate) checked_at: Option<u64>,
    pub(crate) results: Vec<DirectoryKeyNames>,
}

/// Blocking lookup of lowercase hex keys. Called off the async runtime and
/// never while the store mutex is held.
pub(crate) type DirectoryNameLookup =
    Arc<dyn Fn(&[String]) -> Result<DirectoryLookupResponse, DirectoryLookupFailure> + Send + Sync>;

/// Build the HTTP lookup against the Directory's trusted loopback listener.
/// Refuses any base URL that is not a literal loopback address.
pub(crate) fn http_directory_name_lookup(
    base_url: &str,
    token: String,
) -> Result<DirectoryNameLookup, String> {
    let url = format!(
        "{}{NAME_LOOKUP_PATH}",
        validate_loopback_base_url(base_url)?
    );
    Ok(Arc::new(move |keys| {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(DIRECTORY_CONNECT_TIMEOUT)
            .timeout_read(DIRECTORY_READ_TIMEOUT)
            .timeout(DIRECTORY_TOTAL_TIMEOUT)
            .redirects(0)
            .build();
        let body = serde_json::json!({ "pubkeys": keys }).to_string();
        let response = match agent
            .post(&url)
            .set("content-type", "application/json")
            .set(NAME_LOOKUP_TOKEN_HEADER, &token)
            .send_string(&body)
        {
            Ok(response) => response,
            Err(ureq::Error::Status(404 | 405, _)) => {
                return Err(DirectoryLookupFailure::Unsupported);
            }
            Err(ureq::Error::Status(401 | 403, _)) => {
                return Err(DirectoryLookupFailure::Unauthorized);
            }
            Err(ureq::Error::Status(status, _)) => {
                return Err(DirectoryLookupFailure::Unavailable(format!(
                    "status {status}"
                )));
            }
            Err(ureq::Error::Transport(_)) => {
                return Err(DirectoryLookupFailure::Unavailable("transport".to_owned()));
            }
        };
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_DIRECTORY_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| DirectoryLookupFailure::Unavailable("read failed".to_owned()))?;
        if bytes.len() as u64 > MAX_DIRECTORY_RESPONSE_BYTES {
            return Err(DirectoryLookupFailure::Unavailable(
                "response too large".to_owned(),
            ));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| DirectoryLookupFailure::Unavailable("malformed response".to_owned()))
    }))
}

/// Accept an answer only when it answers every requested key exactly once,
/// by its exact canonical key, with a valid status and well-formed names.
/// Anything else makes the whole batch unusable rather than partly trusted.
pub(crate) fn validate_directory_response(
    requested: &[String],
    response: DirectoryLookupResponse,
) -> Result<DirectoryLookupResponse, DirectoryLookupFailure> {
    let mismatch = |reason: &str| Err(DirectoryLookupFailure::Unavailable(reason.to_owned()));
    let requested_set = requested.iter().collect::<BTreeSet<_>>();
    let answered = response
        .results
        .iter()
        .map(|result| &result.pubkey)
        .collect::<BTreeSet<_>>();
    if answered.len() != response.results.len()
        || answered != requested_set
        || response.results.len() != requested.len()
    {
        return mismatch("response did not answer exactly the requested keys");
    }
    for result in &response.results {
        let found = match result.status.as_str() {
            "found" => true,
            "not_found" => false,
            _ => return mismatch("response carried an unrecognized status"),
        };
        if found == result.names.is_empty()
            || result.names.len() > MAX_NAMES_PER_KEY
            || (result.more_names && result.names.len() < MAX_NAMES_PER_KEY)
        {
            return mismatch("response names did not match their status");
        }
        let mut names = BTreeSet::new();
        for name in &result.names {
            if !matches!(name.kind.as_str(), "mailbox" | "managed_agent")
                || !well_formed_name(&name.name)
                || !names.insert(name.name.as_str())
                || name.source.as_deref().is_some_and(|source| {
                    source.is_empty()
                        || source.len() > MAX_SOURCE_BYTES
                        || !source
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                })
            {
                return mismatch("response carried an unrecognized binding");
            }
        }
    }
    Ok(response)
}

/// One `local@domain` name: bounded, printable, no whitespace or controls.
fn well_formed_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name
            .chars()
            .all(|ch| !ch.is_control() && !ch.is_whitespace())
        && name.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.is_empty() && !domain.contains('@')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const KEY_B: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn answer(pubkey: &str, status: &str, names: &[(&str, &str)]) -> DirectoryKeyNames {
        DirectoryKeyNames {
            pubkey: pubkey.to_owned(),
            status: status.to_owned(),
            names: names
                .iter()
                .map(|(name, kind)| DirectoryName {
                    name: (*name).to_owned(),
                    kind: (*kind).to_owned(),
                    source: Some("finite_vip_binding".to_owned()),
                    bound_at: Some(1),
                })
                .collect(),
            more_names: false,
        }
    }

    fn check(
        results: Vec<DirectoryKeyNames>,
    ) -> Result<DirectoryLookupResponse, DirectoryLookupFailure> {
        validate_directory_response(
            &[KEY_A.to_owned(), KEY_B.to_owned()],
            DirectoryLookupResponse {
                checked_at: Some(1),
                results,
            },
        )
    }

    #[test]
    fn directory_answers_must_be_complete_exact_and_well_formed() {
        assert!(
            check(vec![
                answer(KEY_A, "found", &[("one@finite.vip", "mailbox")]),
                answer(KEY_B, "not_found", &[]),
            ])
            .is_ok()
        );
        let long_name = format!("{}@finite.vip", "a".repeat(300));
        let bad = [
            // Partial: one requested key unanswered.
            vec![answer(KEY_A, "found", &[("one@finite.vip", "mailbox")])],
            // Foreign key substituted for a requested one.
            vec![
                answer(KEY_A, "not_found", &[]),
                answer(&"3".repeat(64), "not_found", &[]),
            ],
            // Duplicate answer.
            vec![
                answer(KEY_A, "not_found", &[]),
                answer(KEY_A, "not_found", &[]),
                answer(KEY_B, "not_found", &[]),
            ],
            // Non-canonical (uppercase) key.
            vec![
                answer(&"A".repeat(64), "not_found", &[]),
                answer(KEY_B, "not_found", &[]),
            ],
            // Unknown status, and status/names disagreement.
            vec![answer(KEY_A, "maybe", &[]), answer(KEY_B, "not_found", &[])],
            vec![answer(KEY_A, "found", &[]), answer(KEY_B, "not_found", &[])],
            vec![
                answer(KEY_A, "not_found", &[("one@finite.vip", "mailbox")]),
                answer(KEY_B, "not_found", &[]),
            ],
            // Unknown kind, control characters, overlong, not a name.
            vec![
                answer(KEY_A, "found", &[("one@finite.vip", "owner")]),
                answer(KEY_B, "not_found", &[]),
            ],
            vec![
                answer(KEY_A, "found", &[("one\u{1b}[2J@finite.vip", "mailbox")]),
                answer(KEY_B, "not_found", &[]),
            ],
            vec![
                answer(KEY_A, "found", &[(long_name.as_str(), "mailbox")]),
                answer(KEY_B, "not_found", &[]),
            ],
            vec![
                answer(KEY_A, "found", &[("no-at-sign", "mailbox")]),
                answer(KEY_B, "not_found", &[]),
            ],
        ];
        for results in bad {
            let shown = format!("{results:?}");
            assert!(check(results).is_err(), "{shown}");
        }
    }

    /// A Directory that drip-feeds its body under the per-read timeout must
    /// still be cut off by the total timeout, so the blocking worker (and the
    /// shared permit it holds) is released before the caller gives up.
    #[test]
    fn a_drip_fed_directory_response_is_cut_off_by_the_total_timeout() {
        use std::io::Write as _;
        use std::net::TcpListener;
        use std::time::Instant;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = std::io::Read::read(&mut stream, &mut request);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100000\r\n\r\n",
            );
            for _ in 0..40 {
                if stream.write_all(b" ").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        });
        let lookup =
            http_directory_name_lookup(&format!("http://{address}"), "synthetic".to_owned())
                .unwrap();
        let started = Instant::now();
        let result = lookup(&[KEY_A.to_owned()]);
        let elapsed = started.elapsed();
        assert!(matches!(
            result,
            Err(DirectoryLookupFailure::Unavailable(_))
        ));
        assert!(
            elapsed >= DIRECTORY_TOTAL_TIMEOUT - Duration::from_millis(500),
            "{elapsed:?}"
        );
        assert!(
            elapsed < DIRECTORY_TOTAL_TIMEOUT + Duration::from_millis(900),
            "{elapsed:?}"
        );
        server.join().unwrap();
    }

    #[test]
    fn directory_url_must_be_a_literal_loopback_address() {
        for good in [
            "http://127.0.0.1:8790",
            "http://127.0.0.1:8790/",
            "https://127.0.0.2",
            "http://[::1]:8790",
        ] {
            assert!(validate_loopback_base_url(good).is_ok(), "{good}");
        }
        for bad in [
            "http://localhost:8790",
            "http://identity.internal:8790",
            "http://10.0.0.5:8790",
            "http://0.0.0.0:8790",
            "http://[::]:8790",
            "http://user:pass@127.0.0.1:8790",
            "http://127.0.0.1:8790/prefix",
            "http://127.0.0.1:8790?x=1",
            "http://127.0.0.1:8790#frag",
            "http://127.0.0.1:0",
            "http://127.0.0.1:+80",
            "http://127.0.0.1.nip.io:8790",
            "ftp://127.0.0.1",
            "http://",
        ] {
            assert!(validate_loopback_base_url(bad).is_err(), "{bad}");
        }
        assert!(
            http_directory_name_lookup("http://identity.example:8790", "t".to_owned()).is_err()
        );
    }
}
