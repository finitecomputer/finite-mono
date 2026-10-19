//! `fbrain admin role revoke admin`: remove the admin role and, in the same
//! server transaction, rotate every Folder Key the key reached only through
//! that role, so the demoted key cannot open those Folders' new versions.

use std::io::Write;

use finite_brain_core::AdminAccessAction;
use serde::Deserialize;

use crate::{
    BrainMetadataView, CliEnvironment, CliError, FolderKeyRotationIntent, FolderKeyRotationReceipt,
    PreparedFolderKeyRotation, admin_access_change_event, command_brain_id, fetch_brain_metadata,
    folder_required_recipients, mutate_agent_state, prepare_folder_key_rotation,
    receipt_proves_prepared, required_option_or_positional, resolve_identity_npub,
    signed_json_request, write_json,
};

#[derive(Debug, Deserialize)]
struct DemoteAdminResponse {
    #[serde(flatten)]
    metadata: BrainMetadataView,
    rotations: Vec<FolderKeyRotationReceipt>,
}

pub(crate) fn demotion_route(brain_id: &str, target: &str) -> String {
    format!("/v1/admin/brains/{brain_id}/roles/admin/{target}/demotion")
}

/// Brain metadata as it will read once `target` is no longer an admin.
pub(crate) fn metadata_after_admin_demotion(
    metadata: &BrainMetadataView,
    target: &str,
) -> BrainMetadataView {
    let mut updated = metadata.clone();
    updated.admins.retain(|admin| admin != target);
    updated
}

/// Folders whose current key `target` holds only as an admin: a required
/// recipient now and not after the demotion. Folders it still reaches through
/// explicit Folder access, All-Members membership or the Personal Agent
/// relationship are left alone. The server computes the same set and refuses
/// any other.
pub(crate) fn admin_demotion_rotation_folders(
    metadata: &BrainMetadataView,
    target: &str,
) -> Result<Vec<String>, CliError> {
    let demoted = metadata_after_admin_demotion(metadata, target);
    let mut folder_ids = Vec::new();
    for folder in &metadata.folders {
        let before = folder_required_recipients(metadata, &folder.access, &folder.access_user_ids)?;
        if !before.iter().any(|recipient| recipient == target) {
            continue;
        }
        let after = folder_required_recipients(&demoted, &folder.access, &folder.access_user_ids)?;
        if !after.iter().any(|recipient| recipient == target) {
            folder_ids.push(folder.id.clone());
        }
    }
    Ok(folder_ids)
}

pub(crate) fn admin_role_revoke<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let brain_id = command_brain_id(args, env)?;
    let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
    let target = resolve_identity_npub(env, args, &raw_target)?;
    let metadata = fetch_brain_metadata(env, args, &brain_id)?;
    if !metadata.admins.iter().any(|admin| admin == &target) {
        return Err(CliError::InvalidInput(format!(
            "{target} is not an admin of Brain {brain_id}; nothing was changed"
        )));
    }
    let post_demotion = metadata_after_admin_demotion(&metadata, &target);
    let folder_ids = admin_demotion_rotation_folders(&metadata, &target)?;
    // Prepare every rotation before sending anything: a Folder Key this
    // signer cannot open stops the demotion with nothing changed.
    let mut prepared = Vec::with_capacity(folder_ids.len());
    for folder_id in &folder_ids {
        let rotation = prepare_folder_key_rotation(
            env,
            args,
            &post_demotion,
            &brain_id,
            folder_id,
            FolderKeyRotationIntent::Rotate,
        )
        .map_err(|error| cannot_prepare_rotation(error, &target, folder_id))?;
        prepared.push((folder_id.clone(), rotation));
    }
    let event = admin_access_change_event(
        env,
        &brain_id,
        AdminAccessAction::RemoveAdmin,
        None,
        Some(&target),
        None,
    )?;
    let rotations = prepared
        .iter()
        .map(|(folder_id, rotation)| {
            let mut body = rotation.body.clone();
            body["folderId"] = serde_json::Value::String(folder_id.clone());
            body
        })
        .collect::<Vec<_>>();
    let route = demotion_route(&brain_id, &target);
    let response = signed_json_request(
        env,
        args,
        "POST",
        &route,
        Some(serde_json::json!({
            "accessChangeEvent": event,
            "rotations": rotations,
        })),
    )
    .map_err(|error| unsupported_demotion_server(error, &route, &folder_ids))?;
    let response = verified_demotion(response, &target, &prepared)?;

    let summary = format!(
        "admin role revoked brain={brain_id} target={target} rotatedFolders={}",
        response.rotations.len()
    );
    // Best effort: the local activity log exists only inside a Working Tree.
    let _ = mutate_agent_state(env, |state, now| {
        state.add_activity(now.clone(), "admin.role.revoked", summary.clone());
        for receipt in &response.rotations {
            state.add_activity(
                now.clone(),
                "admin.folder_key.rotated",
                format!(
                    "Folder Key rotated brain={brain_id} folder={} keyVersion={} cutOff={}",
                    receipt.folder_id,
                    receipt.new_key_version,
                    receipt.cut_off_npubs.len()
                ),
            );
        }
        Ok(())
    });
    if json {
        let mut receipt = serde_json::to_value(&response.metadata)?;
        receipt["state"] = "complete".into();
        receipt["operation"] = "admin-role-revoke".into();
        receipt["targetNpub"] = target.clone().into();
        receipt["rotations"] = response
            .rotations
            .iter()
            .map(|rotation| {
                serde_json::json!({
                    "folderId": rotation.folder_id,
                    "previousKeyVersion": rotation.previous_key_version,
                    "newKeyVersion": rotation.new_key_version,
                    "recipientNpubs": rotation.recipient_npubs,
                    "cutOffNpubs": rotation.cut_off_npubs,
                })
            })
            .collect();
        return write_json(output, &receipt);
    }
    writeln!(
        output,
        "complete admin demotion brain={brain_id} target={target} rotatedFolders={}",
        response.rotations.len()
    )?;
    for rotation in &response.rotations {
        writeln!(
            output,
            "rotated folder={} keyVersion={}->{} recipients={} cutOff={}",
            rotation.folder_id,
            rotation.previous_key_version,
            rotation.new_key_version,
            rotation.recipient_npubs.len(),
            rotation.cut_off_npubs.len()
        )?;
    }
    Ok(())
}

/// Accept the demotion only when the server proves it: the key is no longer
/// an admin, it is still a Member, and every prepared rotation (and nothing
/// else) committed for exactly the prepared recipients.
fn verified_demotion(
    response: serde_json::Value,
    target: &str,
    prepared: &[(String, PreparedFolderKeyRotation)],
) -> Result<DemoteAdminResponse, CliError> {
    let response: DemoteAdminResponse = serde_json::from_value(response)
        .map_err(|error| CliError::HttpResponseDecode(error.to_string()))?;
    let role_removed = !response.metadata.admins.iter().any(|admin| admin == target)
        && response
            .metadata
            .members
            .iter()
            .any(|member| member == target);
    let rotations_match = response.rotations.len() == prepared.len()
        && prepared.iter().all(|(folder_id, rotation)| {
            response.rotations.iter().any(|receipt| {
                receipt.folder_id == *folder_id
                    && receipt_proves_prepared(receipt, &response.metadata, folder_id, rotation)
            })
        });
    if !role_removed || !rotations_match {
        return Err(CliError::InvalidInput(
            "admin demotion postcondition was not authoritative".to_owned(),
        ));
    }
    Ok(response)
}

/// Name the Folder the acting key could not rotate; nothing was sent.
fn cannot_prepare_rotation(error: CliError, target: &str, folder_id: &str) -> CliError {
    match error {
        CliError::GrantOpening { .. } | CliError::InvalidInput(_) | CliError::NotFound(_) => {
            CliError::InvalidInput(format!(
                "cannot revoke admin from {target}: Folder {folder_id} must be rotated so the demoted key cannot read it, and this key cannot prepare that rotation ({error}); nothing was changed. Ask an admin who can open Folder {folder_id} to run the demotion"
            ))
        }
        other => other,
    }
}

/// An older Brain server answers the demotion route with its unknown-route
/// 404 (or 405). Nothing was changed; say so instead of a raw error, and do
/// not fall back to a demotion that leaves the key able to read.
fn unsupported_demotion_server(error: CliError, route: &str, folder_ids: &[String]) -> CliError {
    let unsupported = match &error {
        CliError::HttpStatus { status: 404, body } => body.contains("route not found"),
        CliError::HttpStatus { status: 405, .. } => true,
        _ => false,
    };
    if !unsupported {
        return error;
    }
    let consequence = if folder_ids.is_empty() {
        String::new()
    } else {
        format!(
            " Revoking this admin must rotate the Folder Keys of [{}].",
            folder_ids.join(", ")
        )
    };
    CliError::Unsupported(format!(
        "this Brain server does not support admin demotion with Folder Key rotation yet (POST {route}); nothing was changed.{consequence} Upgrade the Brain server and try again"
    ))
}
