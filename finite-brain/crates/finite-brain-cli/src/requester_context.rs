//! Turn-scoped authenticated requester attribution for Brain creation.
//!
//! Finite Chat writes a short-lived local lease immediately around a terminal
//! tool call. `fbrain` uses the matching lease as a transport hint for the
//! requester carried in the signed create request. The lease is not an
//! authorization boundary: the Brain server independently classifies the
//! signer and resolves a Managed Agent's account owner through operator
//! authorities before accepting an Organization Brain requester.

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use finite_nostr::NostrPublicKey;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::CliEnvironment;

const REQUESTER_CONTEXT_DIR: &str = "requester-context-v1";
const REQUESTER_CONTEXT_VERSION: u32 = 1;
const MAX_CONTEXT_BYTES: u64 = 4096;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BrainCreationAuthority {
    DirectHuman,
    AuthenticatedRequester(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SessionEnvironment {
    platform: Option<String>,
    session_key: Option<String>,
    user_id: Option<String>,
}

impl SessionEnvironment {
    fn current() -> Self {
        Self {
            platform: std::env::var("HERMES_SESSION_PLATFORM").ok(),
            session_key: std::env::var("HERMES_SESSION_KEY").ok(),
            user_id: std::env::var("HERMES_SESSION_USER_ID").ok(),
        }
    }

    fn is_finite_runtime(&self) -> bool {
        matches!(
            self.platform.as_deref().map(str::trim),
            Some("finitechat" | "local")
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequesterContext {
    version: u32,
    session_key: String,
    platform: String,
    requesting_user_id: String,
    expires_at_unix: u64,
}

pub(crate) fn resolve(env: &CliEnvironment) -> Result<BrainCreationAuthority, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_owned())?
        .as_secs();
    resolve_at(&SessionEnvironment::current(), &finite_root(env), now)
}

fn resolve_at(
    environment: &SessionEnvironment,
    finite_root: &Path,
    now: u64,
) -> Result<BrainCreationAuthority, String> {
    if !environment.is_finite_runtime() {
        return Ok(BrainCreationAuthority::DirectHuman);
    }
    let session_key = environment
        .session_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(missing_context)?;
    let user_id = environment
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| NostrPublicKey::parse(value).is_ok())
        .ok_or_else(missing_context)?;
    let path = requester_context_path(finite_root, session_key);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| missing_context())?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_CONTEXT_BYTES {
        return Err(missing_context());
    }
    let mut bytes = String::new();
    File::open(&path)
        .and_then(|file| file.take(MAX_CONTEXT_BYTES + 1).read_to_string(&mut bytes))
        .map_err(|_| missing_context())?;
    if bytes.len() as u64 > MAX_CONTEXT_BYTES {
        return Err(missing_context());
    }
    let context: RequesterContext = serde_json::from_str(&bytes).map_err(|_| missing_context())?;
    if context.version != REQUESTER_CONTEXT_VERSION
        || context.platform != "finitechat"
        || context.session_key != session_key
        || context.requesting_user_id != user_id
        || context.expires_at_unix <= now
    {
        if context.expires_at_unix <= now {
            let _ = std::fs::remove_file(path);
        }
        return Err(missing_context());
    }
    let requester = NostrPublicKey::parse(user_id)
        .and_then(|key| key.to_npub())
        .map_err(|_| missing_context())?;
    Ok(BrainCreationAuthority::AuthenticatedRequester(requester))
}

fn missing_context() -> String {
    "authenticated Finite Chat requester context is unavailable; retry from the authenticated chat turn"
        .to_owned()
}

fn finite_root(env: &CliEnvironment) -> PathBuf {
    env.finite_home
        .clone()
        .or_else(|| std::env::var_os("FINITE_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".finite")))
        .unwrap_or_else(|| env.cwd.join(".finite"))
}

fn requester_context_path(finite_root: &Path, session_key: &str) -> PathBuf {
    let digest = Sha256::digest(session_key.as_bytes());
    finite_root
        .join(REQUESTER_CONTEXT_DIR)
        .join(format!("{digest:x}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn environment(platform: &str, session_key: &str, user_id: &str) -> SessionEnvironment {
        SessionEnvironment {
            platform: Some(platform.to_owned()),
            session_key: Some(session_key.to_owned()),
            user_id: Some(user_id.to_owned()),
        }
    }

    fn write_context(root: &Path, session_key: &str, user_id: &str, expires_at: u64) {
        let path = requester_context_path(root, session_key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            serde_json::json!({
                "version": 1,
                "session_key": session_key,
                "platform": "finitechat",
                "requesting_user_id": user_id,
                "expires_at_unix": expires_at,
            })
            .to_string(),
        )
        .unwrap();
    }

    #[test]
    fn direct_human_needs_no_runtime_requester() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_at(&SessionEnvironment::default(), root.path(), 100).unwrap(),
            BrainCreationAuthority::DirectHuman
        );
    }

    #[test]
    fn authenticated_runtime_requires_the_matching_live_lease() {
        let root = tempfile::tempdir().unwrap();
        let environment = environment("finitechat", "session-a", ALICE);
        assert!(resolve_at(&environment, root.path(), 100).is_err());
        write_context(root.path(), "session-a", ALICE, 101);
        assert!(matches!(
            resolve_at(&environment, root.path(), 100).unwrap(),
            BrainCreationAuthority::AuthenticatedRequester(_)
        ));
        assert!(resolve_at(&environment, root.path(), 101).is_err());
    }
}
