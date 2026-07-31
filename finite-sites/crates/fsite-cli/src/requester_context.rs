//! Turn-scoped authenticated requester attribution for Project Init.
//!
//! Hermes owns sender authentication and writes a short-lived local lease
//! immediately around a terminal tool call. `fsite` only consumes a lease
//! when the child process carries the matching task-local Hermes context.

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use finitesites_proto::npub;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

const REQUESTER_CONTEXT_DIR: &str = "requester-context-v1";
const REQUESTER_CONTEXT_V2_DIR: &str = "requester-context-v2";
const REQUESTER_CONTEXT_VERSION: u32 = 1;
const REQUESTER_CONTEXT_V2_VERSION: u32 = 2;
const MAX_CONTEXT_BYTES: u64 = 4096;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ResolvedRequesterContext {
    pub requesting_user_npub: Option<String>,
    pub owner_email: Option<String>,
    pub hosted_requester_assertion: Option<String>,
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

    fn can_infer(&self) -> bool {
        matches!(
            self.platform.as_deref().map(str::trim),
            Some("finitechat" | "local")
        ) && self
            .session_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            && self
                .user_id
                .as_deref()
                .is_some_and(|value| npub::pubkey_from_hex_or_npub(value.trim()).is_ok())
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequesterContextV2 {
    version: u32,
    session_key: String,
    platform: String,
    requesting_user_id: String,
    owner_email: String,
    hosted_requester_assertion: String,
    expires_at_unix: u64,
}

pub fn resolve_with_owner(
    explicit: Option<String>,
    explicit_owner_email: Option<String>,
    finite_root: &Path,
) -> Result<ResolvedRequesterContext, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?
        .as_secs();
    resolve_with_owner_at(
        explicit,
        explicit_owner_email,
        &SessionEnvironment::current(),
        finite_root,
        now,
    )
}

pub fn environment_can_infer() -> bool {
    SessionEnvironment::current().can_infer()
}

#[cfg(test)]
fn resolve_at(
    explicit: Option<String>,
    environment: &SessionEnvironment,
    finite_root: &Path,
    now: u64,
) -> Result<Option<String>, String> {
    Ok(resolve_with_owner_at(explicit, None, environment, finite_root, now)?.requesting_user_npub)
}

fn resolve_with_owner_at(
    explicit: Option<String>,
    explicit_owner_email: Option<String>,
    environment: &SessionEnvironment,
    finite_root: &Path,
    now: u64,
) -> Result<ResolvedRequesterContext, String> {
    let inferred_context = active_requester_v2(environment, finite_root, now);
    let inferred = inferred_context
        .as_ref()
        .map(|context| context.requesting_user_id.clone())
        .or_else(|| active_requester(environment, finite_root, now));
    let Some(inferred) = inferred else {
        // Preserve the standalone contract. Agents outside a live authenticated
        // Finite Chat tool call may still provide the explicit requester.
        return Ok(ResolvedRequesterContext {
            requesting_user_npub: explicit,
            owner_email: explicit_owner_email,
            hosted_requester_assertion: None,
        });
    };
    if let Some(explicit) = explicit {
        let explicit_pubkey = npub::pubkey_from_hex_or_npub(&explicit)
            .map_err(|error| format!("invalid --requesting-user-npub: {error}"))?;
        let inferred_pubkey = npub::pubkey_from_hex_or_npub(&inferred)
            .map_err(|error| format!("invalid authenticated requester context: {error}"))?;
        if explicit_pubkey != inferred_pubkey {
            return Err(
                "--requesting-user-npub disagrees with the active authenticated Finite Chat sender"
                    .to_string(),
            );
        }
    }
    let (owner_email, hosted_requester_assertion) = match inferred_context {
        Some(context) => {
            if let Some(explicit_email) = explicit_owner_email
                && !explicit_email.eq_ignore_ascii_case(&context.owner_email)
            {
                return Err(
                    "--owner-email disagrees with the active verified Finite requester".to_string(),
                );
            }
            (
                Some(context.owner_email),
                Some(context.hosted_requester_assertion),
            )
        }
        None => (explicit_owner_email, None),
    };
    Ok(ResolvedRequesterContext {
        requesting_user_npub: Some(inferred),
        owner_email,
        hosted_requester_assertion,
    })
}

fn active_requester(
    environment: &SessionEnvironment,
    finite_root: &Path,
    now: u64,
) -> Option<String> {
    if !environment.can_infer() {
        return None;
    }
    let session_key = environment.session_key.as_deref()?.trim();
    let user_id = environment.user_id.as_deref()?.trim();
    if session_key.is_empty() || npub::pubkey_from_hex_or_npub(user_id).is_err() {
        return None;
    }
    let path = requester_context_path(finite_root, session_key);
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_CONTEXT_BYTES {
        return None;
    }
    let mut bytes = String::new();
    File::open(&path)
        .ok()?
        .take(MAX_CONTEXT_BYTES + 1)
        .read_to_string(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_CONTEXT_BYTES {
        return None;
    }
    let context: RequesterContext = serde_json::from_str(&bytes).ok()?;
    if context.version != REQUESTER_CONTEXT_VERSION
        || context.platform != "finitechat"
        || context.session_key != session_key
        || context.requesting_user_id != user_id
        || context.expires_at_unix <= now
    {
        if context.expires_at_unix <= now {
            let _ = std::fs::remove_file(path);
        }
        return None;
    }
    Some(user_id.to_string())
}

fn active_requester_v2(
    environment: &SessionEnvironment,
    finite_root: &Path,
    now: u64,
) -> Option<RequesterContextV2> {
    if !environment.can_infer() {
        return None;
    }
    let session_key = environment.session_key.as_deref()?.trim();
    let user_id = environment.user_id.as_deref()?.trim();
    let path = requester_context_path_in(REQUESTER_CONTEXT_V2_DIR, finite_root, session_key);
    let bytes = read_context_file(&path)?;
    let context: RequesterContextV2 = serde_json::from_str(&bytes).ok()?;
    if context.version != REQUESTER_CONTEXT_V2_VERSION
        || context.platform != "finitechat"
        || context.session_key != session_key
        || context.requesting_user_id != user_id
        || context.owner_email.trim().is_empty()
        || context.owner_email.len() > 320
        || context.hosted_requester_assertion.trim().is_empty()
        || context.hosted_requester_assertion.len() > 512
        || context.expires_at_unix <= now
    {
        if context.expires_at_unix <= now {
            let _ = std::fs::remove_file(path);
        }
        return None;
    }
    Some(context)
}

fn read_context_file(path: &Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_CONTEXT_BYTES {
        return None;
    }
    let mut bytes = String::new();
    File::open(path)
        .ok()?
        .take(MAX_CONTEXT_BYTES + 1)
        .read_to_string(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_CONTEXT_BYTES).then_some(bytes)
}

fn requester_context_path(finite_root: &Path, session_key: &str) -> PathBuf {
    requester_context_path_in(REQUESTER_CONTEXT_DIR, finite_root, session_key)
}

fn requester_context_path_in(dir: &str, finite_root: &Path, session_key: &str) -> PathBuf {
    let digest = Sha256::digest(session_key.as_bytes());
    finite_root.join(dir).join(format!("{digest:x}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BOB: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn environment(platform: &str, session_key: &str, user_id: &str) -> SessionEnvironment {
        SessionEnvironment {
            platform: Some(platform.to_string()),
            session_key: Some(session_key.to_string()),
            user_id: Some(user_id.to_string()),
        }
    }

    fn write_context(finite_root: &Path, session_key: &str, user_id: &str, expires_at_unix: u64) {
        let path = requester_context_path(finite_root, session_key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            serde_json::json!({
                "version": 1,
                "session_key": session_key,
                "platform": "finitechat",
                "requesting_user_id": user_id,
                "expires_at_unix": expires_at_unix,
            })
            .to_string(),
        )
        .unwrap();
    }

    fn write_context_v2(
        finite_root: &Path,
        session_key: &str,
        user_id: &str,
        owner_email: &str,
        assertion: &str,
        expires_at_unix: u64,
    ) {
        let path = requester_context_path_in(REQUESTER_CONTEXT_V2_DIR, finite_root, session_key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            serde_json::json!({
                "version": 2,
                "session_key": session_key,
                "platform": "finitechat",
                "requesting_user_id": user_id,
                "owner_email": owner_email,
                "hosted_requester_assertion": assertion,
                "expires_at_unix": expires_at_unix,
            })
            .to_string(),
        )
        .unwrap();
    }

    #[test]
    fn v2_lease_carries_verified_owner_and_rejects_explicit_disagreement() {
        let finite_root = tempfile::tempdir().unwrap();
        write_context(finite_root.path(), "session-a", ALICE, 200);
        write_context_v2(
            finite_root.path(),
            "session-a",
            ALICE,
            "paul@finite.vip",
            "assertion-1",
            200,
        );
        let environment = environment("finitechat", "session-a", ALICE);
        let resolved =
            resolve_with_owner_at(None, None, &environment, finite_root.path(), 100).unwrap();
        assert_eq!(resolved.requesting_user_npub.as_deref(), Some(ALICE));
        assert_eq!(resolved.owner_email.as_deref(), Some("paul@finite.vip"));
        assert_eq!(
            resolved.hosted_requester_assertion.as_deref(),
            Some("assertion-1")
        );
        assert!(
            resolve_with_owner_at(
                None,
                Some("other@example.com".to_owned()),
                &environment,
                finite_root.path(),
                100,
            )
            .unwrap_err()
            .contains("disagrees")
        );
    }

    #[test]
    fn infers_only_the_matching_active_finite_sender() {
        let finite_root = tempfile::tempdir().unwrap();
        write_context(finite_root.path(), "session-a", ALICE, 101);
        assert_eq!(
            resolve_at(
                None,
                &environment("finitechat", "session-a", ALICE),
                finite_root.path(),
                100,
            )
            .unwrap(),
            Some(ALICE.to_string())
        );
        assert_eq!(
            resolve_at(
                None,
                &environment("local", "session-a", ALICE),
                finite_root.path(),
                100,
            )
            .unwrap(),
            Some(ALICE.to_string()),
            "Hermes 0.18.2 exposes the Finite plugin as LOCAL"
        );
        assert_eq!(
            resolve_at(
                None,
                &environment("finitechat", "session-a", BOB),
                finite_root.path(),
                100,
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn dry_run_and_apply_resolve_the_same_active_sender() {
        let finite_root = tempfile::tempdir().unwrap();
        write_context(finite_root.path(), "session-a", ALICE, 200);
        let environment = environment("finitechat", "session-a", ALICE);
        let dry_run = resolve_at(None, &environment, finite_root.path(), 100).unwrap();
        let apply = resolve_at(None, &environment, finite_root.path(), 101).unwrap();
        assert_eq!(dry_run, apply);
        assert_eq!(apply.as_deref(), Some(ALICE));
    }

    #[test]
    fn missing_invalid_and_non_finite_contexts_do_not_infer() {
        let finite_root = tempfile::tempdir().unwrap();
        write_context(finite_root.path(), "wrong-platform", ALICE, 200);
        assert_eq!(
            resolve_at(
                None,
                &SessionEnvironment::default(),
                finite_root.path(),
                100
            )
            .unwrap(),
            None
        );
        assert_eq!(
            resolve_at(
                None,
                &environment("finitechat", "wrong-platform", "not-a-pubkey"),
                finite_root.path(),
                100,
            )
            .unwrap(),
            None
        );
        assert_eq!(
            resolve_at(
                None,
                &environment("telegram", "wrong-platform", ALICE),
                finite_root.path(),
                100,
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn missing_post_expiry_fails_closed_and_removes_the_lease() {
        let finite_root = tempfile::tempdir().unwrap();
        write_context(finite_root.path(), "session-a", ALICE, 100);
        let path = requester_context_path(finite_root.path(), "session-a");
        assert!(path.exists());
        assert_eq!(
            resolve_at(
                None,
                &environment("finitechat", "session-a", ALICE),
                finite_root.path(),
                100,
            )
            .unwrap(),
            None
        );
        assert!(!path.exists());
    }

    #[test]
    fn explicit_standalone_requester_is_preserved_without_an_active_lease() {
        let finite_root = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_at(
                Some(ALICE.to_string()),
                &SessionEnvironment::default(),
                finite_root.path(),
                100,
            )
            .unwrap(),
            Some(ALICE.to_string())
        );
    }

    #[test]
    fn explicit_active_requester_must_match() {
        let finite_root = tempfile::tempdir().unwrap();
        write_context(finite_root.path(), "session-a", ALICE, 200);
        let environment = environment("finitechat", "session-a", ALICE);
        assert_eq!(
            resolve_at(
                Some(ALICE.to_string()),
                &environment,
                finite_root.path(),
                100,
            )
            .unwrap(),
            Some(ALICE.to_string())
        );
        assert!(
            resolve_at(Some(BOB.to_string()), &environment, finite_root.path(), 100,)
                .unwrap_err()
                .contains("disagrees")
        );
    }

    #[test]
    fn removed_restart_context_does_not_infer() {
        let finite_root = tempfile::tempdir().unwrap();
        write_context(finite_root.path(), "session-a", ALICE, 200);
        let path = requester_context_path(finite_root.path(), "session-a");
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            resolve_at(
                None,
                &environment("finitechat", "session-a", ALICE),
                finite_root.path(),
                100,
            )
            .unwrap(),
            None
        );
    }
}
