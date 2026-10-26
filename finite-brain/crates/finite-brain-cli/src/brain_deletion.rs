//! `fbrain brain delete`: preview an Organization Brain deletion, then confirm
//! exactly the previewed state. Also the client side of a deleted Brain: a
//! Working Tree whose Brain is gone stops syncing for good and leaves every
//! file, including unsynced local changes, where it is.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::*;

/// Unsynced paths named in the terminal message; the full list stays in
/// Agent State.
const LISTED_UNSYNCED_PATHS: usize = 20;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrainDeletionPreviewView {
    pub(crate) server: String,
    pub(crate) brain_id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) folder_count: usize,
    pub(crate) page_count: usize,
    pub(crate) member_count: usize,
    pub(crate) admin_count: usize,
    pub(crate) guest_count: usize,
    pub(crate) pending_invitation_count: usize,
    pub(crate) open_invite_token_count: usize,
    pub(crate) blocking_mounts: Vec<BlockingMountView>,
    pub(crate) retired_mount_record_count: usize,
    pub(crate) deletable: bool,
    pub(crate) state_digest: String,
    pub(crate) confirmation_max_age_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlockingMountView {
    pub(crate) connection_id: String,
    pub(crate) direction: String,
    pub(crate) source_brain_id: String,
    pub(crate) source_folder_id: String,
    pub(crate) destination_brain_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrainDeleteResponseView {
    receipt: BrainDeletionReceiptView,
    duplicate: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrainDeletionReceiptView {
    brain_id: String,
    deletion_event_id: String,
    actor_npub: String,
    deleted_at: String,
    state_digest: String,
    folder_count: usize,
    page_count: usize,
}

/// `fbrain brain delete <brain-id> [--confirm <stateDigest>]`. Without
/// `--confirm` it only previews. There is deliberately no flag that skips
/// the preview: the confirmation names one exact server-computed state.
pub(crate) fn brain_delete<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    for flag in ["--force", "--yes", "-y"] {
        if args.iter().any(|arg| arg == flag) {
            return Err(CliError::InvalidInput(format!(
                "`fbrain brain delete` has no {flag}; preview with `fbrain brain delete <brain-id>`, then confirm that exact state with --confirm <stateDigest>"
            )));
        }
    }
    let brain_id = positional_values(args)
        .get(1)
        .cloned()
        .ok_or(CliError::MissingArgument("brain-id"))?;
    BrainId::new(brain_id.clone()).map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let confirmed = option_value(args, "--confirm");
    if let Some(digest) = confirmed.as_deref()
        && !digest.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return Err(CliError::InvalidInput(
            "--confirm requires the exact lowercase sha256:<64 hex characters> digest from the deletion preview; nothing was deleted".to_owned(),
        ));
    }
    let preview = fetch_brain_deletion_preview(env, args, &brain_id)?;
    let Some(confirmed) = confirmed else {
        return write_preview(output, json, &preview);
    };
    if preview.state_digest != confirmed {
        return Err(CliError::InvalidInput(format!(
            "Brain {brain_id} changed since the preview you confirmed; nothing was deleted. Run `fbrain brain delete {brain_id}` to review the current preview, then confirm it"
        )));
    }
    if !preview.deletable {
        return Err(CliError::InvalidInput(format!(
            "Brain {brain_id} still has active cross-Brain Mounts ({}); revoke them with `fbrain mount revoke <mount-id>` first. Nothing was deleted",
            mount_ids(&preview)
        )));
    }
    let event = admin_access_change_event_with_note(
        env,
        &brain_id,
        AdminAccessAction::DeleteBrain,
        None,
        None,
        None,
        Some(&confirmed),
    )?;
    let body = serde_json::json!({ "deletionEvent": event });
    let path = format!("/v1/brains/{brain_id}");
    // A transport failure leaves the outcome unknown. One exact retry of the
    // same signed deletion either deletes or answers the accepted receipt.
    let response = match signed_json_request(env, args, "DELETE", &path, Some(body.clone())) {
        Err(CliError::Http(_)) => signed_json_request(env, args, "DELETE", &path, Some(body)),
        other => other,
    }
    .map_err(|error| deletion_request_error(error, &brain_id))?;
    let response: BrainDeleteResponseView = serde_json::from_value(response)?;
    if let Some(root) = find_agent_state(&env.cwd)?
        && deletion_is_for_working_tree(env, args, &root, &brain_id)?
    {
        record_brain_deleted(env, &root, &brain_id, &response.receipt.deleted_at)?;
    }
    if json {
        write_json(
            output,
            &serde_json::json!({
                "state": "deleted",
                "duplicate": response.duplicate,
                "server": preview.server,
                "name": preview.name,
                "receipt": response.receipt,
            }),
        )
    } else {
        let receipt = &response.receipt;
        writeln!(
            output,
            "deleted Organization Brain \"{}\" ({}) on {} at {}: {} folders, {} pages",
            preview.name,
            receipt.brain_id,
            preview.server,
            receipt.deleted_at,
            receipt.folder_count,
            receipt.page_count
        )?;
        writeln!(
            output,
            "receipt deletionEvent={} actor={} stateDigest={}{}",
            receipt.deletion_event_id,
            receipt.actor_npub,
            receipt.state_digest,
            if response.duplicate {
                " (already accepted)"
            } else {
                ""
            }
        )?;
        writeln!(
            output,
            "Local Working Trees, exports and other offline copies are not erased; remove them when you no longer need them"
        )?;
        Ok(())
    }
}

fn fetch_brain_deletion_preview(
    env: &CliEnvironment,
    args: &[String],
    brain_id: &str,
) -> Result<BrainDeletionPreviewView, CliError> {
    let response = signed_json_request(
        env,
        args,
        "GET",
        &format!("/v1/brains/{brain_id}/deletion-preview"),
        None,
    )
    .map_err(|error| deletion_request_error(error, brain_id))?;
    Ok(serde_json::from_value(response)?)
}

fn deletion_request_error(error: CliError, brain_id: &str) -> CliError {
    if let Some((_, deleted_at)) = brain_deleted_notice(&error) {
        return CliError::InvalidInput(format!(
            "Brain {brain_id} was already permanently deleted at {deleted_at}"
        ));
    }
    unsupported_on_older_server(error, || {
        CliError::Unsupported(
            "this Brain server cannot delete Brains; upgrade the Brain server".to_owned(),
        )
    })
}

fn mount_ids(preview: &BrainDeletionPreviewView) -> String {
    preview
        .blocking_mounts
        .iter()
        .map(|mount| mount.connection_id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn write_preview<W: Write>(
    output: &mut W,
    json: bool,
    preview: &BrainDeletionPreviewView,
) -> Result<(), CliError> {
    let confirm_command = format!(
        "fbrain brain delete {} --confirm {}",
        preview.brain_id, preview.state_digest
    );
    if json {
        return write_json(
            output,
            &serde_json::json!({
                "state": if preview.deletable { "confirmation-required" } else { "blocked" },
                "preview": preview,
                "confirmCommand": preview.deletable.then_some(confirm_command),
            }),
        );
    }
    writeln!(
        output,
        "Delete Organization Brain \"{}\" ({}) on {}?",
        preview.name, preview.brain_id, preview.server
    )?;
    writeln!(
        output,
        "- {} folders and {} pages, with their server-side history",
        preview.folder_count, preview.page_count
    )?;
    writeln!(
        output,
        "- access for {} members ({} admins) and {} guests",
        preview.member_count, preview.admin_count, preview.guest_count
    )?;
    writeln!(
        output,
        "- {} pending invitations and {} open invite tokens stop working",
        preview.pending_invitation_count, preview.open_invite_token_count
    )?;
    if preview.retired_mount_record_count > 0 {
        writeln!(
            output,
            "- {} pending or revoked mount records naming this Brain are removed",
            preview.retired_mount_record_count
        )?;
    }
    if !preview.deletable {
        writeln!(
            output,
            "Blocked: revoke these active mounts first, then preview again:"
        )?;
        for mount in &preview.blocking_mounts {
            writeln!(
                output,
                "- {} {} {}/{} -> {}: fbrain mount revoke {}",
                mount.connection_id,
                mount.direction,
                mount.source_brain_id,
                mount.source_folder_id,
                mount.destination_brain_id,
                mount.connection_id
            )?;
        }
        writeln!(output, "Nothing was deleted.")?;
        return Ok(());
    }
    writeln!(
        output,
        "This deletes the Brain for everyone and cannot be undone here; only an operator restore from backup can recover it. Local Working Trees and exported copies are not erased."
    )?;
    writeln!(
        output,
        "To delete exactly this state within {} minutes, run:\n  {confirm_command}",
        preview.confirmation_max_age_seconds / 60
    )?;
    writeln!(output, "Nothing was deleted.")?;
    Ok(())
}

/// The `(brain id, deletedAt)` of a Brain server's deleted-Brain answer.
pub(crate) fn brain_deleted_notice(error: &CliError) -> Option<(String, String)> {
    match error {
        CliError::SyncStage { source, .. } => brain_deleted_notice(source),
        CliError::HttpStatus { status: 410, body } => {
            let body = serde_json::from_str::<serde_json::Value>(body).ok()?;
            if body["code"] != "brain_deleted" {
                return None;
            }
            Some((
                body["brainDeleted"]["brainId"].as_str()?.to_owned(),
                body["brainDeleted"]["deletedAt"].as_str()?.to_owned(),
            ))
        }
        _ => None,
    }
}

/// A server override may name a different Brain with the same ID. Only the
/// Working Tree's saved server can make that tree permanently stop syncing.
pub(crate) fn deletion_is_for_working_tree(
    env: &CliEnvironment,
    args: &[String],
    root: &Path,
    brain_id: &str,
) -> Result<bool, CliError> {
    let state = read_agent_state(root)?;
    let Some(saved_server) = state.server_url else {
        return Ok(false);
    };
    Ok(state.brain_id == brain_id
        && saved_server.trim().trim_end_matches('/')
            == server_url_for_command(env, args)?
                .trim()
                .trim_end_matches('/'))
}

/// Mark the Working Tree at `root` as belonging to a deleted Brain: it never
/// syncs again, and its files, including unsynced changes, stay in place.
/// Returns the terminal error every later sync answers with.
pub(crate) fn record_brain_deleted(
    env: &CliEnvironment,
    root: &Path,
    brain_id: &str,
    deleted_at: &str,
) -> Result<CliError, CliError> {
    let unsynced_paths = pending_working_tree_change_paths(root)?;
    let mut tree_env = env.clone();
    tree_env.cwd = root.to_path_buf();
    let mut recorded = None;
    mutate_agent_state(&tree_env, |state, now| {
        let deleted = state
            .brain_deleted
            .get_or_insert_with(|| BrainDeletedState {
                deleted_at: deleted_at.to_owned(),
                detected_at: now.clone(),
                unsynced_paths,
            })
            .clone();
        let error = brain_deleted_error(brain_id, &deleted);
        state.sync.status = AgentSyncStatus::Blocked {
            reason: error.to_string(),
        };
        state.daemon.last_error = Some(error.to_string());
        state.add_activity(
            now,
            "brain.deleted",
            format!(
                "Brain was permanently deleted on the server at {}; sync stopped and local files were left in place",
                deleted.deleted_at
            ),
        );
        recorded = Some(error);
        Ok(())
    })?;
    recorded.ok_or_else(|| CliError::InvalidInput("Agent State was not updated".to_owned()))
}

/// The terminal error for a Working Tree whose Brain was deleted.
pub(crate) fn brain_deleted_error(brain_id: &str, deleted: &BrainDeletedState) -> CliError {
    CliError::BrainDeleted {
        brain_id: brain_id.to_owned(),
        deleted_at: deleted.deleted_at.clone(),
        unsynced_paths: deleted
            .unsynced_paths
            .iter()
            .take(LISTED_UNSYNCED_PATHS)
            .cloned()
            .collect(),
        unsynced_count: deleted.unsynced_paths.len(),
    }
}
