//! Loopback-only exact-key name lookup for trusted product reports.
//!
//! A product that already holds an authorized list of keys (the Brain access
//! report) may ask which active, published Finite VIP names are bound to each
//! exact key. It is evidence metadata, never an authorization answer. The
//! route lives only on the loopback router, requires its own read-only
//! credential (never the operator token), accepts a bounded batch, and
//! returns nothing for disabled bindings, unknown keys, or other identities.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{AuthorityState, StoreError, api_error, store_error_code};
use crate::{hex, npub};

/// Loopback route path. Never mounted on the public router.
pub const NAME_LOOKUP_PATH: &str = "/api/v1/name-lookup/by-key";
/// Header carrying the read-only name-lookup credential.
pub const NAME_LOOKUP_TOKEN_HEADER: &str = "x-finite-name-lookup-token";
/// Maximum keys accepted in one batch.
pub const MAX_NAME_LOOKUP_KEYS: usize = 64;
/// Maximum active names returned for one key; `more_names` flags the rest.
pub const MAX_NAMES_PER_KEY: usize = 8;
/// Request body ceiling: a full batch of npubs fits well inside it.
const MAX_NAME_LOOKUP_BODY_BYTES: usize = 16 * 1024;

#[derive(Debug, Deserialize)]
pub(super) struct NameLookupRequest {
    pubkeys: Vec<String>,
}

#[derive(Debug, Serialize)]
struct NameLookupResponse {
    checked_at: u64,
    results: Vec<KeyNames>,
}

#[derive(Debug, Serialize)]
struct KeyNames {
    pubkey: String,
    npub: String,
    status: &'static str,
    names: Vec<BoundName>,
    more_names: bool,
}

#[derive(Debug, Serialize)]
struct BoundName {
    name: String,
    kind: &'static str,
    source: &'static str,
    bound_at: u64,
}

pub(super) async fn lookup_names_by_key(
    State(state): State<AuthorityState>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    // Authenticate before parsing anything the caller sent.
    if let Err((status, code)) = require_name_lookup_credential(&state, &headers) {
        return api_error(status, code);
    }
    if body.len() > MAX_NAME_LOOKUP_BODY_BYTES {
        return api_error(StatusCode::PAYLOAD_TOO_LARGE, "request_too_large");
    }
    let Ok(request) = serde_json::from_slice::<NameLookupRequest>(&body) else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    if request.pubkeys.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "pubkeys_required");
    }
    if request.pubkeys.len() > MAX_NAME_LOOKUP_KEYS {
        return api_error(StatusCode::PAYLOAD_TOO_LARGE, "too_many_pubkeys");
    }
    let mut keys = Vec::with_capacity(request.pubkeys.len());
    for raw in &request.pubkeys {
        let Some(key) = canonical_hex_key(raw) else {
            return api_error(StatusCode::BAD_REQUEST, "invalid_pubkey");
        };
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let domain = state.config.finite_vip_domain.to_ascii_lowercase();
    let mut results = Vec::with_capacity(keys.len());
    for key in keys {
        let (names, more_names) = match active_names_for_key(&state, &key, &domain) {
            Ok(found) => found,
            Err(error) => {
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
            }
        };
        let Some(bytes) = hex::decode32(&key) else {
            return api_error(StatusCode::BAD_REQUEST, "invalid_pubkey");
        };
        results.push(KeyNames {
            npub: npub::encode(&bytes),
            pubkey: key,
            status: if names.is_empty() {
                "not_found"
            } else {
                "found"
            },
            names,
            more_names,
        });
    }
    Json(NameLookupResponse {
        checked_at: state.clock.now(),
        results,
    })
    .into_response()
}

fn canonical_hex_key(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if let Ok(bytes) = npub::decode(raw) {
        return Some(hex::encode(&bytes));
    }
    hex::is_hex32(raw).then(|| raw.to_ascii_lowercase())
}

fn active_names_for_key(
    state: &AuthorityState,
    key: &str,
    domain: &str,
) -> Result<(Vec<BoundName>, bool), StoreError> {
    let conn = state.store.conn.lock().expect("store mutex never poisoned");
    let mut statement = conn.prepare(
        "SELECT b.email, b.created_at,
                EXISTS (
                    SELECT 1 FROM managed_agent_nip05_bindings m
                    WHERE m.name = b.email AND m.pubkey = b.pubkey
                )
         FROM vip_email_bindings b
         WHERE b.pubkey = ?1 AND b.domain = ?2 AND b.disabled_at IS NULL
         ORDER BY b.email
         LIMIT ?3",
    )?;
    let mut names = statement
        .query_map(
            params![key, domain, (MAX_NAMES_PER_KEY + 1) as i64],
            |row| {
                let managed_agent: bool = row.get(2)?;
                Ok(BoundName {
                    name: row.get(0)?,
                    kind: if managed_agent {
                        "managed_agent"
                    } else {
                        "mailbox"
                    },
                    source: "finite_vip_binding",
                    bound_at: row.get(1)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let more_names = names.len() > MAX_NAMES_PER_KEY;
    names.truncate(MAX_NAMES_PER_KEY);
    Ok((names, more_names))
}

fn require_name_lookup_credential(
    state: &AuthorityState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, &'static str)> {
    let Some(expected) = state.name_lookup_token.as_deref() else {
        return Err((StatusCode::UNAUTHORIZED, "name_lookup_disabled"));
    };
    let Some(actual) = headers.get(NAME_LOOKUP_TOKEN_HEADER) else {
        return Err((StatusCode::UNAUTHORIZED, "missing_name_lookup_token"));
    };
    let Ok(actual) = actual.to_str() else {
        return Err((StatusCode::UNAUTHORIZED, "malformed_name_lookup_token"));
    };
    // Compare digests so the check does not short-circuit on a prefix.
    if Sha256::digest(actual.as_bytes()) != Sha256::digest(expected.as_bytes()) {
        return Err((StatusCode::UNAUTHORIZED, "invalid_name_lookup_token"));
    }
    Ok(())
}
