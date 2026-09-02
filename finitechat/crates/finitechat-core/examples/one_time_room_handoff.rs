//! Unmerged operator utility for one exact cross-account Room migration.
//!
//! It opens only scratch copies carrying an explicit marker, never accepts
//! account secrets on the command line, and never serializes decrypted history.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use finite_identity::{FiniteIdentity, IdentityPaths};
use finitechat_core::{
    AppAction, AppProfileSummary, FiniteChatRuntime, OneTimeRoomHandoffEvidence,
    OneTimeRoomHandoffIntent, OneTimeRoomHandoffPreparedRemoval, OpenOptions, npub_from_account_id,
};

const SCRATCH_MARKER: &str = ".finitechat-one-time-room-handoff-scratch";

fn main() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 7
        || !matches!(
            args[0].as_str(),
            "inspect" | "join" | "plan" | "apply" | "prepare-remove" | "submit-remove" | "verify"
        )
    {
        return Err(format!(
            "usage: one_time_room_handoff <inspect|join|plan|apply|prepare-remove|submit-remove|verify> \
             INTENT.json EVIDENCE.json REMOVAL.json SOURCE_USER_ROOT TARGET_USER_ROOT SERVER_URL\n\
             both user roots must contain {SCRATCH_MARKER} with the exact migration_id"
        ));
    }
    let command = &args[0];
    let intent_path = PathBuf::from(&args[1]);
    let evidence_path = PathBuf::from(&args[2]);
    let removal_path = PathBuf::from(&args[3]);
    let source_root = canonical_scratch_root(Path::new(&args[4]))?;
    let target_root = canonical_scratch_root(Path::new(&args[5]))?;
    if source_root == target_root {
        return Err("source and target scratch roots must be different".to_owned());
    }
    let server_url = &args[6];
    if !(server_url.starts_with("http://127.0.0.1:") || server_url.starts_with("http://localhost:"))
    {
        return Err("this unmerged utility only connects to a loopback Room server".to_owned());
    }

    let intent: OneTimeRoomHandoffIntent = read_json(&intent_path)?;
    require_marker(&source_root, &intent.migration_id)?;
    require_marker(&target_root, &intent.migration_id)?;
    let source = open_runtime(&source_root, server_url)?;
    let target = open_runtime(&target_root, server_url)?;
    let source_state = source.state().map_err(|error| error.to_string())?;
    let target_state = target.state().map_err(|error| error.to_string())?;
    if source_state.identity.account_id != intent.source.account_id
        || source_state.identity.device_id != intent.source.device_id
        || target_state.identity.account_id != intent.target.account_id
        || target_state.identity.device_id != intent.target.device_id
    {
        return Err(
            "the opened scratch stores do not match the exact intent identities".to_owned(),
        );
    }

    if command == "inspect" {
        source
            .dispatch_and_wait(AppAction::StartRuntime)
            .map_err(|error| error.to_string())?;
        let source_state = source
            .dispatch_and_wait(AppAction::OpenRoom {
                room_id: intent.room_id.clone(),
            })
            .map_err(|error| error.to_string())?;
        let mut target_state = target
            .dispatch_and_wait(AppAction::StartRuntime)
            .map_err(|error| error.to_string())?;
        if let Some(room_id) = target_state.rooms.first().map(|room| room.room_id.clone()) {
            target_state = target
                .dispatch_and_wait(AppAction::OpenRoom { room_id })
                .map_err(|error| error.to_string())?;
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "source_rooms": source_state.rooms,
                "source_selected_room_details": source_state.room_details,
                "target_rooms": target_state.rooms,
                "target_selected_room_details": target_state.room_details,
            }))
            .map_err(|error| error.to_string())?
        );
        return Ok(());
    }

    if command == "verify" {
        let evidence: OneTimeRoomHandoffEvidence = read_json(&evidence_path)?;
        let target_state = target
            .dispatch_and_wait(AppAction::StartRuntime)
            .and_then(|_| {
                target.dispatch_and_wait(AppAction::OpenRoom {
                    room_id: intent.room_id.clone(),
                })
            })
            .map_err(|error| error.to_string())?;
        let projected_chat_message_count = target_state
            .topics
            .iter()
            .filter(|topic| topic.room_id == intent.room_id)
            .map(|topic| u64::from(topic.message_count))
            .sum::<u64>();
        if projected_chat_message_count != evidence.projected_chat_message_count {
            return Err(format!(
                "target projects {projected_chat_message_count} chat messages; expected {}",
                evidence.projected_chat_message_count
            ));
        }
        let details = target_state
            .room_details
            .as_ref()
            .ok_or_else(|| "target did not expose canonical Room details".to_owned())?;
        let mut actual_member_account_ids = details
            .members
            .iter()
            .map(|member| member.account_id.clone())
            .collect::<Vec<_>>();
        actual_member_account_ids.sort();
        actual_member_account_ids.dedup();
        let mut expected_member_account_ids = intent
            .expected_member_account_ids
            .iter()
            .filter(|account_id| *account_id != &intent.source.account_id)
            .cloned()
            .collect::<Vec<_>>();
        expected_member_account_ids.sort();
        if actual_member_account_ids != expected_member_account_ids {
            return Err("final canonical Room membership does not match the intent".to_owned());
        }
        if target_state
            .paired_agent
            .as_ref()
            .map(|pair| &pair.canonical_room_id)
            != Some(&intent.room_id)
        {
            return Err("target is not paired to the historical canonical Room".to_owned());
        }
        let mut target_room_ids = target_state
            .rooms
            .iter()
            .map(|room| room.room_id.clone())
            .collect::<Vec<_>>();
        target_room_ids.sort();
        let mut expected_target_room_ids = intent.expected_target_other_room_ids.clone();
        expected_target_room_ids.push(intent.room_id.clone());
        expected_target_room_ids.sort();
        if target_room_ids != expected_target_room_ids {
            return Err("target contains an unrelated source Room".to_owned());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "migration_id": intent.migration_id,
                "room_id": intent.room_id,
                "projected_chat_message_count": projected_chat_message_count,
                "member_account_ids": actual_member_account_ids,
                "target_room_ids": target_room_ids,
                "paired_agent": target_state.paired_agent,
            }))
            .map_err(|error| error.to_string())?
        );
        return Ok(());
    }

    if command == "join" {
        ensure_target_joined(&source, &target, &intent)?;
        let evidence = source
            .export_one_time_cross_account_room_handoff_and_wait(intent)
            .map_err(|error| error.to_string())?
            .evidence();
        println!(
            "{}",
            serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
        );
        return Ok(());
    }

    if command == "prepare-remove" {
        let evidence: OneTimeRoomHandoffEvidence = read_json(&evidence_path)?;
        refuse_overwrite(&removal_path, "source-removal ledger")?;
        let prepared = target
            .prepare_one_time_room_handoff_source_removal_and_wait(intent, evidence)
            .map_err(|error| error.to_string())?;
        write_json_create_new(&removal_path, &prepared)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&prepared).map_err(|error| error.to_string())?
        );
        return Ok(());
    }

    if command == "submit-remove" {
        let prepared: OneTimeRoomHandoffPreparedRemoval = read_json(&removal_path)?;
        if prepared.intent != intent {
            return Err("the recorded source-removal request does not match the intent".to_owned());
        }
        let report = target
            .submit_one_time_room_handoff_source_removal_and_wait(prepared)
            .map_err(|error| error.to_string())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
        );
        return Ok(());
    }

    let bundle = source
        .export_one_time_cross_account_room_handoff_and_wait(intent.clone())
        .map_err(|error| error.to_string())?;
    let observed = bundle.evidence();
    if command == "plan" {
        refuse_overwrite(&evidence_path, "evidence")?;
        write_json_create_new(&evidence_path, &observed)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&observed).map_err(|error| error.to_string())?
        );
        return Ok(());
    }

    let expected: OneTimeRoomHandoffEvidence = read_json(&evidence_path)?;
    if expected != observed {
        return Err(format!(
            "live scratch evidence changed since plan\nexpected={}\nobserved={}",
            serde_json::to_string(&expected).map_err(|error| error.to_string())?,
            serde_json::to_string(&observed).map_err(|error| error.to_string())?
        ));
    }
    let report = target
        .import_one_time_cross_account_room_handoff_and_wait(intent, expected, bundle)
        .map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn ensure_target_joined(
    source: &FiniteChatRuntime,
    target: &FiniteChatRuntime,
    intent: &OneTimeRoomHandoffIntent,
) -> Result<(), String> {
    let target_state = target
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(|error| error.to_string())?;
    let target_already_joined = target_state
        .rooms
        .iter()
        .any(|room| room.room_id == intent.room_id);
    require_exact_target_rooms(&target_state, intent, target_already_joined)?;
    if !target_already_joined {
        source
            .dispatch_and_wait(AppAction::StartRuntime)
            .map_err(|error| error.to_string())?;
        let profile = AppProfileSummary {
            account_id: intent.target.account_id.clone(),
            npub: npub_from_account_id(intent.target.account_id.clone())
                .map_err(|error| error.to_string())?,
            display_name: "one-time migration target".to_owned(),
            about: None,
            picture: None,
            stale: false,
            is_agent: false,
        };
        source
            .dispatch_and_wait(AppAction::AddRoomMembers {
                room_id: intent.room_id.clone(),
                profiles: vec![profile],
            })
            .map_err(|error| error.to_string())?;
    }
    let joined = target
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(|error| error.to_string())?;
    if !joined
        .rooms
        .iter()
        .any(|room| room.room_id == intent.room_id)
    {
        return Err("target did not durably join the exact intended Room".to_owned());
    }
    require_exact_target_rooms(&joined, intent, true)?;
    Ok(())
}

fn require_exact_target_rooms(
    state: &finitechat_core::AppState,
    intent: &OneTimeRoomHandoffIntent,
    include_canonical: bool,
) -> Result<(), String> {
    let mut actual = state
        .rooms
        .iter()
        .map(|room| room.room_id.clone())
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = intent.expected_target_other_room_ids.clone();
    if include_canonical {
        expected.push(intent.room_id.clone());
    }
    expected.sort();
    if actual != expected {
        return Err(format!(
            "target Room isolation mismatch before join: expected {expected:?}, found {actual:?}"
        ));
    }
    Ok(())
}

fn refuse_overwrite(path: &Path, label: &str) -> Result<(), String> {
    if path.exists() {
        return Err(format!(
            "refusing to overwrite existing {label} file {}",
            path.display()
        ));
    }
    Ok(())
}

fn canonical_scratch_root(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("failed to resolve {}: {error}", path.display()))
}

fn require_marker(root: &Path, migration_id: &str) -> Result<(), String> {
    let marker = root.join(SCRATCH_MARKER);
    let value = fs::read_to_string(&marker)
        .map_err(|error| format!("failed to read {}: {error}", marker.display()))?;
    if value.trim() != migration_id {
        return Err(format!(
            "scratch marker {} does not contain the exact migration_id",
            marker.display()
        ));
    }
    Ok(())
}

fn open_runtime(
    user_root: &Path,
    server_url: &str,
) -> Result<std::sync::Arc<FiniteChatRuntime>, String> {
    let identity = FiniteIdentity::load(&IdentityPaths::with_finite_home(
        user_root.join("finite-home"),
    ))
    .map_err(|error| {
        format!(
            "failed to load identity under {}: {error}",
            user_root.display()
        )
    })?;
    FiniteChatRuntime::open(OpenOptions {
        data_dir: user_root.join("chat").to_string_lossy().into_owned(),
        server_url: server_url.to_owned(),
        device_id: "hosted-web".to_owned(),
        account_secret_hex: Some(hex::encode(identity.expose_secret_bytes())),
        now_unix_seconds: None,
    })
    .map_err(|error| error.to_string())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn write_json_create_new<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    use std::io::Write as _;
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
    file.write_all(&bytes)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("failed to persist {}: {error}", path.display()))
}
