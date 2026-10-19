//! `fbrain admin folder-access rotate`: rotate one Folder Key for exactly its
//! required recipients, cutting off every key that holds the current wrap
//! without entitlement (the access report's `revocationIncomplete`).

use std::collections::BTreeSet;
use std::io::Write;

use serde::Deserialize;

use crate::{
    BrainMetadataView, CliEnvironment, CliError, FolderKeyRotationIntent,
    PreparedFolderKeyRotation, command_brain_id, current_folder_id, fetch_brain_metadata,
    mutate_agent_state, option_value, prepare_folder_key_rotation, signed_json_request,
    write_json,
};

/// Server receipt for one committed Folder Key rotation.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FolderKeyRotationReceipt {
    folder_id: String,
    previous_key_version: u32,
    new_key_version: u32,
    recipient_npubs: Vec<String>,
    cut_off_npubs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RotateFolderKeyResponse {
    #[serde(flatten)]
    metadata: BrainMetadataView,
    rotation: FolderKeyRotationReceipt,
}

pub(crate) fn rotation_route(brain_id: &str, folder_id: &str) -> String {
    format!("/v1/admin/brains/{brain_id}/folders/{folder_id}/key-rotations")
}

pub(crate) fn access_rotate<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let brain_id = command_brain_id(args, env)?;
    let folder_id = option_value(args, "--folder")
        .or_else(|| current_folder_id(env).ok())
        .ok_or(CliError::MissingArgument("--folder"))?;
    let metadata = fetch_brain_metadata(env, args, &brain_id)?;
    let prepared = prepare_folder_key_rotation(
        env,
        args,
        &metadata,
        &brain_id,
        &folder_id,
        FolderKeyRotationIntent::Rotate,
    )?;
    let route = rotation_route(&brain_id, &folder_id);
    let response = signed_json_request(env, args, "POST", &route, Some(prepared.body.clone()))
        .map_err(|error| unsupported_rotation_server(error, &route))?;
    let receipt = verified_rotation(response, &folder_id, &prepared)?;
    let summary = format!(
        "Folder Key rotated brain={brain_id} folder={folder_id} keyVersion={} cutOff={}",
        receipt.new_key_version,
        receipt.cut_off_npubs.len()
    );
    // Best effort: the local activity log exists only inside a Working Tree.
    let _ = mutate_agent_state(env, |state, now| {
        state.add_activity(now, "admin.folder_key.rotated", summary.clone());
        Ok(())
    });
    let receipt_json = serde_json::json!({
        "state": "complete",
        "operation": "admin-folder-access-rotate",
        "brainId": brain_id,
        "folderId": folder_id,
        "previousKeyVersion": receipt.previous_key_version,
        "newKeyVersion": receipt.new_key_version,
        "recipientNpubs": receipt.recipient_npubs,
        "cutOffNpubs": receipt.cut_off_npubs,
    });
    if json {
        return write_json(output, &receipt_json);
    }
    writeln!(
        output,
        "complete Folder Key rotation brain={brain_id} folder={folder_id} keyVersion={}->{} recipients={} cutOff={}",
        receipt.previous_key_version,
        receipt.new_key_version,
        receipt.recipient_npubs.len(),
        receipt.cut_off_npubs.len()
    )?;
    for npub in &receipt.cut_off_npubs {
        writeln!(output, "cut off {npub}")?;
    }
    Ok(())
}

/// Accept the server's receipt only when it proves what was prepared: the
/// Folder advanced exactly one version and the new key is wrapped for exactly
/// the recipients this client wrapped for.
fn verified_rotation(
    response: serde_json::Value,
    folder_id: &str,
    prepared: &PreparedFolderKeyRotation,
) -> Result<FolderKeyRotationReceipt, CliError> {
    let response: RotateFolderKeyResponse = serde_json::from_value(response)
        .map_err(|error| CliError::HttpResponseDecode(error.to_string()))?;
    let receipt = response.rotation;
    let metadata_version = response
        .metadata
        .folders
        .iter()
        .find(|folder| folder.id == folder_id)
        .map(|folder| folder.current_key_version);
    let recipients = receipt.recipient_npubs.iter().collect::<BTreeSet<_>>();
    let expected = prepared.recipients.iter().collect::<BTreeSet<_>>();
    if receipt.folder_id != folder_id
        || receipt.previous_key_version != prepared.previous_key_version
        || receipt.new_key_version != prepared.new_key_version
        || metadata_version != Some(prepared.new_key_version)
        || recipients != expected
        || receipt.recipient_npubs.len() != expected.len()
    {
        return Err(CliError::InvalidInput(
            "Folder Key rotation postcondition was not authoritative".to_owned(),
        ));
    }
    Ok(receipt)
}

/// An older Brain server answers the new route with its unknown-route 404 (or
/// 405). Nothing was changed; say so instead of surfacing a raw error.
fn unsupported_rotation_server(error: CliError, route: &str) -> CliError {
    match error {
        CliError::HttpStatus { status: 404, body } if body.contains("route not found") => {
            unsupported_rotation(route)
        }
        CliError::HttpStatus { status: 405, .. } => unsupported_rotation(route),
        other => other,
    }
}

fn unsupported_rotation(route: &str) -> CliError {
    CliError::Unsupported(format!(
        "this Brain server does not support Folder Key rotation yet (POST {route}); nothing was changed. Upgrade the Brain server, or revoke a specific key with `fbrain admin folder-access revoke --folder <id> --target <npub>`"
    ))
}
