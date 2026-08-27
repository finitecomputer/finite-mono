use std::collections::{BTreeMap, BTreeSet};
use std::fs;
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::os::fd::OwnedFd;
use std::path::{Component, Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use finite_brain_core::portability::{
    BrainWorkingTreeStateManifest, FOLDER_CONVENTION_DIRECTORIES, MAX_WORKING_TREE_ASSET_BYTES,
    OkfOmittedFolder, OpenedPage, WorkingTreeChange, WorkingTreeChangeIntent,
    WorkingTreeFolderRoot, WorkingTreeIntentAction, WorkingTreeIntentContent,
    WorkingTreeIntentRoute, WorkingTreeMaterializeInput, WorkingTreeObjectManifestEntry,
    WorkingTreeProjection, folder_agent_instructions, folder_convention_marker,
    materialize_brain_working_tree, plan_working_tree_change_intents,
};
use finite_brain_core::{
    Brain, BrainId, BrainKind, DecodedSyncPayload, DisplayName, EncryptedFolderObjectEnvelope,
    Folder, FolderAccessMode, FolderId, FolderKey, FolderObjectAad, FolderObjectOperation,
    FolderObjectRevisionPayload, FolderRole, ObjectId, RevisionValidation, SafeRelativePath,
    TombstoneValidation, UserId, decode_sync_payload, encrypt_folder_object, open_folder_object,
    sha256_hex,
};
use finite_nostr::{GiftWrapValidation, NostrPublicKey, open_gift_wrap};
use nostr::{Event, Keys, Kind, Tag};
#[cfg(unix)]
use rustix::fs::{AtFlags, Mode, OFlags, RenameFlags, open, openat, renameat_with, unlinkat};
use serde::Deserialize;

#[cfg(test)]
use crate::initialize_private_working_tree;
use crate::{
    APP_SPECIFIC_KIND, AdminAccessAction, AgentState, AgentSyncStatus, BrainMetadataView,
    CliEnvironment, CliError, CompletedWrapReport, ConflictEntry, ConflictState, LocalSigner,
    SYNC_BOOTSTRAP_RESPONSE_LIMIT_BYTES, SessionFolderKeyring, SyncChangeReport, SyncOnceReport,
    admin_access_change_event, current_tree_root, deterministic_id,
    encrypted_export_response_limit_bytes, folder_key_grant_request, folder_required_recipients,
    load_signer, read_agent_state, read_json_file, read_working_tree_state, server_url_for_command,
    sign_event, signed_json_request, signed_json_request_to_server,
    signed_json_request_to_server_with_response_limit, signed_json_request_with_response_limit,
    tag_vec, timestamp, timestamp_from_unix, unix_timestamp, write_agent_state, write_json_file,
    write_private_file_atomic, write_working_tree_state,
};

const CIPHER_AES_256_GCM: &str = "AES-256-GCM";
const FOLDER_OBJECT_PAGE_VERSION: &str = "finite-folder-object-page-v1";
// Record pages are bounded by count, and each record is bounded by the
// asset/page payload limits, so 100 records stays far under the 128 MB
// sync-surface response cap even for maximal payloads.
const SYNC_RECORDS_PAGE_LIMIT: u64 = 100;
const MAX_WORKING_TREE_FILE_COUNT: usize = 10_000;
const MAX_WORKING_TREE_RECURSION_DEPTH: usize = 32;

pub(crate) fn run_working_tree_sync(
    env: &CliEnvironment,
    args: &[String],
    activity_kind: &str,
) -> Result<SyncOnceReport, CliError> {
    let root = current_tree_root(env)
        .map_err(|error| sync_stage_error("locate Working Tree", &env.cwd, error))?;
    let agent_state = read_agent_state(&root)
        .map_err(|error| sync_stage_error("read Agent State", &root, error))?;
    let prior_tree_state = read_working_tree_state(&root)
        .map_err(|error| sync_stage_error("read Working Tree state", &root, error))?;
    let prior_export_path = root.join(".finitebrain/encrypted-sync/export.json");
    let prior_export = prior_export_path
        .is_file()
        .then(|| read_json_file(&prior_export_path))
        .transpose()
        .map_err(|error| sync_stage_error("read prior encrypted export", &root, error))?;
    let server_url = server_url_for_command(env, args)?;
    let auth = load_signer(env)?;
    let pending_local_changes = scan_working_tree_changes(&root, &prior_tree_state)
        .map_err(|error| sync_stage_error("scan local Working Tree changes", &root, error))?;
    // Routine sync reconciles against the cached export and pulls only the
    // sync records after the last known sequence, so the response scales with
    // new activity instead of total Brain size. The full export is fetched on
    // first open, before local writes (current Folder Key versions must be
    // authoritative), and whenever the incremental path escalates below.
    let use_cached_export = pending_local_changes.is_empty()
        && prior_tree_state.sync.latest_sequence > 0
        && prior_export.is_some();
    let mut export = match (use_cached_export, prior_export.clone()) {
        (true, Some(cached)) => cached,
        _ => fetch_encrypted_export(env, &server_url, &agent_state.brain_id)?,
    };
    let has_known_mounts = prior_tree_state
        .folder_roots
        .iter()
        .any(|folder| folder.source_brain_id.is_some());
    let mut mounted_discovery = if has_known_mounts {
        Some(fetch_mounted_folder_sync_contexts(
            env,
            &server_url,
            &agent_state.brain_id,
            &export,
        )?)
    } else {
        None
    };
    let mut mounted_exports = mounted_discovery
        .as_mut()
        .map(|discovery| std::mem::take(&mut discovery.contexts))
        .unwrap_or_default();
    let mut session_keys = SessionFolderKeyring::default();
    open_export_folder_key_grants_into_session(&auth, &export, &mut session_keys)?;
    for mounted in &mounted_exports {
        open_export_folder_key_grants_into_session(&auth, &mounted.export, &mut session_keys)?;
    }
    let newly_readable_keys = newly_readable_session_key_count(
        &prior_tree_state,
        &export,
        &mounted_exports,
        &session_keys,
    );
    let mut preflight_remote_result = if !pending_local_changes.is_empty() {
        // Local submission is followed by a bootstrap so accepted writes can
        // be confirmed from the authoritative projection. Validate that this
        // required response is readable before any remote write or local
        // conflict bookkeeping can mutate state.
        Some(
            fetch_bootstrap_remote_sync(
                env,
                &server_url,
                &agent_state.brain_id,
                "validated bootstrap response bound before local changes".to_owned(),
            )
            .map_err(|error| sync_stage_error("preflight remote bootstrap", &root, error))?,
        )
    } else {
        None
    };
    let push_context = PushLocalWorkingTreeContext {
        env,
        server_url: &server_url,
        agent_state: &agent_state,
        export: &export,
        mounted_exports: &mounted_exports,
        session_keys: &session_keys,
    };
    let local_result = push_local_working_tree_changes(&push_context, &root, pending_local_changes)
        .map_err(|error| sync_stage_error("push local Working Tree changes", &root, error))?;
    let force_bootstrap_reason = sync_bootstrap_reason(&local_result, newly_readable_keys);
    let mut remote_result = if local_result.pushed_count > 0 {
        confirm_local_changes_from_preflight(
            env,
            &server_url,
            &agent_state.brain_id,
            preflight_remote_result.take().ok_or_else(|| {
                CliError::InvalidInput(
                    "local writes require a validated bootstrap confirmation base".to_owned(),
                )
            })?,
            force_bootstrap_reason.unwrap_or_else(|| {
                "confirmed accepted local writes through incremental sync".to_owned()
            }),
        )
        .map_err(|error| sync_stage_error("confirm local Working Tree changes", &root, error))?
    } else if local_result.conflict_count > 0 {
        let mut preflight = preflight_remote_result.take().ok_or_else(|| {
            CliError::InvalidInput(
                "local conflicts require a validated bootstrap projection".to_owned(),
            )
        })?;
        preflight.report_reason = force_bootstrap_reason;
        preflight
    } else if let Some(reason) = force_bootstrap_reason {
        fetch_bootstrap_remote_sync(env, &server_url, &agent_state.brain_id, reason)
            .map_err(|error| sync_stage_error("fetch remote bootstrap", &root, error))?
    } else {
        fetch_incremental_remote_sync(
            env,
            &root,
            &server_url,
            &agent_state.brain_id,
            prior_tree_state.sync.latest_sequence,
        )
        .map_err(|error| sync_stage_error("fetch incremental remote sync", &root, error))?
    };
    if use_cached_export {
        if remote_result.used_bootstrap {
            // The incremental path escalated, which means the cached export
            // may be stale exactly where it matters (Folder topology, access,
            // key versions). Refresh it before materializing.
            export = fetch_encrypted_export(env, &server_url, &agent_state.brain_id)
                .map_err(|error| sync_stage_error("refresh encrypted export", &root, error))?;
            open_export_folder_key_grants_into_session(&auth, &export, &mut session_keys)?;
        } else {
            // Grant records carry the same wrapped payload as export grants,
            // so post-sync access reaches this member through the diff.
            let opened_record_grants = open_sync_record_folder_key_grants(
                &auth,
                &export.brain.id,
                &remote_result.records,
                &mut session_keys,
            )?;
            let unknown_folder_grant = opened_record_grants.iter().any(|grant| {
                !export
                    .folders
                    .iter()
                    .any(|folder| folder.id == grant.folder_id)
            });
            let newly_readable_after_records = if opened_record_grants.is_empty() {
                0
            } else {
                newly_readable_session_key_count(
                    &prior_tree_state,
                    &export,
                    &mounted_exports,
                    &session_keys,
                )
            };
            if unknown_folder_grant || newly_readable_after_records > 0 {
                let pulled_records = std::mem::take(&mut remote_result.records);
                let reason = if unknown_folder_grant {
                    // A granted Folder the cached export does not know means
                    // the Folder topology changed; refresh the export so the
                    // new Folder materializes instead of losing access.
                    export = fetch_encrypted_export(env, &server_url, &agent_state.brain_id)
                        .map_err(|error| {
                            sync_stage_error("refresh encrypted export", &root, error)
                        })?;
                    open_export_folder_key_grants_into_session(&auth, &export, &mut session_keys)?;
                    "folder key grant named a Folder outside the cached export; refreshed export and fetched bootstrap"
                        .to_owned()
                } else {
                    "new folder keys were opened from sync records; fetched bootstrap for newly readable content"
                        .to_owned()
                };
                let mut result =
                    fetch_bootstrap_remote_sync(env, &server_url, &agent_state.brain_id, reason)
                        .map_err(|error| {
                            sync_stage_error("fetch remote bootstrap", &root, error)
                        })?;
                result.records = pulled_records;
                remote_result = result;
            }
        }
    }
    if !has_known_mounts {
        mounted_discovery = Some(fetch_mounted_folder_sync_contexts(
            env,
            &server_url,
            &agent_state.brain_id,
            &export,
        )?);
        mounted_exports = std::mem::take(&mut mounted_discovery.as_mut().unwrap().contexts);
        for mounted in &mounted_exports {
            open_export_folder_key_grants_into_session(&auth, &mounted.export, &mut session_keys)?;
        }
    }
    // Opportunistically deliver pending grant wraps: invitees waiting on a
    // wrapped current Folder Key get their grants from any key-holding client
    // that syncs. The freshest wrap markers win: authoritative metadata is
    // fetched every sync and carries them for admins, while the export this
    // sync already trusts (possibly just refreshed) is the fallback when
    // metadata is transiently unavailable. Never blocks; skipped quietly when
    // this Home holds no usable key for a marked Folder.
    let pending_wraps = mounted_discovery
        .as_ref()
        .and_then(|discovery| discovery.metadata.as_ref())
        .map(|metadata| metadata.pending_wraps.as_slice())
        .unwrap_or(export.pending_wraps.as_slice());
    let completed_wraps = complete_pending_grant_wraps_for_sync(
        env,
        args,
        &agent_state.brain_id,
        &auth,
        &session_keys,
        pending_wraps,
    );
    let opened_grants = session_keys.len();
    let mounted_materializations =
        fetch_mounted_folder_materializations(env, &server_url, mounted_exports)?;
    let unsupported_objects = materialize_remote_projection(MaterializeRemoteProjectionContext {
        env,
        root: &root,
        actor_npub: &auth.npub,
        metadata: mounted_discovery.as_ref().unwrap().metadata.as_ref(),
        export: &export,
        bootstrap: &remote_result.bootstrap,
        mounted_folders: &mounted_materializations,
        path_overrides: &local_result.path_overrides,
        session_keys: &session_keys,
        prior_state: Some(&prior_tree_state),
    })
    .map_err(|error| sync_stage_error("materialize remote projection", &root, error))?;
    restore_conflicted_files(&root, &local_result.conflicted_markdown)
        .map_err(|error| sync_stage_error("restore conflicted files", &root, error))?;
    let mut deleted_routes = deleted_folder_routes(&export, &remote_result.bootstrap)?;
    for mounted in &mounted_materializations {
        deleted_routes.extend(deleted_folder_routes(&mounted.export, &mounted.bootstrap)?);
    }
    remove_deleted_folder_roots(
        &root,
        &prior_tree_state,
        prior_export.as_ref(),
        &deleted_routes,
        &export.brain.id,
    )
    .map_err(|error| sync_stage_error("remove deleted Folder roots", &root, error))?;
    write_sync_evidence(&root, &export, &remote_result.bootstrap)
        .map_err(|error| sync_stage_error("write sync evidence", &root, error))?;

    let applied_tree_state = read_working_tree_state(&root)
        .map_err(|error| sync_stage_error("read applied Working Tree state", &root, error))?;
    let remote_changes = sync_record_reports(
        &remote_result.records,
        &prior_tree_state,
        &applied_tree_state,
        remote_result.report_status.as_str(),
        remote_result.report_reason.as_deref(),
    );
    let latest_sequence = remote_result.bootstrap.latest_sequence;
    let active_remote_object_count = remote_result
        .bootstrap
        .objects
        .iter()
        .filter(|object| !object.deleted)
        .count();
    let remote_record_count = if remote_changes.is_empty()
        && remote_result.used_bootstrap
        && latest_sequence > prior_tree_state.sync.latest_sequence
    {
        active_remote_object_count
    } else {
        remote_changes.len()
    };
    let status = if local_result.conflict_count > 0 {
        AgentSyncStatus::BlockedLocalConflicts
    } else if local_result.pushed_count > 0 {
        AgentSyncStatus::PushedLocalChanges
    } else if !remote_changes.is_empty()
        || newly_readable_keys > 0
        || (remote_result.used_bootstrap
            && latest_sequence > prior_tree_state.sync.latest_sequence
            && active_remote_object_count > 0)
    {
        AgentSyncStatus::AppliedRemoteRecords
    } else {
        AgentSyncStatus::CaughtUp
    };

    mutate_agent_state_at_root(&root, timestamp(env), |state, now| {
        state.sync.status = status.clone();
        state.add_activity(
            now,
            activity_kind,
            format!(
                "Sync latest sequence {latest_sequence}; openedGrants={opened_grants}; pushed={}; conflicts={}",
                local_result.pushed_count, local_result.conflict_count
            ),
        );
    })
    .map_err(|error| sync_stage_error("record sync outcome", &root, error))?;

    Ok(SyncOnceReport {
        status,
        latest_sequence,
        record_count: remote_record_count + local_result.pushed_count,
        server_url,
        conflicts: local_result
            .changes
            .iter()
            .filter(|change| change.status == "conflicted")
            .cloned()
            .collect(),
        local_changes: local_result.changes,
        remote_changes,
        unsupported_objects,
        completed_wraps,
    })
}

/// Opportunistically complete pending grant wraps: the markers tell
/// key-holding clients (Brain admin standing) which recipients still need a
/// wrapped current Folder Key, and this Finite Home wraps every marked Folder
/// Key it can open. Best-effort by contract — the markers are empty for
/// non-admins and older servers, a Folder whose key this Home cannot open is
/// skipped, and a failed completion never blocks sync.
fn complete_pending_grant_wraps_for_sync(
    env: &CliEnvironment,
    args: &[String],
    brain_id: &str,
    auth: &LocalSigner,
    session_keys: &SessionFolderKeyring,
    pending_wraps: &[CliPendingWrap],
) -> Vec<CompletedWrapReport> {
    let mut by_folder: BTreeMap<String, Vec<(String, u32)>> = BTreeMap::new();
    for wrap in pending_wraps {
        by_folder
            .entry(wrap.folder_id.clone())
            .or_default()
            .push((wrap.recipient_npub.clone(), wrap.key_version));
    }
    let mut completed = Vec::new();
    for (folder_id, entries) in by_folder {
        // The server requires the exact marked recipient set, so a Folder is
        // only submittable when every marked key version is openable here.
        let mut grants = Vec::with_capacity(entries.len());
        let mut openable = true;
        for (recipient, key_version) in &entries {
            let Some(folder_key) = session_keys.get(brain_id, &folder_id, *key_version) else {
                openable = false;
                break;
            };
            match folder_key_grant_request(
                auth,
                brain_id,
                &folder_id,
                *key_version,
                recipient,
                folder_key,
                env,
            ) {
                Ok(grant) => grants.push(grant),
                Err(_) => {
                    openable = false;
                    break;
                }
            }
        }
        if !openable || grants.is_empty() {
            continue;
        }
        let route = format!("/v1/admin/brains/{brain_id}/folders/{folder_id}/pending-wraps");
        if signed_json_request(
            env,
            args,
            "POST",
            &route,
            Some(serde_json::json!({ "grants": grants })),
        )
        .is_ok()
        {
            for (recipient, _) in &entries {
                completed.push(CompletedWrapReport {
                    folder_id: folder_id.clone(),
                    recipient_npub: recipient.clone(),
                });
            }
        }
    }
    completed
}

fn sync_stage_error(stage: &str, root: &Path, error: CliError) -> CliError {
    CliError::SyncStage {
        stage: stage.to_owned(),
        root: root.to_path_buf(),
        source: Box::new(error),
    }
}

pub(crate) fn open_brain_session_folder_keys(
    env: &CliEnvironment,
    args: &[String],
    brain_id: &str,
) -> Result<SessionFolderKeyring, CliError> {
    let path = format!("/v1/brains/{brain_id}/export");
    let response = signed_json_request_with_response_limit(
        env,
        args,
        "GET",
        &path,
        None,
        encrypted_export_response_limit_bytes(),
    )?;
    let export: CliEncryptedBrainExport = serde_json::from_value(response)?;
    if export.brain.id != brain_id {
        return Err(CliError::InvalidInput(format!(
            "encrypted export returned brain {} while opening {brain_id}",
            export.brain.id
        )));
    }
    let auth = load_signer(env)?;
    let mut keyring = SessionFolderKeyring::default();
    open_export_folder_key_grants_into_session(&auth, &export, &mut keyring)?;
    Ok(keyring)
}

/// Open keys for the high-level collaboration operation. A damaged or
/// undecryptable grant addressed to this signer is treated as an unavailable
/// source key while other grants continue opening; the server receipt then
/// reports the affected Folder as partial and names its current holders.
pub(crate) fn open_brain_session_folder_keys_for_collaboration(
    env: &CliEnvironment,
    args: &[String],
    brain_id: &str,
) -> Result<SessionFolderKeyring, CliError> {
    let path = format!("/v1/brains/{brain_id}/export");
    let response = signed_json_request_with_response_limit(
        env,
        args,
        "GET",
        &path,
        None,
        encrypted_export_response_limit_bytes(),
    )?;
    let export: CliEncryptedBrainExport = serde_json::from_value(response)?;
    if export.brain.id != brain_id {
        return Err(CliError::InvalidInput(format!(
            "encrypted export returned brain {} while opening {brain_id}",
            export.brain.id
        )));
    }
    let auth = load_signer(env)?;
    let mut keyring = SessionFolderKeyring::default();
    open_export_folder_key_grants_into_session_tolerant(&auth, &export, &mut keyring)?;
    Ok(keyring)
}

pub(crate) fn prepare_folder_access_removal(
    env: &CliEnvironment,
    args: &[String],
    metadata: &BrainMetadataView,
    brain_id: &str,
    folder_id: &str,
    target_npub: &str,
) -> Result<serde_json::Value, CliError> {
    prepare_folder_access_removals(
        env,
        args,
        metadata,
        brain_id,
        folder_id,
        &BTreeSet::from([target_npub.to_owned()]),
    )
}

pub(crate) fn prepare_folder_access_removals(
    env: &CliEnvironment,
    args: &[String],
    metadata: &BrainMetadataView,
    brain_id: &str,
    folder_id: &str,
    target_npubs: &BTreeSet<String>,
) -> Result<serde_json::Value, CliError> {
    if target_npubs.is_empty() {
        return Err(CliError::InvalidInput(
            "at least one Folder access target is required".to_owned(),
        ));
    }
    let folder = metadata
        .folders
        .iter()
        .find(|folder| folder.id == folder_id)
        .ok_or_else(|| CliError::NotFound(format!("folder {folder_id}")))?;
    let server_url = server_url_for_command(env, args)?;
    let export = fetch_encrypted_export(env, &server_url, brain_id)?;
    let export_folder = export
        .folders
        .iter()
        .find(|folder| folder.id == folder_id)
        .ok_or_else(|| CliError::NotFound(format!("folder {folder_id} in encrypted export")))?;
    if !export_folder.accessible {
        return Err(CliError::InvalidInput(format!(
            "cannot prepare Folder access revocation: current Folder {folder_id} is not readable"
        )));
    }
    let auth = load_signer(env)?;
    let mut keyring = SessionFolderKeyring::default();
    open_export_folder_key_grants_into_session(&auth, &export, &mut keyring)?;
    let current_key = keyring
        .get(brain_id, folder_id, folder.current_key_version)
        .ok_or_else(|| CliError::GrantOpening {
            brain_id: brain_id.to_owned(),
            folder_id: folder_id.to_owned(),
            key_version: folder.current_key_version,
            reason: "the current Folder Key is required before revocation can be prepared"
                .to_owned(),
        })?;
    let new_key_version = folder
        .current_key_version
        .checked_add(1)
        .ok_or_else(|| CliError::InvalidInput("Folder Key version overflow".to_owned()))?;
    let new_key = FolderKey::generate();
    let remaining_access_user_ids = folder
        .access_user_ids
        .iter()
        .filter(|recipient| !target_npubs.contains(*recipient))
        .cloned()
        .collect::<Vec<_>>();
    let remaining_recipients =
        folder_required_recipients(metadata, &folder.access, &remaining_access_user_ids)?;
    if remaining_recipients.is_empty() {
        return Err(CliError::InvalidInput(
            "cannot revoke Folder access without at least one remaining authorized identity"
                .to_owned(),
        ));
    }
    let grants = remaining_recipients
        .iter()
        .map(|recipient| {
            folder_key_grant_request(
                &auth,
                brain_id,
                folder_id,
                new_key_version,
                recipient,
                &new_key,
                env,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let signing_keys = auth.keys.clone();
    let brain_id_value =
        BrainId::new(brain_id).map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let folder_id_value =
        FolderId::new(folder_id).map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let mut reencrypted_records = Vec::new();
    for object in export
        .objects
        .iter()
        .filter(|object| object.folder_id == folder_id && !object.deleted)
    {
        let payload_json = object.payload_json.as_deref().ok_or_else(|| {
            CliError::InvalidInput(format!(
                "cannot prepare revocation: live object {} is opaque",
                object.object_id
            ))
        })?;
        let envelope_json = decode_sync_payload(payload_json).ciphertext_or_raw(payload_json);
        let envelope = EncryptedFolderObjectEnvelope::from_json(&envelope_json)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let object_id = ObjectId::new(object.object_id.clone())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let old_aad = FolderObjectAad {
            brain_id: brain_id_value.clone(),
            folder_id: folder_id_value.clone(),
            object_id: object_id.clone(),
            key_version: folder.current_key_version,
        };
        let plaintext = open_folder_object(current_key, &old_aad, &envelope)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let new_aad = FolderObjectAad {
            brain_id: brain_id_value.clone(),
            folder_id: folder_id_value.clone(),
            object_id: object_id.clone(),
            key_version: new_key_version,
        };
        let new_envelope = encrypt_folder_object(&new_key, &new_aad, &plaintext)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let new_envelope_json = new_envelope.canonical_json();
        let revision_event = signed_revision_event(
            &signing_keys,
            RevisionEventInput {
                actor_npub: &auth.npub,
                brain_id,
                folder_id: &folder_id_value,
                object_id: &object_id,
                operation: FolderObjectOperation::Update,
                base_revision: Some(object.revision),
                key_version: new_key_version,
                envelope_json: new_envelope_json.clone(),
            },
        )?;
        reencrypted_records.push(serde_json::json!({
            "objectId": object.object_id,
            "baseRevision": object.revision,
            "keyVersion": new_key_version,
            "cipher": CIPHER_AES_256_GCM,
            "ciphertext": new_envelope_json,
            "revisionEvent": revision_event,
        }));
    }
    let access_change_event = admin_access_change_event(
        env,
        brain_id,
        AdminAccessAction::RemoveFolderAccess,
        Some(folder_id),
        target_npubs.iter().next().map(String::as_str),
        Some(new_key_version),
    )?;
    Ok(serde_json::json!({
        "newKeyVersion": new_key_version,
        "grants": grants,
        "reencryptedRecords": reencrypted_records,
        "accessChangeEvent": access_change_event,
    }))
}

fn newly_readable_session_key_count(
    prior_tree_state: &finite_brain_core::portability::BrainWorkingTreeStateManifest,
    export: &CliEncryptedBrainExport,
    mounted_exports: &[MountedFolderSyncContext],
    session_keys: &SessionFolderKeyring,
) -> usize {
    let primary = export.folders.iter().filter(|folder| {
        session_keys.contains(&export.brain.id, &folder.id, folder.current_key_version)
            && !prior_tree_state.folder_roots.iter().any(|root| {
                root.source_brain_id.is_none() && root.folder_id == folder.id && root.can_read
            })
    });
    let mounted = mounted_exports.iter().filter(|mounted| {
        mounted.source_folder().is_some_and(|folder| {
            session_keys.contains(
                &mounted.export.brain.id,
                &folder.id,
                folder.current_key_version,
            ) && !prior_tree_state.folder_roots.iter().any(|root| {
                root.source_brain_id.as_deref() == Some(mounted.export.brain.id.as_str())
                    && root.folder_id == folder.id
                    && root.can_read
            })
        })
    });
    primary.count() + mounted.count()
}

#[cfg(test)]
pub(crate) fn pending_working_tree_change_count(root: &Path) -> Result<usize, CliError> {
    Ok(pending_working_tree_change_paths(root)?.len())
}

pub(crate) fn pending_working_tree_change_paths(root: &Path) -> Result<Vec<String>, CliError> {
    let tree_state = read_working_tree_state(root)?;
    let mut paths = Vec::new();
    for change in scan_working_tree_changes(root, &tree_state)? {
        match change {
            WorkingTreeChange::Upsert { path, .. }
            | WorkingTreeChange::UpsertAsset { path, .. }
            | WorkingTreeChange::Delete { path } => paths.push(path.to_string()),
            WorkingTreeChange::Rename { from_path, to_path } => {
                paths.push(from_path.to_string());
                paths.push(to_path.to_string());
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn fetch_encrypted_export(
    env: &CliEnvironment,
    server_url: &str,
    brain_id: &str,
) -> Result<CliEncryptedBrainExport, CliError> {
    let path = format!("/v1/brains/{brain_id}/export");
    let response = signed_json_request_to_server_with_response_limit(
        env,
        server_url,
        "GET",
        &path,
        None,
        encrypted_export_response_limit_bytes(),
    )?;
    serde_json::from_value(response).map_err(CliError::from)
}

fn fetch_sync_bootstrap(
    env: &CliEnvironment,
    server_url: &str,
    brain_id: &str,
) -> Result<CliSyncBootstrap, CliError> {
    let path = format!("/v1/brains/{brain_id}/sync/bootstrap");
    let response = signed_json_request_to_server_with_response_limit(
        env,
        server_url,
        "GET",
        &path,
        None,
        SYNC_BOOTSTRAP_RESPONSE_LIMIT_BYTES,
    )?;
    serde_json::from_value(response).map_err(CliError::from)
}

fn fetch_bootstrap_remote_sync(
    env: &CliEnvironment,
    server_url: &str,
    brain_id: &str,
    reason: String,
) -> Result<RemoteSyncResult, CliError> {
    Ok(RemoteSyncResult {
        bootstrap: fetch_sync_bootstrap(env, server_url, brain_id)?,
        records: Vec::new(),
        report_status: "rebootstrapped".to_owned(),
        report_reason: Some(reason),
        used_bootstrap: true,
    })
}

fn fetch_incremental_remote_sync(
    env: &CliEnvironment,
    root: &Path,
    server_url: &str,
    brain_id: &str,
    after_sequence: u64,
) -> Result<RemoteSyncResult, CliError> {
    let pull = match fetch_all_sync_records(env, server_url, brain_id, after_sequence) {
        Ok(pull) => pull,
        Err(error) if is_rebootstrap_required_error(&error) => {
            return fetch_bootstrap_remote_sync(
                env,
                server_url,
                brain_id,
                format!("incremental cursor {after_sequence} expired; fetched bootstrap"),
            );
        }
        Err(error) if is_sync_records_route_unavailable(&error) => {
            return fetch_bootstrap_remote_sync(
                env,
                server_url,
                brain_id,
                "incremental sync records route unavailable; fetched bootstrap".to_owned(),
            );
        }
        Err(error) => return Err(error),
    };
    let records = pull.records;
    match apply_incremental_records(root, after_sequence, pull.latest_sequence, &records) {
        Ok(bootstrap) => Ok(RemoteSyncResult {
            bootstrap,
            records,
            report_status: "applied".to_owned(),
            report_reason: None,
            used_bootstrap: false,
        }),
        Err(reason) => {
            let mut result =
                fetch_bootstrap_remote_sync(env, server_url, brain_id, reason.clone())?;
            result.records = records;
            result.report_reason = Some(reason);
            Ok(result)
        }
    }
}

fn confirm_local_changes_from_preflight(
    env: &CliEnvironment,
    server_url: &str,
    brain_id: &str,
    preflight: RemoteSyncResult,
    reason: String,
) -> Result<RemoteSyncResult, CliError> {
    let after_sequence = preflight.bootstrap.latest_sequence;
    let pull = match fetch_all_sync_records(env, server_url, brain_id, after_sequence) {
        Ok(pull) => pull,
        Err(error)
            if is_rebootstrap_required_error(&error)
                || is_sync_records_route_unavailable(&error) =>
        {
            return fetch_bootstrap_remote_sync(
                env,
                server_url,
                brain_id,
                format!(
                    "{reason}; incremental confirmation unavailable after sequence {after_sequence}, so the bounded bootstrap compatibility path was used"
                ),
            );
        }
        Err(error) => return Err(error),
    };
    let records = pull.records;
    let bootstrap = apply_incremental_records_to_bootstrap(
        preflight.bootstrap,
        after_sequence,
        pull.latest_sequence,
        &records,
    )
    .map_err(CliError::InvalidInput)?;
    Ok(RemoteSyncResult {
        bootstrap,
        records,
        report_status: "applied".to_owned(),
        report_reason: Some(reason),
        used_bootstrap: true,
    })
}

fn fetch_all_sync_records(
    env: &CliEnvironment,
    server_url: &str,
    brain_id: &str,
    after_sequence: u64,
) -> Result<IncrementalSyncPull, CliError> {
    let mut after = after_sequence;
    let mut records = Vec::new();
    loop {
        let page = fetch_sync_records_page(env, server_url, brain_id, after)?;
        if page.brain_id != brain_id {
            return Err(CliError::InvalidInput(format!(
                "sync records response brain {} did not match requested brain {brain_id}",
                page.brain_id
            )));
        }
        let latest_sequence = page.latest_sequence;
        records.extend(page.records);
        if !page.has_more {
            return Ok(IncrementalSyncPull {
                latest_sequence,
                records,
            });
        }
        if page.next_sequence <= after {
            return Err(CliError::InvalidInput(format!(
                "sync records cursor did not advance after sequence {after}"
            )));
        }
        after = page.next_sequence;
    }
}

fn fetch_sync_records_page(
    env: &CliEnvironment,
    server_url: &str,
    brain_id: &str,
    after_sequence: u64,
) -> Result<CliSyncPull, CliError> {
    let path = format!(
        "/v1/brains/{brain_id}/sync/records?after={after_sequence}&limit={SYNC_RECORDS_PAGE_LIMIT}"
    );
    // Record pages carry full ciphertext payloads, so a catch-up page scales
    // with batch activity, not with the generic JSON cap: a busy window on a
    // large Brain would otherwise brick every sync behind the cursor.
    let response = signed_json_request_to_server_with_response_limit(
        env,
        server_url,
        "GET",
        &path,
        None,
        SYNC_BOOTSTRAP_RESPONSE_LIMIT_BYTES,
    )?;
    serde_json::from_value(response).map_err(CliError::from)
}

fn sync_bootstrap_reason(local_result: &LocalSyncResult, opened_grants: usize) -> Option<String> {
    if local_result.pushed_count > 0 {
        Some(
            "local writes were accepted; fetched bootstrap to confirm server projection".to_owned(),
        )
    } else if local_result.conflict_count > 0 {
        Some("local conflicts were recorded; fetched bootstrap before restoring edits".to_owned())
    } else if opened_grants > 0 {
        Some("new folder keys were opened; fetched bootstrap for newly readable content".to_owned())
    } else {
        None
    }
}

fn is_rebootstrap_required_error(error: &CliError) -> bool {
    matches!(error, CliError::Http(message) if message.contains("rebootstrap required"))
        || matches!(error, CliError::HttpStatus { status: 410, body } if body.contains("rebootstrap required"))
}

fn is_sync_records_route_unavailable(error: &CliError) -> bool {
    matches!(error, CliError::HttpStatus { status: 404, .. })
}

fn apply_incremental_records(
    root: &Path,
    after_sequence: u64,
    latest_sequence: u64,
    records: &[CliSyncRecord],
) -> Result<CliSyncBootstrap, String> {
    let base = incremental_base_bootstrap(root, after_sequence)?;
    apply_incremental_records_to_bootstrap(base, after_sequence, latest_sequence, records)
}

fn apply_incremental_records_to_bootstrap(
    base: CliSyncBootstrap,
    after_sequence: u64,
    latest_sequence: u64,
    records: &[CliSyncRecord],
) -> Result<CliSyncBootstrap, String> {
    if base.latest_sequence != after_sequence {
        return Err(format!(
            "bootstrap sequence {} does not match cursor {after_sequence}",
            base.latest_sequence
        ));
    }
    if latest_sequence < after_sequence {
        return Err(format!(
            "sync records latest sequence {latest_sequence} is older than cursor {after_sequence}"
        ));
    }
    let mut control_records = base.control_records;
    let mut objects = base
        .objects
        .into_iter()
        .map(|object| ((object.folder_id.clone(), object.object_id.clone()), object))
        .collect::<BTreeMap<_, _>>();

    for record in records {
        if record.sequence <= after_sequence {
            return Err(format!(
                "sync record {} did not advance cursor {after_sequence}",
                record.sequence
            ));
        }
        let payload = decode_sync_payload(&record.payload_json);
        match record.record_type.as_str() {
            "folder_object_revision" => {
                let folder_id = record_folder_id(record)?;
                let object_id = record_object_id(record)?;
                objects.insert(
                    (folder_id.clone(), object_id.clone()),
                    CliSyncObject {
                        folder_id,
                        object_id,
                        revision: record_revision(record)?,
                        ciphertext: payload.ciphertext_or_raw(&record.payload_json),
                        deleted: false,
                    },
                );
            }
            "folder_object_tombstone" => {
                let folder_id = record_folder_id(record)?;
                let object_id = record_object_id(record)?;
                objects.insert(
                    (folder_id.clone(), object_id.clone()),
                    CliSyncObject {
                        folder_id,
                        object_id,
                        revision: record_revision(record)?,
                        ciphertext: record.payload_json.clone(),
                        deleted: true,
                    },
                );
            }
            "brain_admin_access_change" if payload.is_folder_subtree_tombstone() => {
                let deleted_folder_ids = folder_subtree_tombstone_ids(record, &payload)?;
                objects.retain(|(folder_id, _), _| !deleted_folder_ids.contains(folder_id));
                control_records.push(record.clone());
            }
            // Grant records do not change the object projection, but they are
            // part of the control-record surface the server bootstrap serves;
            // keeping them lets the routine diff path observe post-sync grants
            // without forcing a rebootstrap.
            "folder_key_grant" => {
                control_records.push(record.clone());
            }
            other => {
                return Err(format!(
                    "sync record {} type {other} requires bootstrap",
                    record.sequence
                ));
            }
        }
    }

    Ok(CliSyncBootstrap {
        latest_sequence,
        objects: objects.into_values().collect(),
        control_records,
    })
}

fn incremental_base_bootstrap(
    root: &Path,
    after_sequence: u64,
) -> Result<CliSyncBootstrap, String> {
    match read_cached_sync_bootstrap(root) {
        Ok(Some(cached)) if cached.latest_sequence == after_sequence => Ok(cached),
        Ok(Some(cached)) => Err(format!(
            "cached bootstrap sequence {} does not match cursor {after_sequence}",
            cached.latest_sequence
        )),
        Ok(None) if after_sequence == 0 => Ok(CliSyncBootstrap {
            latest_sequence: 0,
            objects: Vec::new(),
            control_records: Vec::new(),
        }),
        Ok(None) => Err(format!(
            "cached bootstrap missing for incremental cursor {after_sequence}"
        )),
        Err(error) => Err(format!("cached bootstrap unreadable: {error}")),
    }
}

fn read_cached_sync_bootstrap(root: &Path) -> Result<Option<CliSyncBootstrap>, CliError> {
    let path = root.join(".finitebrain/encrypted-sync/bootstrap.json");
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(CliError::from)
}

fn record_folder_id(record: &CliSyncRecord) -> Result<String, String> {
    record
        .folder_id
        .clone()
        .ok_or_else(|| format!("sync record {} is missing folderId", record.sequence))
}

fn record_object_id(record: &CliSyncRecord) -> Result<String, String> {
    record
        .object_id
        .clone()
        .ok_or_else(|| format!("sync record {} is missing objectId", record.sequence))
}

fn record_revision(record: &CliSyncRecord) -> Result<u64, String> {
    record
        .revision
        .ok_or_else(|| format!("sync record {} is missing revision", record.sequence))
}

fn sync_record_reports(
    records: &[CliSyncRecord],
    prior_state: &finite_brain_core::portability::BrainWorkingTreeStateManifest,
    applied_state: &finite_brain_core::portability::BrainWorkingTreeStateManifest,
    status: &str,
    reason: Option<&str>,
) -> Vec<SyncChangeReport> {
    records
        .iter()
        .map(|record| {
            let payload = decode_sync_payload(&record.payload_json);
            SyncChangeReport {
                status: status.to_owned(),
                action: sync_record_action(record, &payload),
                actor_npub: Some(record.actor_npub.clone()),
                sequence: Some(record.sequence),
                path: sync_record_path(record, prior_state, applied_state),
                from_path: None,
                folder_id: record.folder_id.clone(),
                source_brain_id: None,
                object_id: record.object_id.clone(),
                route: "sync-record".to_owned(),
                reason: reason.map(ToOwned::to_owned),
            }
        })
        .collect()
}

fn sync_record_action(record: &CliSyncRecord, payload: &DecodedSyncPayload) -> String {
    match record.record_type.as_str() {
        "folder_object_revision" => {
            if payload.base_revision_is_none() {
                "create".to_owned()
            } else {
                "update".to_owned()
            }
        }
        "folder_object_tombstone" => "delete".to_owned(),
        "brain_admin_access_change" if payload.is_folder_subtree_tombstone() => {
            "delete-folder-subtree".to_owned()
        }
        other => other.to_owned(),
    }
}

fn folder_subtree_tombstone_ids(
    record: &CliSyncRecord,
    payload: &DecodedSyncPayload,
) -> Result<BTreeSet<String>, String> {
    let DecodedSyncPayload::AdminChange {
        subtree_tombstone_ids,
    } = payload
    else {
        return Err(format!(
            "sync record {} deletion payload is invalid",
            record.sequence
        ));
    };
    let folder_ids = subtree_tombstone_ids.as_ref().ok_or_else(|| {
        format!(
            "sync record {} deletion payload is missing folderIds",
            record.sequence
        )
    })?;
    if folder_ids.is_empty() {
        return Err(format!(
            "sync record {} deletion payload has no folderIds",
            record.sequence
        ));
    }
    folder_ids
        .iter()
        .map(|folder_id| {
            FolderId::new(folder_id.clone())
                .map(|folder_id| folder_id.to_string())
                .map_err(|error| error.to_string())
        })
        .collect()
}

fn sync_record_path(
    record: &CliSyncRecord,
    prior_state: &finite_brain_core::portability::BrainWorkingTreeStateManifest,
    applied_state: &finite_brain_core::portability::BrainWorkingTreeStateManifest,
) -> Option<String> {
    let folder_id = record.folder_id.as_deref()?;
    let object_id = record.object_id.as_deref()?;
    working_tree_path_for_record(applied_state, folder_id, object_id)
        .or_else(|| working_tree_path_for_record(prior_state, folder_id, object_id))
}

fn working_tree_path_for_record(
    state: &finite_brain_core::portability::BrainWorkingTreeStateManifest,
    folder_id: &str,
    object_id: &str,
) -> Option<String> {
    let object = state.objects.iter().find(|object| {
        object.source_brain_id.is_none()
            && object.folder_id == folder_id
            && object.object_id == object_id
    })?;
    let folder = state
        .folder_roots
        .iter()
        .find(|folder| folder.source_brain_id.is_none() && folder.folder_id == folder_id)?;
    Some(format!("{}/{}", folder.path, object.path))
}

fn fetch_brain_metadata_for_sync(
    env: &CliEnvironment,
    server_url: &str,
    brain_id: &str,
) -> Result<CliBrainMetadata, CliError> {
    let path = format!("/v1/brains/{brain_id}/metadata");
    let response = signed_json_request_to_server(env, server_url, "GET", &path, None)?;
    serde_json::from_value(response).map_err(CliError::from)
}

fn fetch_mounted_folder_sync_contexts(
    env: &CliEnvironment,
    server_url: &str,
    brain_id: &str,
    export: &CliEncryptedBrainExport,
) -> Result<MountedFolderSyncDiscovery, CliError> {
    // Role-bearing Working Tree context must come from authoritative metadata.
    // A transient metadata failure may not invent a role: preserve the prior
    // generated root instructions by omitting their replacement in this pass.
    let metadata = match fetch_brain_metadata_for_sync(env, server_url, brain_id) {
        Ok(metadata) => Some(metadata),
        Err(CliError::Http(_)) | Err(CliError::HttpStatus { .. }) => None,
        Err(error) => return Err(error),
    };
    let mut used_paths = export
        .folders
        .iter()
        .map(|folder| folder.path.clone())
        .collect::<BTreeSet<_>>();
    let mut contexts = Vec::new();
    for mount in metadata
        .as_ref()
        .into_iter()
        .flat_map(|metadata| &metadata.mounted_folders)
        .filter(|mount| mount.state == "available")
        .cloned()
    {
        let source_export = fetch_encrypted_export(env, server_url, &mount.source_brain_id)?;
        let display_path = mounted_folder_display_path(&mut used_paths, &mount, &source_export)?;
        contexts.push(MountedFolderSyncContext {
            mount,
            export: source_export,
            display_path,
        });
    }
    Ok(MountedFolderSyncDiscovery { metadata, contexts })
}

struct MountedFolderSyncDiscovery {
    metadata: Option<CliBrainMetadata>,
    contexts: Vec<MountedFolderSyncContext>,
}

fn fetch_mounted_folder_materializations(
    env: &CliEnvironment,
    server_url: &str,
    mounted_exports: Vec<MountedFolderSyncContext>,
) -> Result<Vec<MountedFolderMaterializeContext>, CliError> {
    mounted_exports
        .into_iter()
        .map(|mounted| {
            let bootstrap = fetch_sync_bootstrap(env, server_url, &mounted.export.brain.id)?;
            Ok(MountedFolderMaterializeContext {
                mount: mounted.mount,
                export: mounted.export,
                display_path: mounted.display_path,
                bootstrap,
            })
        })
        .collect()
}

fn mounted_folder_display_path(
    used_paths: &mut BTreeSet<String>,
    mount: &CliMountedFolder,
    source_export: &CliEncryptedBrainExport,
) -> Result<String, CliError> {
    let source_folder = source_export
        .folders
        .iter()
        .find(|folder| folder.id == mount.source_folder_id)
        .ok_or_else(|| CliError::NotFound(format!("folder {}", mount.source_folder_id)))?;
    let candidates = [
        source_folder.path.clone(),
        mount.display_name.clone(),
        format!("{}/{}", mount.source_brain_id, source_folder.path),
        format!("{}/{}", mount.source_brain_id, mount.source_folder_id),
    ];
    for candidate in candidates {
        if SafeRelativePath::new("mounted_folder_path", candidate.clone()).is_ok()
            && !used_paths.contains(&candidate)
        {
            used_paths.insert(candidate.clone());
            return Ok(candidate);
        }
    }
    Err(CliError::InvalidInput(format!(
        "mounted folder path collides for {}",
        mount.mount_id
    )))
}

fn write_sync_evidence(
    root: &Path,
    export: &CliEncryptedBrainExport,
    bootstrap: &CliSyncBootstrap,
) -> Result<(), CliError> {
    let sync_dir = root.join(".finitebrain/encrypted-sync");
    write_json_file(&sync_dir.join("export.json"), export)?;
    write_json_file(&sync_dir.join("bootstrap.json"), bootstrap)?;
    Ok(())
}

fn restore_conflicted_files(
    root: &Path,
    conflicted_markdown: &BTreeMap<String, String>,
) -> Result<(), CliError> {
    for (relative_path, markdown) in conflicted_markdown {
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, markdown)?;
    }
    Ok(())
}

fn open_export_folder_key_grants_into_session(
    auth: &crate::LocalSigner,
    export: &CliEncryptedBrainExport,
    session_keys: &mut SessionFolderKeyring,
) -> Result<usize, CliError> {
    let opened = opened_export_folder_key_grants(auth, export)?;
    let mut opened_count = 0;
    for grant in opened {
        let folder_key =
            FolderKey::from_base64(&grant.folder_key).map_err(|_| CliError::GrantOpening {
                brain_id: grant.brain_id.clone(),
                folder_id: grant.folder_id.clone(),
                key_version: grant.key_version,
                reason: "opened grant did not contain a valid Folder Key".to_owned(),
            })?;
        if session_keys.insert(
            grant.brain_id,
            grant.folder_id,
            grant.key_version,
            folder_key,
        ) {
            opened_count += 1;
        }
    }
    Ok(opened_count)
}

fn open_export_folder_key_grants_into_session_tolerant(
    auth: &crate::LocalSigner,
    export: &CliEncryptedBrainExport,
    session_keys: &mut SessionFolderKeyring,
) -> Result<usize, CliError> {
    let opened = opened_export_folder_key_grants_tolerant(auth, export);
    let mut opened_count = 0;
    for grant in opened {
        let folder_key =
            FolderKey::from_base64(&grant.folder_key).map_err(|_| CliError::GrantOpening {
                brain_id: grant.brain_id.clone(),
                folder_id: grant.folder_id.clone(),
                key_version: grant.key_version,
                reason: "opened grant did not contain a valid Folder Key".to_owned(),
            })?;
        if session_keys.insert(
            grant.brain_id,
            grant.folder_id,
            grant.key_version,
            folder_key,
        ) {
            opened_count += 1;
        }
    }
    Ok(opened_count)
}

fn opened_export_folder_key_grants(
    auth: &crate::LocalSigner,
    export: &CliEncryptedBrainExport,
) -> Result<Vec<CliFolderKeyGrantPlaintext>, CliError> {
    let mut opened = Vec::new();
    for grant in &export.key_grants {
        if let Some(plaintext) = open_folder_key_grant_plaintext(auth, &export.brain.id, grant)? {
            opened.push(plaintext);
        }
    }

    Ok(opened)
}

/// Open one wrapped Folder Key grant addressed to this signer. Returns `None`
/// for grants addressed to other recipients; a damaged or undecryptable grant
/// addressed to this signer fails closed.
fn open_folder_key_grant_plaintext(
    auth: &crate::LocalSigner,
    brain_id: &str,
    grant: &CliFolderKeyGrant,
) -> Result<Option<CliFolderKeyGrantPlaintext>, CliError> {
    if grant.recipient_npub != auth.npub {
        return Ok(None);
    }
    let keys = auth.keys.clone();
    let recipient = NostrPublicKey::parse(&auth.npub)
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    let validation = GiftWrapValidation::new(recipient);
    let unusable_grant = || {
        CliError::GrantOpening {
        brain_id: brain_id.to_owned(),
        folder_id: grant.folder_id.clone(),
        key_version: grant.key_version,
        reason: "the local signer could not validate and decrypt it; verify this Member Identity has a valid current grant"
            .to_owned(),
    }
    };
    let event = Event::from_json(grant.wrapped_event_json.clone()).map_err(|_| unusable_grant())?;
    let opened_wrap = open_gift_wrap(&keys, &event, &validation).map_err(|_| unusable_grant())?;
    let plaintext = serde_json::from_str::<CliFolderKeyGrantPlaintext>(&opened_wrap.rumor.content)
        .map_err(|_| unusable_grant())?;
    if plaintext.version != "finite-folder-key-grant-v1"
        || plaintext.brain_id != brain_id
        || plaintext.folder_id != grant.folder_id
        || plaintext.key_version != grant.key_version
        || plaintext.issuer_npub != grant.issuer_npub
        || plaintext.recipient_npub != auth.npub
    {
        return Err(unusable_grant());
    }
    FolderKey::from_base64(&plaintext.folder_key).map_err(|_| unusable_grant())?;
    Ok(Some(plaintext))
}

/// Open Folder Key grants delivered as incremental sync records into the
/// session keyring. Grant records carry the same wrapped payload as export
/// grants, so a post-sync grant reaches this member through the routine diff
/// without a full export. Returns the plaintexts of newly inserted grants
/// addressed to this signer.
fn open_sync_record_folder_key_grants(
    auth: &crate::LocalSigner,
    brain_id: &str,
    records: &[CliSyncRecord],
    session_keys: &mut SessionFolderKeyring,
) -> Result<Vec<CliFolderKeyGrantPlaintext>, CliError> {
    let mut opened = Vec::new();
    for record in records {
        if record.record_type != "folder_key_grant" {
            continue;
        }
        let grant: CliFolderKeyGrant =
            serde_json::from_str(&record.payload_json).map_err(|_| {
                CliError::InvalidInput(format!(
                    "folder key grant sync record {} payload did not parse",
                    record.sequence
                ))
            })?;
        let Some(plaintext) = open_folder_key_grant_plaintext(auth, brain_id, &grant)? else {
            continue;
        };
        let folder_key =
            FolderKey::from_base64(&plaintext.folder_key).map_err(|_| CliError::GrantOpening {
                brain_id: plaintext.brain_id.clone(),
                folder_id: plaintext.folder_id.clone(),
                key_version: plaintext.key_version,
                reason: "opened grant did not contain a valid Folder Key".to_owned(),
            })?;
        if session_keys.insert(
            brain_id,
            plaintext.folder_id.clone(),
            plaintext.key_version,
            folder_key,
        ) {
            opened.push(plaintext);
        }
    }
    Ok(opened)
}

pub(crate) fn open_offered_folder_key(
    env: &CliEnvironment,
    brain_id: &str,
    folder_id: &str,
    key_version: u32,
    issuer_npub: &str,
    wrapped_event_json: &str,
) -> Result<FolderKey, CliError> {
    let auth = load_signer(env)?;
    let export = CliEncryptedBrainExport {
        brain: CliExportBrain {
            id: brain_id.to_owned(),
            kind: String::new(),
            name: String::new(),
            owner_user_id: None,
        },
        folders: Vec::new(),
        objects: Vec::new(),
        key_grants: vec![CliFolderKeyGrant {
            folder_id: folder_id.to_owned(),
            key_version,
            issuer_npub: issuer_npub.to_owned(),
            recipient_npub: auth.npub.clone(),
            wrapped_event_json: wrapped_event_json.to_owned(),
        }],
        access_state: CliExportAccessState {
            members: Vec::new(),
            admins: Vec::new(),
        },

        pending_wraps: Vec::new(),
    };
    let plaintext = opened_export_folder_key_grants(&auth, &export)?
        .into_iter()
        .next()
        .ok_or_else(|| CliError::GrantOpening {
            brain_id: brain_id.to_owned(),
            folder_id: folder_id.to_owned(),
            key_version,
            reason: "the Mount Offer did not include a usable grant for this controller".to_owned(),
        })?;
    FolderKey::from_base64(&plaintext.folder_key).map_err(|_| CliError::GrantOpening {
        brain_id: brain_id.to_owned(),
        folder_id: folder_id.to_owned(),
        key_version,
        reason: "the Mount Offer grant did not contain a valid Folder Key".to_owned(),
    })
}

pub(crate) fn opened_export_folder_key_grants_tolerant(
    auth: &crate::LocalSigner,
    export: &CliEncryptedBrainExport,
) -> Vec<CliFolderKeyGrantPlaintext> {
    let keys = auth.keys.clone();
    let recipient = match NostrPublicKey::parse(&auth.npub) {
        Ok(recipient) => recipient,
        Err(_) => return Vec::new(),
    };
    let validation = GiftWrapValidation::new(recipient);
    export
        .key_grants
        .iter()
        .filter(|grant| grant.recipient_npub == auth.npub)
        .filter_map(|grant| {
            let event = Event::from_json(grant.wrapped_event_json.clone()).ok()?;
            let opened_wrap = open_gift_wrap(&keys, &event, &validation).ok()?;
            let plaintext =
                serde_json::from_str::<CliFolderKeyGrantPlaintext>(&opened_wrap.rumor.content)
                    .ok()?;
            if plaintext.version != "finite-folder-key-grant-v1"
                || plaintext.brain_id != export.brain.id
                || plaintext.folder_id != grant.folder_id
                || plaintext.key_version != grant.key_version
                || plaintext.issuer_npub != grant.issuer_npub
                || plaintext.recipient_npub != auth.npub
            {
                return None;
            }
            FolderKey::from_base64(&plaintext.folder_key).ok()?;
            Some(plaintext)
        })
        .collect()
}

struct PushLocalWorkingTreeContext<'a> {
    env: &'a CliEnvironment,
    server_url: &'a str,
    agent_state: &'a AgentState,
    export: &'a CliEncryptedBrainExport,
    mounted_exports: &'a [MountedFolderSyncContext],
    session_keys: &'a SessionFolderKeyring,
}

fn push_local_working_tree_changes(
    context: &PushLocalWorkingTreeContext<'_>,
    root: &Path,
    changes: Vec<WorkingTreeChange>,
) -> Result<LocalSyncResult, CliError> {
    let tree_state = read_working_tree_state(root)?;
    if changes.is_empty() {
        return Ok(LocalSyncResult::default());
    }

    let intents = plan_working_tree_change_intents(&tree_state, &changes);
    let mut current_key_version_by_folder = context
        .export
        .folders
        .iter()
        .map(|folder| {
            (
                (context.export.brain.id.clone(), folder.id.clone()),
                folder.current_key_version,
            )
        })
        .collect::<BTreeMap<_, _>>();
    for mounted in context.mounted_exports {
        if let Some(folder) = mounted.source_folder() {
            current_key_version_by_folder.insert(
                (mounted.export.brain.id.clone(), folder.id.clone()),
                folder.current_key_version,
            );
        }
    }
    let signing_keys = load_signer(context.env)?.keys;
    let actor_npub = NostrPublicKey::from_protocol(signing_keys.public_key())
        .to_npub()
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;

    let submit_context = SubmitIntentContext {
        env: context.env,
        server_url: context.server_url,
        agent_state: context.agent_state,
        signing_keys: &signing_keys,
        actor_npub: &actor_npub,
        session_keys: context.session_keys,
        current_key_version_by_folder: &current_key_version_by_folder,
    };
    let mut result = LocalSyncResult::default();
    let mut conflicts = Vec::new();
    for (change, intent) in changes.iter().zip(intents.iter()) {
        match submit_change_intent(&submit_context, intent) {
            Ok(SubmitIntentOutcome::Submitted) => {
                result.pushed_count += 1;
                result
                    .changes
                    .push(sync_change_report(change, intent, "pushed", None));
                if let (Some(folder_id), Some(object_id), Some(target_path)) = (
                    intent.folder_id.as_ref(),
                    intent.object_id.as_ref(),
                    intent.target_path.as_ref(),
                ) {
                    let route_brain_id = intent
                        .source_brain_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| context.agent_state.brain_id.clone());
                    result.path_overrides.insert(
                        (
                            route_brain_id,
                            folder_id.to_string(),
                            object_id.as_str().to_owned(),
                        ),
                        target_path.to_string(),
                    );
                }
            }
            Ok(SubmitIntentOutcome::Conflict(reason)) => {
                result.conflict_count += 1;
                preserve_conflicted_content(&mut result, change);
                result.changes.push(sync_change_report(
                    change,
                    intent,
                    "conflicted",
                    Some(reason.clone()),
                ));
                conflicts.push(conflict_for_change(
                    change,
                    intent,
                    reason,
                    timestamp(context.env),
                ));
            }
            Err(error) if is_http_conflict(&error) => {
                result.conflict_count += 1;
                preserve_conflicted_content(&mut result, change);
                result.changes.push(sync_change_report(
                    change,
                    intent,
                    "conflicted",
                    Some(error.to_string()),
                ));
                conflicts.push(conflict_for_change(
                    change,
                    intent,
                    error.to_string(),
                    timestamp(context.env),
                ));
            }
            Err(error) => return Err(error),
        }
    }

    if !conflicts.is_empty() {
        mutate_agent_state_at_root(root, timestamp(context.env), |state, now| {
            for conflict in conflicts {
                if !state.conflicts.iter().any(|existing| {
                    existing.id == conflict.id && existing.state == ConflictState::Open
                }) {
                    state.conflicts.push(conflict);
                }
            }
            state.add_activity(now, "sync.blocked", "Local working-tree conflicts recorded");
        })?;
    }

    Ok(result)
}

fn sync_change_report(
    change: &WorkingTreeChange,
    intent: &WorkingTreeChangeIntent,
    status: &str,
    reason: Option<String>,
) -> SyncChangeReport {
    let (path, from_path) = match change {
        WorkingTreeChange::Upsert { path, .. }
        | WorkingTreeChange::UpsertAsset { path, .. }
        | WorkingTreeChange::Delete { path } => (Some(path.to_string()), None),
        WorkingTreeChange::Rename { from_path, to_path } => {
            (Some(to_path.to_string()), Some(from_path.to_string()))
        }
    };
    SyncChangeReport {
        status: status.to_owned(),
        action: sync_action_label(intent.action).to_owned(),
        actor_npub: None,
        sequence: None,
        path,
        from_path,
        folder_id: intent.folder_id.as_ref().map(ToString::to_string),
        source_brain_id: intent.source_brain_id.as_ref().map(ToString::to_string),
        object_id: intent
            .object_id
            .as_ref()
            .map(|object| object.as_str().to_owned()),
        route: sync_route_label(intent.route).to_owned(),
        reason,
    }
}

fn sync_action_label(action: WorkingTreeIntentAction) -> &'static str {
    match action {
        WorkingTreeIntentAction::Create => "create",
        WorkingTreeIntentAction::Update => "update",
        WorkingTreeIntentAction::Move => "move",
        WorkingTreeIntentAction::Delete => "delete",
        WorkingTreeIntentAction::Unresolved => "unresolved",
    }
}

fn sync_route_label(route: WorkingTreeIntentRoute) -> &'static str {
    match route {
        WorkingTreeIntentRoute::EncryptedObjectWrite => "encrypted-object-write",
        WorkingTreeIntentRoute::EncryptedObjectMove => "encrypted-object-move",
        WorkingTreeIntentRoute::EncryptedObjectDelete => "encrypted-object-delete",
        WorkingTreeIntentRoute::Unresolved => "unresolved",
    }
}

struct SubmitIntentContext<'a> {
    env: &'a CliEnvironment,
    server_url: &'a str,
    agent_state: &'a AgentState,
    signing_keys: &'a Keys,
    actor_npub: &'a str,
    session_keys: &'a SessionFolderKeyring,
    current_key_version_by_folder: &'a BTreeMap<(String, String), u32>,
}

fn submit_change_intent(
    context: &SubmitIntentContext<'_>,
    intent: &WorkingTreeChangeIntent,
) -> Result<SubmitIntentOutcome, CliError> {
    if intent.route == WorkingTreeIntentRoute::Unresolved
        || intent.action == WorkingTreeIntentAction::Unresolved
    {
        return Ok(SubmitIntentOutcome::Conflict(
            intent
                .reason
                .clone()
                .unwrap_or_else(|| "working-tree change could not be mapped".to_owned()),
        ));
    }

    let folder_id = intent
        .folder_id
        .as_ref()
        .ok_or_else(|| CliError::InvalidInput("missing intent folder id".to_owned()))?;
    let object_id = intent
        .object_id
        .as_ref()
        .ok_or_else(|| CliError::InvalidInput("missing intent object id".to_owned()))?;
    let route_brain_id = intent
        .source_brain_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| context.agent_state.brain_id.clone());
    let Some(current_key_version) = context
        .current_key_version_by_folder
        .get(&(route_brain_id.clone(), folder_id.to_string()))
        .copied()
    else {
        return Ok(SubmitIntentOutcome::Conflict(format!(
            "folder {folder_id} is missing from encrypted export for brain {route_brain_id}"
        )));
    };
    let current_session_key =
        context
            .session_keys
            .get(&route_brain_id, &folder_id.to_string(), current_key_version);
    if current_session_key.is_none() {
        return Ok(SubmitIntentOutcome::Conflict(format!(
            "current Folder Key v{current_key_version} unavailable for {route_brain_id}/{folder_id}"
        )));
    }

    match intent.action {
        WorkingTreeIntentAction::Create
        | WorkingTreeIntentAction::Update
        | WorkingTreeIntentAction::Move => {
            let content = intent.content.as_ref().ok_or_else(|| {
                CliError::InvalidInput("write intent is missing plaintext content".to_owned())
            })?;
            let target_path = intent.target_path.as_ref().ok_or_else(|| {
                CliError::InvalidInput("write intent is missing target path".to_owned())
            })?;
            let key = current_session_key.expect("checked above");
            let aad = FolderObjectAad {
                brain_id: BrainId::new(route_brain_id.clone())
                    .map_err(|error| CliError::InvalidInput(error.to_string()))?,
                folder_id: folder_id.clone(),
                object_id: object_id.clone(),
                key_version: current_key_version,
            };
            let plaintext = match content {
                WorkingTreeIntentContent::PageMarkdown(markdown) => {
                    encode_folder_object_page_plaintext(target_path, markdown)?
                }
                WorkingTreeIntentContent::AssetBytes {
                    bytes,
                    content_type,
                    ..
                } => encode_folder_object_asset_plaintext(target_path, bytes, content_type)?,
            };
            let envelope = encrypt_folder_object(key, &aad, &plaintext)
                .map_err(|error| CliError::InvalidInput(error.to_string()))?;
            let envelope_json = envelope.canonical_json();
            let operation = match intent.action {
                WorkingTreeIntentAction::Create => FolderObjectOperation::Create,
                WorkingTreeIntentAction::Update => FolderObjectOperation::Update,
                WorkingTreeIntentAction::Move => FolderObjectOperation::Move,
                _ => unreachable!("handled above"),
            };
            let event = signed_revision_event(
                context.signing_keys,
                RevisionEventInput {
                    actor_npub: context.actor_npub,
                    brain_id: &route_brain_id,
                    folder_id,
                    object_id,
                    operation,
                    base_revision: intent.base_revision,
                    key_version: current_key_version,
                    envelope_json: envelope_json.clone(),
                },
            )?;
            let body = serde_json::json!({
                "baseRevision": intent.base_revision,
                "keyVersion": current_key_version,
                "cipher": CIPHER_AES_256_GCM,
                "ciphertext": envelope_json,
                "revisionEvent": event
            });
            let route = match intent.action {
                WorkingTreeIntentAction::Move => format!(
                    "/v1/brains/{}/folders/{}/objects/{}/move",
                    route_brain_id,
                    folder_id,
                    object_id.as_str()
                ),
                _ => format!(
                    "/v1/brains/{}/folders/{}/objects/{}",
                    route_brain_id,
                    folder_id,
                    object_id.as_str()
                ),
            };
            signed_json_request_to_server(
                context.env,
                context.server_url,
                if intent.action == WorkingTreeIntentAction::Move {
                    "POST"
                } else {
                    "PUT"
                },
                &route,
                Some(body),
            )?;
            Ok(SubmitIntentOutcome::Submitted)
        }
        WorkingTreeIntentAction::Delete => {
            let base_revision = intent.base_revision.ok_or_else(|| {
                CliError::InvalidInput("delete intent is missing base revision".to_owned())
            })?;
            let event = signed_tombstone_event(
                context.signing_keys,
                context.actor_npub,
                &route_brain_id,
                folder_id,
                object_id,
                base_revision,
            )?;
            let body = serde_json::json!({
                "baseRevision": base_revision,
                "tombstoneEvent": event
            });
            let route = format!(
                "/v1/brains/{}/folders/{}/objects/{}",
                route_brain_id,
                folder_id,
                object_id.as_str()
            );
            signed_json_request_to_server(
                context.env,
                context.server_url,
                "DELETE",
                &route,
                Some(body),
            )?;
            Ok(SubmitIntentOutcome::Submitted)
        }
        WorkingTreeIntentAction::Unresolved => Ok(SubmitIntentOutcome::Conflict(
            intent
                .reason
                .clone()
                .unwrap_or_else(|| "working-tree change could not be mapped".to_owned()),
        )),
    }
}

pub(crate) struct RevisionEventInput<'a> {
    pub(crate) actor_npub: &'a str,
    pub(crate) brain_id: &'a str,
    pub(crate) folder_id: &'a FolderId,
    pub(crate) object_id: &'a ObjectId,
    pub(crate) operation: FolderObjectOperation,
    pub(crate) base_revision: Option<u64>,
    pub(crate) key_version: u32,
    pub(crate) envelope_json: String,
}

pub(crate) fn signed_revision_event(
    keys: &Keys,
    input: RevisionEventInput<'_>,
) -> Result<serde_json::Value, CliError> {
    let created_at_unix = unix_timestamp();
    let expected = RevisionValidation {
        brain_id: BrainId::new(input.brain_id.to_owned())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        folder_id: input.folder_id.clone(),
        object_id: input.object_id.clone(),
        operation: input.operation,
        revision: input.base_revision.map_or(1, |base| base + 1),
        base_revision: input.base_revision,
        key_version: input.key_version,
        envelope_json: input.envelope_json,
        author_npub: input.actor_npub.to_owned(),
        created_at: timestamp_from_unix(created_at_unix),
    };
    let payload = FolderObjectRevisionPayload::new(&expected);
    let event = sign_event(
        keys,
        Kind::Custom(APP_SPECIFIC_KIND),
        payload.canonical_json(),
        revision_tags(&expected)?,
        created_at_unix,
        Some("folder-object-revision"),
    )?;
    serde_json::from_str(&event.as_json()).map_err(CliError::from)
}

fn signed_tombstone_event(
    keys: &Keys,
    actor_npub: &str,
    brain_id: &str,
    folder_id: &FolderId,
    object_id: &ObjectId,
    base_revision: u64,
) -> Result<serde_json::Value, CliError> {
    let created_at_unix = unix_timestamp();
    let expected = TombstoneValidation {
        brain_id: BrainId::new(brain_id.to_owned())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        folder_id: folder_id.clone(),
        object_id: object_id.clone(),
        revision: base_revision + 1,
        base_revision,
        author_npub: actor_npub.to_owned(),
        deleted_at: timestamp_from_unix(created_at_unix),
    };
    let payload = finite_brain_core::FolderObjectTombstonePayload::new(&expected);
    let event = sign_event(
        keys,
        Kind::Custom(APP_SPECIFIC_KIND),
        payload.canonical_json(),
        tombstone_tags(&expected)?,
        created_at_unix,
        Some("folder-object-tombstone"),
    )?;
    serde_json::from_str(&event.as_json()).map_err(CliError::from)
}

fn revision_tags(input: &RevisionValidation) -> Result<Vec<Tag>, CliError> {
    Ok(vec![
        tag_vec([
            "d",
            &format!(
                "finite-folder-object-revision:{}:{}:{}:{}",
                input.brain_id,
                input.folder_id,
                input.object_id.as_str(),
                input.revision
            ),
        ])?,
        tag_vec(["brain", &input.brain_id.to_string()])?,
        tag_vec(["folder", &input.folder_id.to_string()])?,
        tag_vec(["object", input.object_id.as_str()])?,
        tag_vec(["operation", input.operation.as_str()])?,
        tag_vec(["keyVersion", &input.key_version.to_string()])?,
    ])
}

fn tombstone_tags(input: &TombstoneValidation) -> Result<Vec<Tag>, CliError> {
    Ok(vec![
        tag_vec([
            "d",
            &format!(
                "finite-folder-object-tombstone:{}:{}:{}:{}",
                input.brain_id,
                input.folder_id,
                input.object_id.as_str(),
                input.revision
            ),
        ])?,
        tag_vec(["brain", &input.brain_id.to_string()])?,
        tag_vec(["folder", &input.folder_id.to_string()])?,
        tag_vec(["object", input.object_id.as_str()])?,
        tag_vec(["operation", "delete"])?,
    ])
}

struct MaterializeRemoteProjectionContext<'a> {
    env: &'a CliEnvironment,
    root: &'a Path,
    actor_npub: &'a str,
    metadata: Option<&'a CliBrainMetadata>,
    export: &'a CliEncryptedBrainExport,
    bootstrap: &'a CliSyncBootstrap,
    mounted_folders: &'a [MountedFolderMaterializeContext],
    path_overrides: &'a BTreeMap<(String, String, String), String>,
    session_keys: &'a SessionFolderKeyring,
    prior_state: Option<&'a BrainWorkingTreeStateManifest>,
}

fn materialize_remote_projection(
    context: MaterializeRemoteProjectionContext<'_>,
) -> Result<Vec<SyncChangeReport>, CliError> {
    let MaterializeRemoteProjectionContext {
        env,
        root,
        actor_npub,
        metadata,
        export,
        bootstrap,
        mounted_folders,
        path_overrides,
        session_keys,
        prior_state,
    } = context;
    let loaded_prior_state;
    let prior_state = if let Some(prior_state) = prior_state {
        prior_state
    } else {
        loaded_prior_state = read_working_tree_state(root)?;
        &loaded_prior_state
    };
    let brain = brain_from_export(export)?;
    let mut prior_paths = prior_state
        .objects
        .iter()
        .map(|entry| {
            (
                (
                    entry
                        .source_brain_id
                        .clone()
                        .unwrap_or_else(|| export.brain.id.clone()),
                    entry.folder_id.clone(),
                    entry.object_id.clone(),
                ),
                entry.path.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for (key, path) in path_overrides {
        prior_paths.insert(key.clone(), path.clone());
    }
    let mut opened_pages = Vec::new();
    let mut unsupported_objects = Vec::new();
    let mut preserved_legacy_objects = Vec::new();
    let mut readable_folder_routes = BTreeSet::new();
    {
        let mut append_context = OpenedObjectsAppendContext {
            session_keys,
            prior_paths: &prior_paths,
            prior_state,
            opened_pages: &mut opened_pages,
            unsupported_objects: &mut unsupported_objects,
            preserved_legacy_objects: &mut preserved_legacy_objects,
            readable_folder_routes: &mut readable_folder_routes,
        };

        append_opened_objects_from_bootstrap(export, bootstrap, None, None, &mut append_context)?;
        for mounted in mounted_folders {
            append_opened_objects_from_bootstrap(
                &mounted.export,
                &mounted.bootstrap,
                Some(&mounted.mount.source_folder_id),
                Some(&mounted.display_path),
                &mut append_context,
            )?;
        }
    }

    for folder in &export.folders {
        if session_keys.contains(&export.brain.id, &folder.id, folder.current_key_version) {
            readable_folder_routes.insert((export.brain.id.clone(), folder.id.clone()));
        }
    }
    for mounted in mounted_folders {
        if let Some(folder) = mounted.source_folder()
            && session_keys.contains(
                &mounted.export.brain.id,
                &folder.id,
                folder.current_key_version,
            )
        {
            readable_folder_routes.insert((mounted.export.brain.id.clone(), folder.id.clone()));
        }
    }

    let locked_folders = export
        .folders
        .iter()
        .filter(|folder| {
            !readable_folder_routes.contains(&(export.brain.id.clone(), folder.id.clone()))
        })
        .map(|folder| {
            Ok(OkfOmittedFolder {
                folder_id: FolderId::new(folder.id.clone())
                    .map_err(|error| CliError::InvalidInput(error.to_string()))?,
                source_brain_id: None,
                display_path: SafeRelativePath::new("folder_path", folder.path.clone())
                    .map_err(|error| CliError::InvalidInput(error.to_string()))?,
                reason: if folder.accessible {
                    "missing-folder-key".to_owned()
                } else {
                    "no-folder-access".to_owned()
                },
            })
        })
        .collect::<Result<Vec<_>, CliError>>()?;

    let mut projection = materialize_brain_working_tree(WorkingTreeMaterializeInput {
        generated_at: timestamp(env),
        generated_by_npub: UserId::new(actor_npub.to_owned())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        acting_role: metadata
            .map(|metadata| brain_role_for_actor(&brain, metadata, actor_npub))
            .unwrap_or_else(|| "unknown".to_owned()),
        brain,
        opened_pages,
        opened_assets: Vec::new(),
        locked_folders,
        latest_sequence: bootstrap.latest_sequence,
    })
    .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    if metadata.is_none() {
        // Never overwrite previously authoritative role context with a guess.
        // On a first materialization this intentionally leaves root context
        // absent until metadata can be fetched successfully.
        projection.files.remove("AGENTS.md");
    }
    add_empty_readable_folders(&mut projection, export, None, &readable_folder_routes, None)?;
    for mounted in mounted_folders {
        add_empty_readable_folders(
            &mut projection,
            &mounted.export,
            Some(&mounted.export.brain.id),
            &readable_folder_routes,
            Some((&mounted.mount.source_folder_id, &mounted.display_path)),
        )?;
    }
    validate_preserved_legacy_object_paths(&projection, &prior_state.objects)?;
    projection.state.objects.extend(preserved_legacy_objects);
    let mut deleted_routes = deleted_folder_routes(export, bootstrap)?;
    for mounted in mounted_folders {
        deleted_routes.extend(deleted_folder_routes(&mounted.export, &mounted.bootstrap)?);
    }
    preserve_unreadable_prior_projection(
        prior_state,
        &mut projection,
        &export.brain.id,
        &readable_folder_routes,
        &deleted_routes,
    )?;
    remove_stale_object_files(root, &prior_state.objects, &projection.state.objects)?;
    remove_obsolete_compiled_convention_markers(root, &projection.state.folder_roots)?;
    write_projection_files(root, &projection.files, &projection.binary_files)?;
    Ok(unsupported_objects)
}

fn brain_role_for_actor(brain: &Brain, metadata: &CliBrainMetadata, actor_npub: &str) -> String {
    if brain.kind == BrainKind::Personal {
        if brain
            .owner_user_id
            .as_ref()
            .is_some_and(|owner| owner.as_str() == actor_npub)
        {
            "owner"
        } else if metadata
            .personal_agent
            .as_ref()
            .is_some_and(|agent| agent.agent_npub == actor_npub)
        {
            "personal_agent"
        } else if brain
            .members
            .iter()
            .any(|member| member.user_id.as_str() == actor_npub)
        {
            "member"
        } else {
            "guest"
        }
    } else if brain
        .admins
        .iter()
        .any(|admin| admin.as_str() == actor_npub)
    {
        "admin"
    } else if brain
        .members
        .iter()
        .any(|member| member.user_id.as_str() == actor_npub)
    {
        "member"
    } else {
        "guest"
    }
    .to_owned()
}

fn validate_preserved_legacy_object_paths(
    projection: &WorkingTreeProjection,
    preserved: &[WorkingTreeObjectManifestEntry],
) -> Result<(), CliError> {
    for object in preserved {
        if object.content_type == "text/markdown" {
            continue;
        }
        let folder = projection
            .state
            .folder_roots
            .iter()
            .find(|folder| {
                folder.folder_id == object.folder_id
                    && folder.source_brain_id == object.source_brain_id
            })
            .ok_or_else(|| {
                CliError::InvalidInput(format!(
                    "legacy Asset {} has no materialized Folder root",
                    object.object_id
                ))
            })?;
        let path = if folder.path.is_empty() {
            object.path.clone()
        } else {
            format!("{}/{}", folder.path, object.path)
        };
        let path = SafeRelativePath::new("legacy_asset_path", path)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?
            .to_string();
        let collides_with_file = projection.files.keys().any(|candidate| {
            candidate == &path
                || candidate.starts_with(&format!("{path}/"))
                || path.starts_with(&format!("{candidate}/"))
        });
        let collides_with_folder = projection.state.folder_roots.iter().any(|candidate| {
            candidate.path == path || candidate.path.starts_with(&format!("{path}/"))
        });
        if collides_with_file || collides_with_folder {
            return Err(CliError::InvalidInput(format!(
                "legacy Asset path collision at {path}; preserved bytes were not changed"
            )));
        }
    }
    Ok(())
}

fn preserve_unreadable_prior_projection(
    prior_state: &BrainWorkingTreeStateManifest,
    projection: &mut WorkingTreeProjection,
    primary_brain_id: &str,
    readable_folder_routes: &BTreeSet<(String, String)>,
    deleted_folder_routes: &BTreeSet<(String, String)>,
) -> Result<(), CliError> {
    let is_unreadable = |source_brain_id: Option<&str>, folder_id: &str| {
        let source_brain_id = source_brain_id.unwrap_or(primary_brain_id);
        !readable_folder_routes.contains(&(source_brain_id.to_owned(), folder_id.to_owned()))
    };

    for root in &prior_state.folder_roots {
        let source_brain_id = root.source_brain_id.as_deref().unwrap_or(primary_brain_id);
        if deleted_folder_routes.contains(&(source_brain_id.to_owned(), root.folder_id.clone())) {
            continue;
        }
        let route = (root.source_brain_id.clone(), root.folder_id.clone());
        if !is_unreadable(root.source_brain_id.as_deref(), &root.folder_id) {
            continue;
        }
        if let Some(candidate) = projection.state.folder_roots.iter_mut().find(|candidate| {
            (
                candidate.source_brain_id.clone(),
                candidate.folder_id.clone(),
            ) == route
        }) {
            candidate.path.clone_from(&root.path);
            candidate.can_read = false;
            candidate.metadata_only = true;
        } else {
            let mut preserved = root.clone();
            preserved.can_read = false;
            preserved.metadata_only = true;
            projection.state.folder_roots.push(preserved);
        }
    }

    for object in &prior_state.objects {
        let source_brain_id = object
            .source_brain_id
            .as_deref()
            .unwrap_or(primary_brain_id);
        if deleted_folder_routes.contains(&(source_brain_id.to_owned(), object.folder_id.clone())) {
            continue;
        }
        let route_is_unreadable =
            is_unreadable(object.source_brain_id.as_deref(), &object.folder_id);
        let object_key = (
            object.source_brain_id.clone(),
            object.folder_id.clone(),
            object.object_id.clone(),
        );
        if route_is_unreadable
            && !projection.state.objects.iter().any(|candidate| {
                (
                    candidate.source_brain_id.clone(),
                    candidate.folder_id.clone(),
                    candidate.object_id.clone(),
                ) == object_key
            })
        {
            projection.state.objects.push(object.clone());
        }
    }

    projection.state.folder_roots.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.source_brain_id.cmp(&right.source_brain_id))
            .then(left.folder_id.cmp(&right.folder_id))
    });
    projection.state.objects.sort_by(|left, right| {
        left.source_brain_id
            .cmp(&right.source_brain_id)
            .then(left.folder_id.cmp(&right.folder_id))
            .then(left.path.cmp(&right.path))
    });
    projection.files.insert(
        ".finitebrain/working-tree-state.json".to_owned(),
        serde_json::to_string_pretty(&projection.state)?,
    );
    Ok(())
}

fn deleted_folder_routes(
    export: &CliEncryptedBrainExport,
    bootstrap: &CliSyncBootstrap,
) -> Result<BTreeSet<(String, String)>, CliError> {
    let mut routes = BTreeSet::new();
    for record in &bootstrap.control_records {
        let payload = decode_sync_payload(&record.payload_json);
        if !payload.is_folder_subtree_tombstone() {
            continue;
        }
        for folder_id in
            folder_subtree_tombstone_ids(record, &payload).map_err(CliError::InvalidInput)?
        {
            routes.insert((export.brain.id.clone(), folder_id));
        }
    }
    Ok(routes)
}

fn remove_deleted_folder_roots(
    root: &Path,
    prior_state: &BrainWorkingTreeStateManifest,
    prior_export: Option<&CliEncryptedBrainExport>,
    deleted_folder_routes: &BTreeSet<(String, String)>,
    primary_brain_id: &str,
) -> Result<(), CliError> {
    let mut deleted_paths = BTreeSet::new();
    for folder in &prior_state.folder_roots {
        let source_brain_id = folder
            .source_brain_id
            .as_deref()
            .unwrap_or(primary_brain_id);
        if !deleted_folder_routes.contains(&(source_brain_id.to_owned(), folder.folder_id.clone()))
        {
            continue;
        }
        deleted_paths.insert(folder.path.clone());
    }
    if let Some(prior_export) = prior_export {
        for folder in &prior_export.folders {
            if deleted_folder_routes.contains(&(prior_export.brain.id.clone(), folder.id.clone())) {
                deleted_paths.insert(folder.path.clone());
            }
        }
    }
    for deleted_path in deleted_paths {
        if deleted_path.is_empty() || deleted_path == "." {
            continue;
        }
        let path = SafeRelativePath::new("folder_path", deleted_path)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        // The empty display path represents the Brain's logical root folder.
        // It is not a removable directory: joining it to `root` resolves to the
        // Working Tree itself and would also delete the local `.finitebrain`
        // control state needed to recover and continue syncing.
        if path.as_str().is_empty() || path.as_str() == "." {
            continue;
        }
        let path = root.join(path.as_str());
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || metadata.is_file() {
            fs::remove_file(path)?;
        } else if metadata.is_dir() {
            fs::remove_dir_all(path)?;
        }
    }
    Ok(())
}

struct OpenedObjectsAppendContext<'a> {
    session_keys: &'a SessionFolderKeyring,
    prior_paths: &'a BTreeMap<(String, String, String), String>,
    prior_state: &'a BrainWorkingTreeStateManifest,
    opened_pages: &'a mut Vec<OpenedPage>,
    unsupported_objects: &'a mut Vec<SyncChangeReport>,
    preserved_legacy_objects: &'a mut Vec<WorkingTreeObjectManifestEntry>,
    readable_folder_routes: &'a mut BTreeSet<(String, String)>,
}

fn append_opened_objects_from_bootstrap(
    export: &CliEncryptedBrainExport,
    bootstrap: &CliSyncBootstrap,
    only_folder_id: Option<&str>,
    display_path_override: Option<&str>,
    context: &mut OpenedObjectsAppendContext<'_>,
) -> Result<(), CliError> {
    let source_brain_id = BrainId::new(export.brain.id.clone())
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    for object in bootstrap.objects.iter().filter(|object| {
        !object.deleted && only_folder_id.is_none_or(|folder_id| folder_id == object.folder_id)
    }) {
        let envelope = EncryptedFolderObjectEnvelope::from_json(&object.ciphertext)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let Some(folder_key) =
            context
                .session_keys
                .get(&export.brain.id, &object.folder_id, envelope.key_version)
        else {
            continue;
        };
        let aad = FolderObjectAad {
            brain_id: source_brain_id.clone(),
            folder_id: FolderId::new(object.folder_id.clone())
                .map_err(|error| CliError::InvalidInput(error.to_string()))?,
            object_id: ObjectId::new(object.object_id.clone())
                .map_err(|error| CliError::InvalidInput(error.to_string()))?,
            key_version: envelope.key_version,
        };
        let plaintext = open_folder_object(folder_key, &aad, &envelope)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let folder = export
            .folders
            .iter()
            .find(|folder| folder.id == object.folder_id)
            .ok_or_else(|| CliError::NotFound(format!("folder {}", object.folder_id)))?;
        let fallback_object_path = context
            .prior_paths
            .get(&(
                export.brain.id.clone(),
                object.folder_id.clone(),
                object.object_id.clone(),
            ))
            .cloned()
            .unwrap_or_else(|| format!("{}.md", object.object_id));
        context
            .readable_folder_routes
            .insert((export.brain.id.clone(), object.folder_id.clone()));
        let folder_id = FolderId::new(object.folder_id.clone())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let object_id = ObjectId::new(object.object_id.clone())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let folder_display_path = SafeRelativePath::new(
            "folder_path",
            display_path_override.unwrap_or(&folder.path).to_owned(),
        )
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        match decode_folder_object_plaintext(plaintext, fallback_object_path)? {
            CliDecodedFolderObjectPlaintext::Page { path, markdown } => {
                context.opened_pages.push(OpenedPage {
                    folder_id,
                    source_brain_id: display_path_override.map(|_| source_brain_id.clone()),
                    object_id,
                    folder_display_path,
                    page_path: SafeRelativePath::new("page_path", path)
                        .map_err(|error| CliError::InvalidInput(error.to_string()))?,
                    markdown,
                    revision: object.revision,
                    key_version: envelope.key_version,
                    content_type: "text/markdown".to_owned(),
                });
            }
            CliDecodedFolderObjectPlaintext::UnsupportedAsset { path, content_type } => {
                let projected_path = format!("{folder_display_path}/{path}");
                let manifest_source_brain_id =
                    display_path_override.map(|_| export.brain.id.clone());
                if let Some(prior) = context.prior_state.objects.iter().find(|prior| {
                    prior.folder_id == object.folder_id
                        && prior.object_id == object.object_id
                        && prior.source_brain_id == manifest_source_brain_id
                        && prior.content_type != "text/markdown"
                }) {
                    let mut preserved = prior.clone();
                    preserved.revision = object.revision;
                    preserved.key_version = envelope.key_version;
                    if !context.preserved_legacy_objects.iter().any(|candidate| {
                        candidate.object_id == preserved.object_id
                            && candidate.folder_id == preserved.folder_id
                            && candidate.source_brain_id == preserved.source_brain_id
                    }) {
                        context.preserved_legacy_objects.push(preserved);
                    }
                }
                context.unsupported_objects.push(SyncChangeReport {
                    status: "unsupported".to_owned(),
                    action: "preserve".to_owned(),
                    actor_npub: None,
                    sequence: None,
                    path: Some(projected_path),
                    from_path: None,
                    folder_id: Some(folder_id.to_string()),
                    source_brain_id: manifest_source_brain_id,
                    object_id: Some(object_id.as_str().to_owned()),
                    route: "encrypted-record-preserved".to_owned(),
                    reason: Some(format!(
                        "legacy inline Asset ({content_type}) remains encrypted on the server; bytes were not materialized"
                    )),
                });
            }
        }
    }
    Ok(())
}

fn add_empty_readable_folders(
    projection: &mut WorkingTreeProjection,
    export: &CliEncryptedBrainExport,
    source_brain_id: Option<&str>,
    readable_folder_routes: &BTreeSet<(String, String)>,
    only_folder_and_path: Option<(&str, &str)>,
) -> Result<(), CliError> {
    let existing = projection
        .state
        .folder_roots
        .iter()
        .map(|root| (root.source_brain_id.clone(), root.folder_id.clone()))
        .collect::<BTreeSet<_>>();
    for folder in export.folders.iter().filter(|folder| {
        only_folder_and_path.is_none_or(|(folder_id, _)| folder_id == folder.id)
            && readable_folder_routes.contains(&(export.brain.id.clone(), folder.id.clone()))
            && !existing.contains(&(source_brain_id.map(ToOwned::to_owned), folder.id.clone()))
    }) {
        let folder_path = only_folder_and_path
            .map(|(_, display_path)| display_path.to_owned())
            .unwrap_or_else(|| folder.path.clone());
        projection.state.folder_roots.push(WorkingTreeFolderRoot {
            folder_id: folder.id.clone(),
            source_brain_id: source_brain_id.map(ToOwned::to_owned),
            path: folder_path.clone(),
            can_read: true,
            metadata_only: false,
        });
        projection.files.insert(
            format!("{}/AGENTS.md", folder_path),
            folder_agent_instructions(&folder.id),
        );
        projection.files.insert(
            format!("{}/_index.md", folder_path),
            format!("# {}\n\n", folder_path),
        );
        for convention in FOLDER_CONVENTION_DIRECTORIES {
            projection.files.insert(
                format!("{}/{convention}/.keep", folder_path),
                folder_convention_marker(&folder.id, convention),
            );
        }
    }
    projection
        .state
        .folder_roots
        .sort_by(|left, right| left.path.cmp(&right.path));
    projection.files.insert(
        ".finitebrain/working-tree-state.json".to_owned(),
        serde_json::to_string_pretty(&projection.state)?,
    );
    Ok(())
}

fn brain_from_export(export: &CliEncryptedBrainExport) -> Result<Brain, CliError> {
    let kind = match export.brain.kind.as_str() {
        "personal" => BrainKind::Personal,
        "organization" => BrainKind::Organization,
        other => {
            return Err(CliError::InvalidInput(format!(
                "unknown brain kind {other}"
            )));
        }
    };
    Ok(Brain {
        id: BrainId::new(export.brain.id.clone())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        kind,
        name: DisplayName::new("brain_name", export.brain.name.clone())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        owner_user_id: export
            .brain
            .owner_user_id
            .clone()
            .map(UserId::new)
            .transpose()
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        folders: export
            .folders
            .iter()
            .map(folder_from_export)
            .collect::<Result<Vec<_>, _>>()?,
        members: export
            .access_state
            .members
            .iter()
            .map(|member| {
                Ok(finite_brain_core::BrainMember {
                    user_id: UserId::new(member.clone())
                        .map_err(|error| CliError::InvalidInput(error.to_string()))?,
                    folder_access: BTreeSet::new(),
                })
            })
            .collect::<Result<Vec<_>, CliError>>()?,
        admins: export
            .access_state
            .admins
            .iter()
            .map(|admin| {
                UserId::new(admin.clone())
                    .map_err(|error| CliError::InvalidInput(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn folder_from_export(folder: &CliExportFolder) -> Result<Folder, CliError> {
    Ok(Folder {
        id: FolderId::new(folder.id.clone())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        name: DisplayName::new(
            "folder_name",
            folder
                .path
                .split('/')
                .next_back()
                .unwrap_or(folder.id.as_str())
                .to_owned(),
        )
        .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        role: match folder.id.as_str() {
            "home" => FolderRole::PersonalHome,
            "brain-ops" => FolderRole::BrainOps,
            "general" => FolderRole::General,
            _ => FolderRole::Folder,
        },
        access: parse_folder_access(&folder.access)?,
        parent_folder_id: None,
        path: SafeRelativePath::new("folder_path", folder.path.clone())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        current_key_version: folder.current_key_version,
    })
}

fn parse_folder_access(access: &str) -> Result<FolderAccessMode, CliError> {
    match access {
        "owner" => Ok(FolderAccessMode::Owner),
        "admin_only" => Ok(FolderAccessMode::AdminOnly),
        "all_members" => Ok(FolderAccessMode::AllMembers),
        "restricted" => Ok(FolderAccessMode::Restricted),
        other => Err(CliError::InvalidInput(format!(
            "unknown folder access mode {other}"
        ))),
    }
}

fn scan_working_tree_changes(
    root: &Path,
    state: &finite_brain_core::portability::BrainWorkingTreeStateManifest,
) -> Result<Vec<WorkingTreeChange>, CliError> {
    let mut changes = Vec::new();
    let known = state
        .objects
        .iter()
        .map(|object| {
            (
                format!(
                    "{}/{}",
                    folder_path_for_object(state, object).unwrap_or_default(),
                    object.path
                ),
                object,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();

    for folder in state.folder_roots.iter().filter(|folder| folder.can_read) {
        let folder_root = root.join(&folder.path);
        if !folder_root.exists() {
            continue;
        }
        let file_paths = collect_working_tree_file_paths(root, &folder_root)?;
        for relative_path in file_paths {
            if is_generated_folder_file(&folder.path, &relative_path) {
                continue;
            }
            if is_ignored_os_metadata_file(&relative_path) {
                continue;
            }
            seen.insert(relative_path.clone());
            if is_markdown_path(&relative_path) {
                let body = fs::read_to_string(root.join(&relative_path))?;
                match known.get(&relative_path) {
                    Some(object) if object.content_hash == sha256_hex(body.as_bytes()) => {}
                    _ => changes.push(WorkingTreeChange::Upsert {
                        path: SafeRelativePath::new("change_path", relative_path)
                            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
                        markdown: body,
                    }),
                }
            } else if known
                .get(&relative_path)
                .is_some_and(|object| object.content_type != "text/markdown")
            {
                // A pre-hard-cut client may have materialized these bytes and
                // recorded them in the old manifest. Keep the local copy as a
                // derived legacy artifact, while the remote projection reports
                // the encrypted object as unsupported. It is not a pending
                // local write and must not become a permanent sync conflict.
                continue;
            } else {
                changes.push(WorkingTreeChange::UpsertAsset {
                    path: SafeRelativePath::new("change_path", relative_path)
                        .map_err(|error| CliError::InvalidInput(error.to_string()))?,
                    bytes: Vec::new(),
                    content_type: "application/octet-stream".to_owned(),
                    has_source_note: false,
                });
            }
        }
    }

    for (relative_path, object) in known {
        if object.content_type != "text/markdown" {
            continue;
        }
        if !seen.contains(&relative_path) && !root.join(&relative_path).exists() {
            changes.push(WorkingTreeChange::Delete {
                path: SafeRelativePath::new("change_path", relative_path)
                    .map_err(|error| CliError::InvalidInput(error.to_string()))?,
            });
        }
    }

    Ok(changes)
}

fn folder_path_for_object(
    state: &finite_brain_core::portability::BrainWorkingTreeStateManifest,
    object: &WorkingTreeObjectManifestEntry,
) -> Option<String> {
    state
        .folder_roots
        .iter()
        .find(|folder| {
            folder.folder_id == object.folder_id && folder.source_brain_id == object.source_brain_id
        })
        .map(|folder| folder.path.clone())
}

fn collect_working_tree_file_paths(
    root: &Path,
    folder_root: &Path,
) -> Result<Vec<String>, CliError> {
    let mut paths = Vec::new();
    collect_working_tree_file_paths_inner(root, folder_root, &mut paths, 0)?;
    paths.sort();
    Ok(paths)
}

fn collect_working_tree_file_paths_inner(
    root: &Path,
    directory: &Path,
    paths: &mut Vec<String>,
    depth: usize,
) -> Result<(), CliError> {
    if depth > MAX_WORKING_TREE_RECURSION_DEPTH {
        return Err(CliError::InvalidInput(format!(
            "working tree folder depth exceeds limit {MAX_WORKING_TREE_RECURSION_DEPTH}"
        )));
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_working_tree_file_paths_inner(root, &path, paths, depth + 1)?;
        } else if file_type.is_file() {
            if paths.len() >= MAX_WORKING_TREE_FILE_COUNT {
                return Err(CliError::InvalidInput(format!(
                    "working tree file count exceeds limit {MAX_WORKING_TREE_FILE_COUNT}"
                )));
            }
            paths.push(relative_path_string(root, &path)?);
        }
    }
    Ok(())
}

fn is_markdown_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        == Some("md")
}

fn relative_path_string(root: &Path, path: &Path) -> Result<String, CliError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/"))
}

fn is_generated_folder_file(folder_path: &str, relative_path: &str) -> bool {
    let Some(local) = relative_path
        .strip_prefix(folder_path)
        .and_then(|rest| rest.strip_prefix('/'))
    else {
        return true;
    };
    local == "AGENTS.md"
        || local == "_index.md"
        || local.starts_with("_wiki/")
        || local == "raw/.keep"
        || local == "raw/assets/.keep"
        || local == "compiled/.keep"
        || local == "wiki/.keep"
        || local == "inventory/.keep"
        || local == "datasets/.keep"
        || local == "output/.keep"
}

fn is_ignored_os_metadata_file(relative_path: &str) -> bool {
    // macOS Finder drops .DS_Store files into every visited directory. They
    // are never Brain content: silently ignore them instead of reporting a
    // permanent non-Markdown sync conflict.
    Path::new(relative_path)
        .file_name()
        .and_then(|name| name.to_str())
        == Some(".DS_Store")
}

fn remove_stale_object_files(
    root: &Path,
    old_objects: &[WorkingTreeObjectManifestEntry],
    new_objects: &[WorkingTreeObjectManifestEntry],
) -> Result<(), CliError> {
    let new_paths = new_objects
        .iter()
        .map(|object| {
            (
                (
                    object.source_brain_id.clone(),
                    object.folder_id.clone(),
                    object.object_id.clone(),
                ),
                object.path.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for old in old_objects {
        if old.content_type != "text/markdown" {
            continue;
        }
        let key = (
            old.source_brain_id.clone(),
            old.folder_id.clone(),
            old.object_id.clone(),
        );
        let should_remove = match new_paths.get(&key) {
            Some(new_path) => new_path != &old.path,
            None => true,
        };
        if !should_remove {
            continue;
        }
        let Some(folder_path) = folder_path_for_removed_object(root, old)? else {
            continue;
        };
        let path = root.join(folder_path).join(&old.path);
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn remove_obsolete_compiled_convention_markers(
    root: &Path,
    folder_roots: &[WorkingTreeFolderRoot],
) -> Result<(), CliError> {
    for folder in folder_roots.iter().filter(|folder| folder.can_read) {
        let legacy_marker = format!(
            "# compiled\n\nAgent convention directory for Folder `{}`.\n",
            folder.folder_id
        );
        remove_obsolete_compiled_convention_marker(root, &folder.path, legacy_marker.as_bytes())?;
    }
    Ok(())
}

#[cfg(unix)]
fn remove_obsolete_compiled_convention_marker(
    root: &Path,
    folder_path: &str,
    legacy_marker: &[u8],
) -> Result<(), CliError> {
    let Some(compiled_fd) = open_compiled_directory(root, folder_path)? else {
        return Ok(());
    };
    remove_legacy_marker_from_compiled_fd(&compiled_fd, legacy_marker)
}

#[cfg(unix)]
fn open_compiled_directory(root: &Path, folder_path: &str) -> Result<Option<OwnedFd>, CliError> {
    let mut current = match open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(_) => return Ok(None),
    };
    for component in Path::new(folder_path)
        .components()
        .chain([Component::Normal("compiled".as_ref())])
    {
        let Component::Normal(component) = component else {
            return Ok(None);
        };
        current = match openat(
            &current,
            component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(_) => return Ok(None),
        };
    }
    Ok(Some(current))
}

#[cfg(unix)]
fn remove_legacy_marker_from_compiled_fd(
    compiled_fd: &OwnedFd,
    legacy_marker: &[u8],
) -> Result<(), CliError> {
    let Some(quarantine_name) = quarantine_legacy_marker(compiled_fd) else {
        return Ok(());
    };
    verify_and_remove_quarantined_marker(compiled_fd, &quarantine_name, legacy_marker)
}

#[cfg(unix)]
fn quarantine_legacy_marker(compiled_fd: &OwnedFd) -> Option<String> {
    for attempt in 0..128 {
        let quarantine_name = format!(
            ".finitebrain-legacy-compiled-keep-{}-{attempt}",
            std::process::id()
        );
        match renameat_with(
            compiled_fd,
            ".keep",
            compiled_fd,
            quarantine_name.as_str(),
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => return Some(quarantine_name),
            Err(rustix::io::Errno::EXIST) => continue,
            Err(_) => return None,
        }
    }
    None
}

#[cfg(unix)]
fn verify_and_remove_quarantined_marker(
    compiled_fd: &OwnedFd,
    quarantine_name: &str,
    legacy_marker: &[u8],
) -> Result<(), CliError> {
    let marker_fd = match openat(
        compiled_fd,
        quarantine_name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(_) => {
            restore_quarantined_marker(compiled_fd, quarantine_name);
            return Ok(());
        }
    };
    let marker = File::from(marker_fd);
    if !marker.metadata()?.is_file() {
        restore_quarantined_marker(compiled_fd, quarantine_name);
        return Ok(());
    }
    let mut body = Vec::with_capacity(legacy_marker.len().saturating_add(1));
    if let Err(error) = marker
        .take(legacy_marker.len().saturating_add(1) as u64)
        .read_to_end(&mut body)
    {
        restore_quarantined_marker(compiled_fd, quarantine_name);
        return Err(error.into());
    }
    if body == legacy_marker {
        unlinkat(compiled_fd, quarantine_name, AtFlags::empty()).map_err(std::io::Error::from)?;
    } else {
        restore_quarantined_marker(compiled_fd, quarantine_name);
    }
    Ok(())
}

#[cfg(unix)]
fn restore_quarantined_marker(compiled_fd: &OwnedFd, quarantine_name: &str) {
    let _ = renameat_with(
        compiled_fd,
        quarantine_name,
        compiled_fd,
        ".keep",
        RenameFlags::NOREPLACE,
    );
}

#[cfg(not(unix))]
fn remove_obsolete_compiled_convention_marker(
    _root: &Path,
    _folder_path: &str,
    _legacy_marker: &[u8],
) -> Result<(), CliError> {
    // The legacy marker is harmless. Without Unix handle-relative open/unlink
    // semantics, preserving it is safer than traversing a junction or reparse
    // point during cosmetic cleanup.
    Ok(())
}

fn folder_path_for_removed_object(
    root: &Path,
    object: &WorkingTreeObjectManifestEntry,
) -> Result<Option<PathBuf>, CliError> {
    let state = read_working_tree_state(root)?;
    Ok(state
        .folder_roots
        .iter()
        .find(|folder| {
            folder.folder_id == object.folder_id && folder.source_brain_id == object.source_brain_id
        })
        .map(|folder| PathBuf::from(&folder.path)))
}

fn write_projection_files(
    root: &Path,
    files: &BTreeMap<String, String>,
    binary_files: &BTreeMap<String, Vec<u8>>,
) -> Result<(), CliError> {
    for (relative_path, body) in files {
        let path = root.join(relative_path);
        if relative_path == ".finitebrain/working-tree-state.json" {
            let state: BrainWorkingTreeStateManifest = serde_json::from_str(body)?;
            write_working_tree_state(root, &state)?;
        } else if relative_path.starts_with(".finitebrain/") {
            write_private_file_atomic(&path, body.as_bytes())?;
        } else {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, body)?;
        }
    }
    for (relative_path, bytes) in binary_files {
        let path = root.join(relative_path);
        if relative_path.starts_with(".finitebrain/") {
            write_private_file_atomic(&path, bytes)?;
        } else {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, bytes)?;
        }
    }
    Ok(())
}

fn conflict_for_change(
    change: &WorkingTreeChange,
    intent: &WorkingTreeChangeIntent,
    reason: String,
    created_at: String,
) -> ConflictEntry {
    let path = match change {
        WorkingTreeChange::Upsert { path, .. }
        | WorkingTreeChange::UpsertAsset { path, .. }
        | WorkingTreeChange::Delete { path } => Some(path.to_string()),
        WorkingTreeChange::Rename { from_path, to_path } => {
            Some(format!("{from_path} -> {to_path}"))
        }
    };
    let folder_id = intent.folder_id.as_ref().map(ToString::to_string);
    let id = deterministic_id(
        "conflict",
        &[
            folder_id.as_deref().unwrap_or("-"),
            path.as_deref().unwrap_or("-"),
            &reason,
        ],
    );
    ConflictEntry {
        id,
        folder_id,
        path,
        reason,
        state: ConflictState::Open,
        created_at,
        resolved_at: None,
    }
}

fn is_http_conflict(error: &CliError) -> bool {
    matches!(error, CliError::HttpStatus { status: 409, .. })
}

fn mutate_agent_state_at_root<F>(root: &Path, now: String, f: F) -> Result<(), CliError>
where
    F: FnOnce(&mut AgentState, String),
{
    let mut state = read_agent_state(root)?;
    f(&mut state, now);
    write_agent_state(root, &state)
}

#[derive(Debug, Default)]
struct LocalSyncResult {
    pushed_count: usize,
    conflict_count: usize,
    changes: Vec<SyncChangeReport>,
    path_overrides: BTreeMap<(String, String, String), String>,
    conflicted_markdown: BTreeMap<String, String>,
}

#[derive(Debug)]
struct RemoteSyncResult {
    bootstrap: CliSyncBootstrap,
    records: Vec<CliSyncRecord>,
    report_status: String,
    report_reason: Option<String>,
    used_bootstrap: bool,
}

#[derive(Debug)]
struct IncrementalSyncPull {
    latest_sequence: u64,
    records: Vec<CliSyncRecord>,
}

#[derive(Debug)]
struct MountedFolderSyncContext {
    mount: CliMountedFolder,
    export: CliEncryptedBrainExport,
    display_path: String,
}

impl MountedFolderSyncContext {
    fn source_folder(&self) -> Option<&CliExportFolder> {
        self.export
            .folders
            .iter()
            .find(|folder| folder.id == self.mount.source_folder_id)
    }
}

#[derive(Debug)]
struct MountedFolderMaterializeContext {
    mount: CliMountedFolder,
    export: CliEncryptedBrainExport,
    display_path: String,
    bootstrap: CliSyncBootstrap,
}

impl MountedFolderMaterializeContext {
    fn source_folder(&self) -> Option<&CliExportFolder> {
        self.export
            .folders
            .iter()
            .find(|folder| folder.id == self.mount.source_folder_id)
    }
}

enum SubmitIntentOutcome {
    Submitted,
    Conflict(String),
}

fn preserve_conflicted_content(result: &mut LocalSyncResult, change: &WorkingTreeChange) {
    match change {
        WorkingTreeChange::Upsert { path, markdown } => {
            result
                .conflicted_markdown
                .insert(path.to_string(), markdown.clone());
        }
        WorkingTreeChange::UpsertAsset { .. }
        | WorkingTreeChange::Rename { .. }
        | WorkingTreeChange::Delete { .. } => {}
    }
}

pub(crate) fn encode_folder_object_page_plaintext(
    path: &SafeRelativePath,
    markdown: &str,
) -> Result<String, CliError> {
    serde_json::to_string(&CliFolderObjectPagePlaintext {
        version: FOLDER_OBJECT_PAGE_VERSION.to_owned(),
        path: path.as_str().to_owned(),
        markdown: markdown.to_owned(),
    })
    .map_err(CliError::from)
}

pub(crate) fn encode_folder_object_asset_plaintext(
    path: &SafeRelativePath,
    bytes: &[u8],
    content_type: &str,
) -> Result<String, CliError> {
    if bytes.len() > MAX_WORKING_TREE_ASSET_BYTES {
        return Err(CliError::InvalidInput(format!(
            "folder object asset exceeds size limit {MAX_WORKING_TREE_ASSET_BYTES}"
        )));
    }
    let filename = path
        .as_str()
        .rsplit('/')
        .next()
        .unwrap_or(path.as_str())
        .to_owned();
    serde_json::to_string(&CliFolderObjectAssetPlaintext {
        object_type: "asset".to_owned(),
        path: path.as_str().to_owned(),
        filename,
        content_type: content_type.to_owned(),
        size: bytes.len() as u64,
        content_hash: sha256_hex(bytes),
        bytes_base64: BASE64_STANDARD.encode(bytes),
    })
    .map_err(CliError::from)
}

#[cfg(test)]
fn decode_folder_object_page_plaintext(
    plaintext: Vec<u8>,
    fallback_path: String,
) -> Result<(String, String), CliError> {
    match decode_folder_object_plaintext(plaintext, fallback_path)? {
        CliDecodedFolderObjectPlaintext::Page { path, markdown } => Ok((path, markdown)),
        CliDecodedFolderObjectPlaintext::UnsupportedAsset { path, .. } => {
            Err(CliError::InvalidInput(format!(
                "folder object asset plaintext is not a Markdown Page: {path}"
            )))
        }
    }
}

fn decode_folder_object_plaintext(
    plaintext: Vec<u8>,
    fallback_path: String,
) -> Result<CliDecodedFolderObjectPlaintext, CliError> {
    let text =
        String::from_utf8(plaintext).map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(CliDecodedFolderObjectPlaintext::Page {
            path: fallback_path,
            markdown: text,
        });
    };
    if value.get("version").and_then(|version| version.as_str()) == Some(FOLDER_OBJECT_PAGE_VERSION)
    {
        let page: CliFolderObjectPagePlaintext =
            serde_json::from_value(value).map_err(CliError::from)?;
        let page_path = SafeRelativePath::new("page_path", page.path)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        if Path::new(page_path.as_str())
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("md")
        {
            return Err(CliError::InvalidInput(
                "folder object page path must end in .md".to_owned(),
            ));
        }
        return Ok(CliDecodedFolderObjectPlaintext::Page {
            path: page_path.to_string(),
            markdown: page.markdown,
        });
    }
    if value
        .get("type")
        .and_then(|object_type| object_type.as_str())
        == Some("asset")
    {
        let asset_path = value
            .get("path")
            .and_then(|path| path.as_str())
            .ok_or_else(|| {
                CliError::InvalidInput("folder object asset path is missing".to_owned())
            })?;
        let asset_path = SafeRelativePath::new("asset_path", asset_path)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let content_type = value
            .get("contentType")
            .and_then(|content_type| content_type.as_str())
            .unwrap_or("application/octet-stream")
            .to_owned();
        return Ok(CliDecodedFolderObjectPlaintext::UnsupportedAsset {
            path: asset_path.to_string(),
            content_type,
        });
    }
    Ok(CliDecodedFolderObjectPlaintext::Page {
        path: fallback_path,
        markdown: text,
    })
}

enum CliDecodedFolderObjectPlaintext {
    Page { path: String, markdown: String },
    UnsupportedAsset { path: String, content_type: String },
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliFolderObjectPagePlaintext {
    version: String,
    path: String,
    markdown: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliFolderObjectAssetPlaintext {
    #[serde(rename = "type")]
    object_type: String,
    path: String,
    filename: String,
    content_type: String,
    size: u64,
    content_hash: String,
    bytes_base64: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliEncryptedBrainExport {
    pub(crate) brain: CliExportBrain,
    pub(crate) folders: Vec<CliExportFolder>,
    #[serde(default)]
    pub(crate) objects: Vec<CliExportObject>,
    pub(crate) key_grants: Vec<CliFolderKeyGrant>,
    pub(crate) access_state: CliExportAccessState,
    /// Pending grant wraps, present only for key-holding (admin-standing)
    /// requesters; older servers omit the field entirely.
    #[serde(default)]
    pub(crate) pending_wraps: Vec<CliPendingWrap>,
}

/// One pending grant wrap marker from the sync surface: a recipient still
/// waiting for the current Folder Key, wrapped for them.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliPendingWrap {
    pub(crate) folder_id: String,
    pub(crate) recipient_npub: String,
    pub(crate) key_version: u32,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliExportBrain {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) owner_user_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliExportFolder {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) access: String,
    pub(crate) current_key_version: u32,
    pub(crate) accessible: bool,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliExportObject {
    pub(crate) folder_id: String,
    pub(crate) object_id: String,
    pub(crate) payload_json: Option<String>,
    pub(crate) revision: u64,
    pub(crate) updated_at: String,
    pub(crate) deleted: bool,
    pub(crate) opaque: bool,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliFolderKeyGrant {
    folder_id: String,
    key_version: u32,
    issuer_npub: String,
    recipient_npub: String,
    wrapped_event_json: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliExportAccessState {
    pub(crate) members: Vec<String>,
    pub(crate) admins: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliSyncBootstrap {
    latest_sequence: u64,
    objects: Vec<CliSyncObject>,
    #[serde(default)]
    control_records: Vec<CliSyncRecord>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliSyncPull {
    brain_id: String,
    after_sequence: u64,
    latest_sequence: u64,
    records: Vec<CliSyncRecord>,
    count: usize,
    has_more: bool,
    next_sequence: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliSyncRecord {
    sequence: u64,
    record_event_id: String,
    record_type: String,
    folder_id: Option<String>,
    object_id: Option<String>,
    revision: Option<u64>,
    actor_npub: String,
    client_created_at: String,
    payload_json: String,
    record_event_kind: u16,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliSyncObject {
    folder_id: String,
    object_id: String,
    revision: u64,
    ciphertext: String,
    deleted: bool,
}

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliBrainMetadata {
    #[serde(default)]
    personal_agent: Option<CliPersonalAgent>,
    #[serde(default)]
    mounted_folders: Vec<CliMountedFolder>,
    /// Pending grant wraps, present only for key-holding (admin-standing)
    /// requesters; older servers omit the field entirely.
    #[serde(default)]
    pending_wraps: Vec<CliPendingWrap>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliPersonalAgent {
    agent_npub: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliMountedFolder {
    mount_id: String,
    source_brain_id: String,
    source_folder_id: String,
    display_name: String,
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliFolderKeyGrantPlaintext {
    pub(crate) version: String,
    pub(crate) brain_id: String,
    pub(crate) folder_id: String,
    pub(crate) key_version: u32,
    pub(crate) folder_key: String,
    pub(crate) issuer_npub: String,
    pub(crate) recipient_npub: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use finite_brain_core::portability::{
        BrainDirectoryBrainSummary, BrainDirectoryManifest, BrainDirectoryPath,
        BrainDirectoryPortability, BrainWorkingTreeStateManifest, WorkingTreeSyncState,
    };
    use finite_brain_core::{DisplayName, validate_revision_event};
    use tempfile::TempDir;

    #[test]
    fn deleted_folder_cleanup_uses_prior_export_when_manifest_lacks_ancestor_root() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("Parent/Child/raw")).unwrap();
        fs::write(temp.path().join("Parent/Child/raw/.keep"), "generated").unwrap();
        let prior_state = BrainWorkingTreeStateManifest {
            version: "finite-brain-working-tree-state-v1".to_owned(),
            folder_roots: vec![],
            objects: vec![],
            sync: WorkingTreeSyncState { latest_sequence: 1 },
        };
        let prior_export = CliEncryptedBrainExport {
            brain: CliExportBrain {
                id: "brain".to_owned(),
                kind: "organization".to_owned(),
                name: "Brain".to_owned(),
                owner_user_id: None,
            },
            folders: vec![
                CliExportFolder {
                    id: "parent".to_owned(),
                    path: "Parent".to_owned(),
                    access: "restricted".to_owned(),
                    current_key_version: 1,
                    accessible: true,
                },
                CliExportFolder {
                    id: "child".to_owned(),
                    path: "Parent/Child".to_owned(),
                    access: "restricted".to_owned(),
                    current_key_version: 1,
                    accessible: true,
                },
            ],
            objects: vec![],
            key_grants: vec![],
            access_state: CliExportAccessState {
                members: vec![],
                admins: vec![],
            },

            pending_wraps: Vec::new(),
        };
        let deleted_routes = BTreeSet::from([
            ("brain".to_owned(), "parent".to_owned()),
            ("brain".to_owned(), "child".to_owned()),
        ]);

        remove_deleted_folder_roots(
            temp.path(),
            &prior_state,
            Some(&prior_export),
            &deleted_routes,
            "brain",
        )
        .unwrap();

        assert!(!temp.path().join("Parent").exists());
    }

    #[test]
    fn deleted_logical_root_folder_never_removes_working_tree() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".finitebrain")).unwrap();
        fs::write(temp.path().join(".finitebrain/agent-state.json"), "state").unwrap();
        let prior_state = BrainWorkingTreeStateManifest {
            version: "finite-brain-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "root-folder".to_owned(),
                source_brain_id: None,
                path: ".".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: vec![],
            sync: WorkingTreeSyncState { latest_sequence: 1 },
        };

        remove_deleted_folder_roots(
            temp.path(),
            &prior_state,
            None,
            &BTreeSet::from([("brain".to_owned(), "root-folder".to_owned())]),
            "brain",
        )
        .unwrap();

        assert!(temp.path().is_dir());
        assert!(temp.path().join(".finitebrain/agent-state.json").is_file());
    }

    #[test]
    fn scan_detects_markdown_create_update_and_delete() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("General/_wiki")).unwrap();
        fs::write(root.join("General/existing.md"), "# Changed\n").unwrap();
        fs::write(root.join("General/new.md"), "# New\n").unwrap();
        fs::write(root.join("General/AGENTS.md"), "# Generated\n").unwrap();
        fs::write(root.join("General/_wiki/index.md"), "# Generated\n").unwrap();
        let state = BrainWorkingTreeStateManifest {
            version: "finite-brain-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "general".to_owned(),
                source_brain_id: None,
                path: "General".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: vec![
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "existing.md".to_owned(),
                    object_id: "obj_existing00000".to_owned(),
                    revision: 1,
                    key_version: 1,
                    content_type: "text/markdown".to_owned(),
                    content_hash: sha256_hex("# Old\n".as_bytes()),
                },
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "deleted.md".to_owned(),
                    object_id: "obj_deleted000000".to_owned(),
                    revision: 1,
                    key_version: 1,
                    content_type: "text/markdown".to_owned(),
                    content_hash: sha256_hex("# Deleted\n".as_bytes()),
                },
            ],
            sync: WorkingTreeSyncState { latest_sequence: 1 },
        };

        let changes = scan_working_tree_changes(root, &state).unwrap();

        assert_eq!(changes.len(), 3);
        assert!(changes.iter().any(|change| matches!(
            change,
            WorkingTreeChange::Upsert { path, markdown }
                if path.as_str() == "General/existing.md" && markdown == "# Changed\n"
        )));
        assert!(changes.iter().any(|change| matches!(
            change,
            WorkingTreeChange::Upsert { path, markdown }
                if path.as_str() == "General/new.md" && markdown == "# New\n"
        )));
        assert!(changes.iter().any(|change| matches!(
            change,
            WorkingTreeChange::Delete { path } if path.as_str() == "General/deleted.md"
        )));
    }

    #[test]
    fn scan_ignores_ds_store_files() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("General/wiki")).unwrap();
        fs::write(root.join("General/existing.md"), "# Same\n").unwrap();
        fs::write(root.join("General/.DS_Store"), b"finder-metadata").unwrap();
        fs::write(root.join("General/wiki/.DS_Store"), b"finder-metadata").unwrap();
        let state = BrainWorkingTreeStateManifest {
            version: "finite-brain-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "general".to_owned(),
                source_brain_id: None,
                path: "General".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: vec![WorkingTreeObjectManifestEntry {
                folder_id: "general".to_owned(),
                source_brain_id: None,
                path: "existing.md".to_owned(),
                object_id: "obj_existing00000".to_owned(),
                revision: 1,
                key_version: 1,
                content_type: "text/markdown".to_owned(),
                content_hash: sha256_hex("# Same\n".as_bytes()),
            }],
            sync: WorkingTreeSyncState { latest_sequence: 1 },
        };

        let changes = scan_working_tree_changes(root, &state).unwrap();

        assert!(changes.is_empty(), "unexpected changes: {changes:?}");
    }

    #[test]
    fn scan_reports_new_non_markdown_files_without_reopening_preserved_legacy_assets() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("General/raw/assets")).unwrap();
        let source_note =
            "# Source Notes\n\n- Existing: raw/assets/existing.pdf\n- New: raw/assets/new.pdf\n";
        fs::write(root.join("General/raw/source-note.md"), source_note).unwrap();
        fs::write(root.join("General/raw/assets/existing.pdf"), b"changed-pdf").unwrap();
        fs::write(root.join("General/raw/assets/new.pdf"), b"new-pdf").unwrap();
        fs::write(
            root.join("General/raw/assets/missing-note.pdf"),
            b"missing-note",
        )
        .unwrap();
        fs::write(root.join("General/stray.bin"), b"stray").unwrap();
        fs::write(root.join("General/raw/assets/.keep"), "# generated\n").unwrap();
        let state = BrainWorkingTreeStateManifest {
            version: "finite-brain-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "general".to_owned(),
                source_brain_id: None,
                path: "General".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: vec![
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "raw/source-note.md".to_owned(),
                    object_id: "obj_sourcenote000".to_owned(),
                    revision: 1,
                    key_version: 1,
                    content_type: "text/markdown".to_owned(),
                    content_hash: sha256_hex(source_note.as_bytes()),
                },
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "raw/assets/existing.pdf".to_owned(),
                    object_id: "obj_assetexisting".to_owned(),
                    revision: 2,
                    key_version: 1,
                    content_type: "application/pdf".to_owned(),
                    content_hash: sha256_hex(b"old-pdf"),
                },
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "raw/assets/missing-note.pdf".to_owned(),
                    object_id: "obj_missingnote00".to_owned(),
                    revision: 1,
                    key_version: 1,
                    content_type: "application/pdf".to_owned(),
                    content_hash: sha256_hex(b"missing-note"),
                },
            ],
            sync: WorkingTreeSyncState { latest_sequence: 1 },
        };

        let changes = scan_working_tree_changes(root, &state).unwrap();

        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|change| matches!(
            change,
            WorkingTreeChange::UpsertAsset {
                bytes,
                content_type,
                has_source_note,
                ..
            } if bytes.is_empty()
                && content_type == "application/octet-stream"
                && !has_source_note
        )));
        let intents = plan_working_tree_change_intents(&state, &changes);
        let by_path = changes
            .iter()
            .zip(intents.iter())
            .map(|(change, intent)| {
                let path = match change {
                    WorkingTreeChange::UpsertAsset { path, .. } => path.to_string(),
                    other => panic!("unexpected change in asset scan test: {other:?}"),
                };
                (path, intent)
            })
            .collect::<BTreeMap<_, _>>();

        assert_eq!(by_path.len(), 2);
        assert!(by_path.contains_key("General/raw/assets/new.pdf"));
        assert!(by_path.contains_key("General/stray.bin"));
        assert!(by_path.values().all(|intent| matches!(
            intent,
            WorkingTreeChangeIntent {
                action: WorkingTreeIntentAction::Unresolved,
                route: WorkingTreeIntentRoute::Unresolved,
                content: None,
                reason: Some(reason),
                ..
            } if reason.contains("Asset Source Note")
        )));
    }

    #[test]
    fn scan_does_not_treat_a_markdown_mention_as_permission_to_upload_binary_bytes() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("General/raw/assets")).unwrap();
        fs::write(
            root.join("General/raw/source-note.md"),
            "# Source Notes\n\n- Almost: raw/assets/file.pdf.bak\n",
        )
        .unwrap();
        fs::write(root.join("General/raw/assets/file.pdf"), b"asset").unwrap();
        let state = BrainWorkingTreeStateManifest {
            version: "finite-brain-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "general".to_owned(),
                source_brain_id: None,
                path: "General".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: Vec::new(),
            sync: WorkingTreeSyncState { latest_sequence: 1 },
        };

        let changes = scan_working_tree_changes(root, &state).unwrap();
        let intents = plan_working_tree_change_intents(&state, &changes);
        let asset_intent = changes
            .iter()
            .zip(intents.iter())
            .find_map(|(change, intent)| {
                matches!(
                    change,
                    WorkingTreeChange::UpsertAsset { path, .. }
                        if path.as_str() == "General/raw/assets/file.pdf"
                )
                .then_some(intent)
            })
            .unwrap();

        assert!(matches!(
            asset_intent,
            WorkingTreeChangeIntent {
                action: WorkingTreeIntentAction::Unresolved,
                reason: Some(reason),
                ..
            } if reason.contains("Asset Source Note")
        ));
    }

    #[test]
    fn scan_reports_oversized_non_markdown_files_without_reading_them() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("General/raw/assets")).unwrap();
        fs::write(
            root.join("General/raw/source-note.md"),
            "# Source Notes\n\n- Huge: raw/assets/huge.bin\n",
        )
        .unwrap();
        let huge = fs::File::create(root.join("General/raw/assets/huge.bin")).unwrap();
        huge.set_len((MAX_WORKING_TREE_ASSET_BYTES + 1) as u64)
            .unwrap();
        let state = BrainWorkingTreeStateManifest {
            version: "finite-brain-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "general".to_owned(),
                source_brain_id: None,
                path: "General".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: Vec::new(),
            sync: WorkingTreeSyncState { latest_sequence: 1 },
        };

        let changes = scan_working_tree_changes(root, &state).unwrap();
        let asset = changes
            .iter()
            .find(|change| matches!(change, WorkingTreeChange::UpsertAsset { .. }))
            .unwrap();
        assert!(matches!(
            asset,
            WorkingTreeChange::UpsertAsset { bytes, .. } if bytes.is_empty()
        ));
        let intent = plan_working_tree_change_intents(&state, std::slice::from_ref(asset))
            .pop()
            .unwrap();
        assert_eq!(intent.action, WorkingTreeIntentAction::Unresolved);
        assert!(intent.content.is_none());
    }

    #[test]
    fn signed_revision_events_validate_against_core_contract() {
        let keys = Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")
            .unwrap();
        let actor_npub = NostrPublicKey::from_protocol(keys.public_key())
            .to_npub()
            .unwrap();
        let folder_key = FolderKey::from_bytes([7; 32]);
        let aad = FolderObjectAad {
            brain_id: BrainId::new("brain").unwrap(),
            folder_id: FolderId::new("general").unwrap(),
            object_id: ObjectId::new("obj_000000000001").unwrap(),
            key_version: 1,
        };
        let envelope = encrypt_folder_object(&folder_key, &aad, "# Page\n").unwrap();
        let envelope_json = envelope.canonical_json();
        let event_json = signed_revision_event(
            &keys,
            RevisionEventInput {
                actor_npub: &actor_npub,
                brain_id: "brain",
                folder_id: &FolderId::new("general").unwrap(),
                object_id: &ObjectId::new("obj_000000000001").unwrap(),
                operation: FolderObjectOperation::Create,
                base_revision: None,
                key_version: 1,
                envelope_json: envelope_json.clone(),
            },
        )
        .unwrap();
        let event = Event::from_json(event_json.to_string()).unwrap();
        let expected = RevisionValidation {
            brain_id: BrainId::new("brain").unwrap(),
            folder_id: FolderId::new("general").unwrap(),
            object_id: ObjectId::new("obj_000000000001").unwrap(),
            operation: FolderObjectOperation::Create,
            revision: 1,
            base_revision: None,
            key_version: 1,
            envelope_json,
            author_npub: actor_npub,
            created_at: timestamp_from_unix(event.created_at.as_secs()),
        };

        validate_revision_event(&event, &expected).unwrap();
    }

    #[test]
    fn submit_change_intent_conflicts_without_current_folder_key() {
        let temp = TempDir::new().unwrap();
        let env = CliEnvironment {
            cwd: temp.path().to_path_buf(),
            config_dir: temp.path().join("config"),
            server_url: None,
            public_base_url: None,
            working_tree_root: None,
            now: Some("2026-06-26T23:30:00Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(temp.path().join("finite-home")),
            embedding_provider: None,
        };
        let keys = Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")
            .unwrap();
        let actor_npub = NostrPublicKey::from_protocol(keys.public_key())
            .to_npub()
            .unwrap();
        let agent_state = AgentState::new("brain", "2026-06-26T23:30:00Z");
        let mut session_keys = SessionFolderKeyring::default();
        session_keys.insert("brain", "general", 1, FolderKey::from_bytes([1; 32]));
        let current_key_version_by_folder =
            BTreeMap::from([(("brain".to_owned(), "general".to_owned()), 2)]);
        let context = SubmitIntentContext {
            env: &env,
            server_url: "http://127.0.0.1:9",
            agent_state: &agent_state,
            signing_keys: &keys,
            actor_npub: &actor_npub,
            session_keys: &session_keys,
            current_key_version_by_folder: &current_key_version_by_folder,
        };
        let intent = WorkingTreeChangeIntent {
            action: WorkingTreeIntentAction::Create,
            route: WorkingTreeIntentRoute::EncryptedObjectWrite,
            folder_id: Some(FolderId::new("general").unwrap()),
            source_brain_id: None,
            object_id: Some(ObjectId::new("obj_currentkey01").unwrap()),
            target_path: Some(SafeRelativePath::new("page_path", "page.md").unwrap()),
            from_path: None,
            base_revision: None,
            content: Some(WorkingTreeIntentContent::PageMarkdown(
                "# Page\n".to_owned(),
            )),
            reason: None,
        };

        let outcome = submit_change_intent(&context, &intent).unwrap();

        assert!(matches!(
            outcome,
            SubmitIntentOutcome::Conflict(reason)
                if reason.contains("current Folder Key v2 unavailable")
        ));
    }

    #[test]
    fn encrypted_page_plaintext_requires_markdown_path() {
        let path = SafeRelativePath::new("page_path", "notes/page.txt").unwrap();
        let plaintext = encode_folder_object_page_plaintext(&path, "# Page\n").unwrap();

        let error =
            decode_folder_object_page_plaintext(plaintext.into_bytes(), "fallback.md".to_owned())
                .unwrap_err();

        assert!(error.to_string().contains("must end in .md"));
    }

    #[test]
    fn legacy_asset_plaintext_is_classified_without_decoding_inline_bytes() {
        let path = SafeRelativePath::new("asset_path", "raw/assets/source.pdf").unwrap();
        let encoded =
            encode_folder_object_asset_plaintext(&path, b"%PDF test\n", "application/pdf").unwrap();
        let mut plaintext: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        plaintext["bytesBase64"] = serde_json::Value::String("not-valid-base64".to_owned());

        match decode_folder_object_plaintext(
            serde_json::to_vec(&plaintext).unwrap(),
            "fallback.md".to_owned(),
        )
        .unwrap()
        {
            CliDecodedFolderObjectPlaintext::UnsupportedAsset { path, content_type } => {
                assert_eq!(path, "raw/assets/source.pdf");
                assert_eq!(content_type, "application/pdf");
            }
            CliDecodedFolderObjectPlaintext::Page { .. } => {
                panic!("expected unsupported asset plaintext")
            }
        }
    }

    #[test]
    fn empty_readable_folders_stay_materialized() {
        let brain = Brain {
            id: BrainId::new("brain").unwrap(),
            kind: BrainKind::Personal,
            name: DisplayName::new("brain_name", "Brain").unwrap(),
            owner_user_id: Some(UserId::new("npub-owner").unwrap()),
            folders: vec![Folder {
                id: FolderId::new("home").unwrap(),
                name: DisplayName::new("folder_name", "home").unwrap(),
                role: FolderRole::PersonalHome,
                access: FolderAccessMode::Owner,
                parent_folder_id: None,
                path: SafeRelativePath::new("folder_path", "home").unwrap(),
                current_key_version: 1,
            }],
            members: Vec::new(),
            admins: Vec::new(),
        };
        let mut projection = materialize_brain_working_tree(WorkingTreeMaterializeInput {
            generated_at: "2026-06-26T23:30:00Z".to_owned(),
            generated_by_npub: UserId::new("npub-owner").unwrap(),
            acting_role: "owner".to_owned(),
            brain,
            opened_pages: Vec::new(),
            opened_assets: Vec::new(),
            locked_folders: Vec::new(),
            latest_sequence: 0,
        })
        .unwrap();
        let export = CliEncryptedBrainExport {
            brain: CliExportBrain {
                id: "brain".to_owned(),
                kind: "personal".to_owned(),
                name: "Brain".to_owned(),
                owner_user_id: Some("npub-owner".to_owned()),
            },
            folders: vec![CliExportFolder {
                id: "home".to_owned(),
                path: "home".to_owned(),
                access: "owner".to_owned(),
                current_key_version: 1,
                accessible: true,
            }],
            objects: Vec::new(),
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },

            pending_wraps: Vec::new(),
        };
        let readable = BTreeSet::from([("brain".to_owned(), "home".to_owned())]);

        add_empty_readable_folders(&mut projection, &export, None, &readable, None).unwrap();

        assert_eq!(projection.state.folder_roots.len(), 1);
        assert_eq!(projection.state.folder_roots[0].folder_id, "home");
        assert!(projection.files.contains_key("home/AGENTS.md"));
        assert!(projection.files.contains_key("home/raw/.keep"));
        assert!(!projection.files.contains_key("home/raw/assets/.keep"));
        assert!(projection.files.contains_key("home/wiki/.keep"));
        assert!(projection.files.contains_key("home/inventory/.keep"));
        assert!(projection.files.contains_key("home/datasets/.keep"));
        assert!(
            projection
                .files
                .get("home/AGENTS.md")
                .unwrap()
                .contains("Source Note")
        );
        assert!(
            projection
                .files
                .get("home/AGENTS.md")
                .unwrap()
                .contains("wiki/")
        );
    }

    #[test]
    fn working_tree_role_uses_authoritative_brain_and_personal_agent_metadata() {
        let mut brain = Brain {
            id: BrainId::new("brain").unwrap(),
            kind: BrainKind::Personal,
            name: DisplayName::new("brain_name", "Brain").unwrap(),
            owner_user_id: Some(UserId::new("npub-owner").unwrap()),
            folders: Vec::new(),
            members: Vec::new(),
            admins: Vec::new(),
        };
        let metadata = CliBrainMetadata {
            personal_agent: Some(CliPersonalAgent {
                agent_npub: "npub-agent".to_owned(),
            }),
            mounted_folders: Vec::new(),
            pending_wraps: Vec::new(),
        };
        assert_eq!(
            brain_role_for_actor(&brain, &metadata, "npub-owner"),
            "owner"
        );
        assert_eq!(
            brain_role_for_actor(&brain, &metadata, "npub-agent"),
            "personal_agent"
        );
        brain.members = vec![finite_brain_core::BrainMember {
            user_id: UserId::new("npub-personal-member").unwrap(),
            folder_access: BTreeSet::new(),
        }];
        assert_eq!(
            brain_role_for_actor(&brain, &metadata, "npub-personal-member"),
            "member"
        );
        assert_eq!(
            brain_role_for_actor(&brain, &metadata, "npub-guest"),
            "guest"
        );

        brain.kind = BrainKind::Organization;
        brain.owner_user_id = None;
        brain.admins = vec![UserId::new("npub-admin").unwrap()];
        brain.members = vec![finite_brain_core::BrainMember {
            user_id: UserId::new("npub-member").unwrap(),
            folder_access: BTreeSet::new(),
        }];
        assert_eq!(
            brain_role_for_actor(&brain, &metadata, "npub-admin"),
            "admin"
        );
        assert_eq!(
            brain_role_for_actor(&brain, &metadata, "npub-member"),
            "member"
        );
    }

    #[test]
    fn stale_object_cleanup_removes_old_path_after_move() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        fs::create_dir_all(root.join("General")).unwrap();
        fs::write(root.join("General/old.md"), "# Old\n").unwrap();
        fs::write(root.join("General/legacy.pdf"), b"legacy bytes").unwrap();
        let state = BrainWorkingTreeStateManifest {
            version: "finite-brain-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "general".to_owned(),
                source_brain_id: None,
                path: "General".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: vec![
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "old.md".to_owned(),
                    object_id: "obj_same0000000".to_owned(),
                    revision: 1,
                    key_version: 1,
                    content_type: "text/markdown".to_owned(),
                    content_hash: sha256_hex("# Old\n".as_bytes()),
                },
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "legacy.pdf".to_owned(),
                    object_id: "obj_legacy000001".to_owned(),
                    revision: 1,
                    key_version: 1,
                    content_type: "application/pdf".to_owned(),
                    content_hash: sha256_hex(b"legacy bytes"),
                },
            ],
            sync: WorkingTreeSyncState { latest_sequence: 1 },
        };
        write_json_file(&root.join(".finitebrain/working-tree-state.json"), &state).unwrap();
        let new_objects = vec![WorkingTreeObjectManifestEntry {
            folder_id: "general".to_owned(),
            source_brain_id: None,
            path: "new.md".to_owned(),
            object_id: "obj_same0000000".to_owned(),
            revision: 2,
            key_version: 1,
            content_type: "text/markdown".to_owned(),
            content_hash: sha256_hex("# New\n".as_bytes()),
        }];

        remove_stale_object_files(root, &state.objects, &new_objects).unwrap();

        assert!(!root.join("General/old.md").exists());
        assert_eq!(
            fs::read(root.join("General/legacy.pdf")).unwrap(),
            b"legacy bytes"
        );
    }

    #[test]
    fn incremental_page_update_keeps_cached_legacy_asset_ciphertext_byte_for_byte() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        let legacy_ciphertext = "{\"legacy\":\"ciphertext-must-not-change\"}";
        write_json_file(
            &root.join(".finitebrain/encrypted-sync/bootstrap.json"),
            &CliSyncBootstrap {
                latest_sequence: 4,
                objects: vec![
                    CliSyncObject {
                        folder_id: "home".to_owned(),
                        object_id: "obj_page00000001".to_owned(),
                        revision: 1,
                        ciphertext: "old-page-ciphertext".to_owned(),
                        deleted: false,
                    },
                    CliSyncObject {
                        folder_id: "home".to_owned(),
                        object_id: "obj_legacyasset03".to_owned(),
                        revision: 1,
                        ciphertext: legacy_ciphertext.to_owned(),
                        deleted: false,
                    },
                ],
                control_records: Vec::new(),
            },
        )
        .unwrap();
        let records = vec![CliSyncRecord {
            sequence: 5,
            record_event_id: "event-page-update".to_owned(),
            record_type: "folder_object_revision".to_owned(),
            folder_id: Some("home".to_owned()),
            object_id: Some("obj_page00000001".to_owned()),
            revision: Some(2),
            actor_npub: "npub-actor".to_owned(),
            client_created_at: "2026-08-04T00:00:00Z".to_owned(),
            payload_json: serde_json::json!({
                "ciphertext": "new-page-ciphertext"
            })
            .to_string(),
            record_event_kind: 30_101,
        }];

        let incremental = apply_incremental_records(root, 4, 5, &records).unwrap();

        assert_eq!(incremental.latest_sequence, 5);
        assert_eq!(
            incremental
                .objects
                .iter()
                .find(|object| object.object_id == "obj_legacyasset03")
                .unwrap()
                .ciphertext,
            legacy_ciphertext
        );
        assert_eq!(
            incremental
                .objects
                .iter()
                .find(|object| object.object_id == "obj_page00000001")
                .unwrap()
                .ciphertext,
            "new-page-ciphertext"
        );
    }

    #[test]
    fn incremental_revision_keeps_legacy_bare_ciphertext_payload_byte_for_byte() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        write_json_file(
            &root.join(".finitebrain/encrypted-sync/bootstrap.json"),
            &CliSyncBootstrap {
                latest_sequence: 0,
                objects: Vec::new(),
                control_records: Vec::new(),
            },
        )
        .unwrap();
        // A legacy record whose payload_json is the bare ciphertext string, not JSON.
        let records = vec![CliSyncRecord {
            sequence: 1,
            record_event_id: "event-legacy-asset".to_owned(),
            record_type: "folder_object_revision".to_owned(),
            folder_id: Some("home".to_owned()),
            object_id: Some("obj_legacyasset04".to_owned()),
            revision: Some(1),
            actor_npub: "npub-actor".to_owned(),
            client_created_at: "2026-08-04T00:00:00Z".to_owned(),
            payload_json: "legacy-bare-ciphertext".to_owned(),
            record_event_kind: 30_101,
        }];

        let incremental = apply_incremental_records(root, 0, 1, &records).unwrap();

        assert_eq!(
            incremental
                .objects
                .iter()
                .find(|object| object.object_id == "obj_legacyasset04")
                .unwrap()
                .ciphertext,
            "legacy-bare-ciphertext"
        );
    }

    #[test]
    fn incremental_folder_key_grant_record_joins_control_records_without_rebootstrap() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        write_json_file(
            &root.join(".finitebrain/encrypted-sync/bootstrap.json"),
            &CliSyncBootstrap {
                latest_sequence: 4,
                objects: vec![CliSyncObject {
                    folder_id: "home".to_owned(),
                    object_id: "obj_page00000001".to_owned(),
                    revision: 1,
                    ciphertext: "page-ciphertext".to_owned(),
                    deleted: false,
                }],
                control_records: Vec::new(),
            },
        )
        .unwrap();
        let records = vec![CliSyncRecord {
            sequence: 5,
            record_event_id: "event-folder-grant".to_owned(),
            record_type: "folder_key_grant".to_owned(),
            folder_id: Some("home".to_owned()),
            object_id: None,
            revision: None,
            actor_npub: "npub-admin".to_owned(),
            client_created_at: "2026-08-04T00:00:00Z".to_owned(),
            payload_json: serde_json::json!({
                "folderId": "home",
                "keyVersion": 1,
                "issuerNpub": "npub-admin",
                "recipientNpub": "npub-member",
                "wrappedEventJson": "{}"
            })
            .to_string(),
            record_event_kind: 30_101,
        }];

        let incremental = apply_incremental_records(root, 4, 5, &records).unwrap();

        assert_eq!(incremental.latest_sequence, 5);
        assert_eq!(incremental.objects.len(), 1);
        assert_eq!(incremental.control_records.len(), 1);
        assert_eq!(
            incremental.control_records[0].record_type,
            "folder_key_grant"
        );
    }

    #[test]
    fn materialize_remote_projection_opens_pages_and_preserves_legacy_asset_records() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        fs::create_dir_all(root.join("home/raw/assets")).unwrap();
        let prior_local_asset = b"prior local bytes stay untouched";
        fs::write(root.join("home/raw/assets/source.pdf"), prior_local_asset).unwrap();
        write_json_file(
            &root.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: "finite-brain-working-tree-state-v1".to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "home".to_owned(),
                    source_brain_id: None,
                    path: "home".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: vec![WorkingTreeObjectManifestEntry {
                    folder_id: "home".to_owned(),
                    source_brain_id: None,
                    path: "raw/assets/source.pdf".to_owned(),
                    object_id: "obj_legacyasset01".to_owned(),
                    revision: 1,
                    key_version: 1,
                    content_type: "application/pdf".to_owned(),
                    content_hash: sha256_hex(prior_local_asset),
                }],
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        let folder_key = FolderKey::from_bytes([3; 32]);
        let mut session_keys = SessionFolderKeyring::default();
        session_keys.insert("brain", "home", 1, folder_key.clone());
        let env = CliEnvironment {
            cwd: root.to_path_buf(),
            config_dir: root.join("config"),
            server_url: None,
            public_base_url: None,
            working_tree_root: None,
            now: Some("2026-06-26T23:30:00Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(root.join("finite-home")),
            embedding_provider: None,
        };
        let object_id = ObjectId::new("obj_remote000001").unwrap();
        let page_path = SafeRelativePath::new("page_path", "docs/from-envelope.md").unwrap();
        let plaintext = encode_folder_object_page_plaintext(&page_path, "# Remote\n").unwrap();
        let aad = FolderObjectAad {
            brain_id: BrainId::new("brain").unwrap(),
            folder_id: FolderId::new("home").unwrap(),
            object_id: object_id.clone(),
            key_version: 1,
        };
        let envelope = encrypt_folder_object(&folder_key, &aad, &plaintext).unwrap();
        let asset_object_id = ObjectId::new("obj_legacyasset01").unwrap();
        let asset_path = SafeRelativePath::new("asset_path", "raw/assets/source.pdf").unwrap();
        let asset_plaintext = encode_folder_object_asset_plaintext(
            &asset_path,
            b"server asset bytes",
            "application/pdf",
        )
        .unwrap();
        let asset_aad = FolderObjectAad {
            brain_id: BrainId::new("brain").unwrap(),
            folder_id: FolderId::new("home").unwrap(),
            object_id: asset_object_id.clone(),
            key_version: 1,
        };
        let asset_envelope =
            encrypt_folder_object(&folder_key, &asset_aad, &asset_plaintext).unwrap();
        let export = CliEncryptedBrainExport {
            brain: CliExportBrain {
                id: "brain".to_owned(),
                kind: "personal".to_owned(),
                name: "Brain".to_owned(),
                owner_user_id: Some("npub-owner".to_owned()),
            },
            folders: vec![CliExportFolder {
                id: "home".to_owned(),
                path: "home".to_owned(),
                access: "owner".to_owned(),
                current_key_version: 1,
                accessible: true,
            }],
            objects: Vec::new(),
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },

            pending_wraps: Vec::new(),
        };
        let bootstrap = CliSyncBootstrap {
            latest_sequence: 7,
            objects: vec![
                CliSyncObject {
                    folder_id: "home".to_owned(),
                    object_id: object_id.as_str().to_owned(),
                    revision: 2,
                    ciphertext: envelope.canonical_json(),
                    deleted: false,
                },
                CliSyncObject {
                    folder_id: "home".to_owned(),
                    object_id: asset_object_id.as_str().to_owned(),
                    revision: 3,
                    ciphertext: asset_envelope.canonical_json(),
                    deleted: false,
                },
            ],
            control_records: Vec::new(),
        };

        let unsupported = materialize_remote_projection(MaterializeRemoteProjectionContext {
            env: &env,
            root,
            actor_npub: "npub-owner",
            metadata: Some(&CliBrainMetadata::default()),
            export: &export,
            bootstrap: &bootstrap,
            mounted_folders: &[],
            path_overrides: &BTreeMap::new(),
            session_keys: &session_keys,
            prior_state: None,
        })
        .unwrap();

        assert_eq!(
            fs::read_to_string(root.join("home/docs/from-envelope.md")).unwrap(),
            "# Remote\n"
        );
        let state = read_working_tree_state(root).unwrap();
        assert_eq!(state.objects.len(), 2);
        assert!(
            state
                .objects
                .iter()
                .any(|object| object.path == "docs/from-envelope.md")
        );
        let preserved = state
            .objects
            .iter()
            .find(|object| object.object_id == asset_object_id.as_str())
            .unwrap();
        assert_eq!(preserved.revision, 3);
        assert_eq!(
            fs::read(root.join("home/raw/assets/source.pdf")).unwrap(),
            prior_local_asset
        );
        assert_eq!(unsupported.len(), 1);
        assert_eq!(unsupported[0].status, "unsupported");
        assert_eq!(unsupported[0].action, "preserve");
        assert_eq!(
            unsupported[0].object_id.as_deref(),
            Some("obj_legacyasset01")
        );
        assert!(
            unsupported[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("bytes were not materialized")
        );
        assert_eq!(state.sync.latest_sequence, 7);
        assert!(
            scan_working_tree_changes(root, &state)
                .unwrap()
                .iter()
                .all(|change| !matches!(change, WorkingTreeChange::UpsertAsset { .. })),
            "a preserved legacy Asset must not become a perpetual local conflict"
        );

        let legacy_ciphertext_before = bootstrap.objects[1].ciphertext.clone();
        let rebootstrap_unsupported =
            materialize_remote_projection(MaterializeRemoteProjectionContext {
                env: &env,
                root,
                actor_npub: "npub-owner",
                metadata: Some(&CliBrainMetadata::default()),
                export: &export,
                bootstrap: &bootstrap,
                mounted_folders: &[],
                path_overrides: &BTreeMap::new(),
                session_keys: &session_keys,
                prior_state: None,
            })
            .unwrap();
        assert_eq!(rebootstrap_unsupported.len(), 1);
        assert_eq!(bootstrap.objects[1].ciphertext, legacy_ciphertext_before);
        assert_eq!(
            fs::read(root.join("home/raw/assets/source.pdf")).unwrap(),
            prior_local_asset
        );
    }

    #[test]
    fn preserved_legacy_asset_path_collision_fails_before_overwriting_local_bytes() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        fs::create_dir_all(root.join("home/docs")).unwrap();
        let prior_local_asset = b"only local legacy bytes";
        fs::write(root.join("home/docs/collision.md"), prior_local_asset).unwrap();
        let prior_state = BrainWorkingTreeStateManifest {
            version: "finite-brain-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "home".to_owned(),
                source_brain_id: None,
                path: "home".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: vec![WorkingTreeObjectManifestEntry {
                folder_id: "home".to_owned(),
                source_brain_id: None,
                path: "docs/collision.md".to_owned(),
                object_id: "obj_legacyasset02".to_owned(),
                revision: 1,
                key_version: 1,
                content_type: "application/octet-stream".to_owned(),
                content_hash: sha256_hex(prior_local_asset),
            }],
            sync: WorkingTreeSyncState { latest_sequence: 0 },
        };
        write_json_file(
            &root.join(".finitebrain/working-tree-state.json"),
            &prior_state,
        )
        .unwrap();
        let folder_key = FolderKey::from_bytes([5; 32]);
        let mut session_keys = SessionFolderKeyring::default();
        session_keys.insert("brain", "home", 1, folder_key.clone());
        let env = CliEnvironment {
            cwd: root.to_path_buf(),
            config_dir: root.join("config"),
            server_url: None,
            public_base_url: None,
            working_tree_root: None,
            now: Some("2026-06-26T23:30:00Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(root.join("finite-home")),
            embedding_provider: None,
        };
        let page_id = ObjectId::new("obj_pagecollision").unwrap();
        let page_path = SafeRelativePath::new("page_path", "docs/collision.md").unwrap();
        let page_plaintext =
            encode_folder_object_page_plaintext(&page_path, "# Must not overwrite\n").unwrap();
        let page_aad = FolderObjectAad {
            brain_id: BrainId::new("brain").unwrap(),
            folder_id: FolderId::new("home").unwrap(),
            object_id: page_id.clone(),
            key_version: 1,
        };
        let page_envelope = encrypt_folder_object(&folder_key, &page_aad, &page_plaintext).unwrap();
        let asset_id = ObjectId::new("obj_legacyasset02").unwrap();
        let asset_path = SafeRelativePath::new("asset_path", "docs/collision.md").unwrap();
        let asset_plaintext = encode_folder_object_asset_plaintext(
            &asset_path,
            b"server legacy bytes",
            "application/octet-stream",
        )
        .unwrap();
        let asset_aad = FolderObjectAad {
            brain_id: BrainId::new("brain").unwrap(),
            folder_id: FolderId::new("home").unwrap(),
            object_id: asset_id.clone(),
            key_version: 1,
        };
        let asset_envelope =
            encrypt_folder_object(&folder_key, &asset_aad, &asset_plaintext).unwrap();
        let export = CliEncryptedBrainExport {
            brain: CliExportBrain {
                id: "brain".to_owned(),
                kind: "personal".to_owned(),
                name: "Brain".to_owned(),
                owner_user_id: Some("npub-owner".to_owned()),
            },
            folders: vec![CliExportFolder {
                id: "home".to_owned(),
                path: "home".to_owned(),
                access: "owner".to_owned(),
                current_key_version: 1,
                accessible: true,
            }],
            objects: Vec::new(),
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },

            pending_wraps: Vec::new(),
        };
        let bootstrap = CliSyncBootstrap {
            latest_sequence: 2,
            objects: vec![
                CliSyncObject {
                    folder_id: "home".to_owned(),
                    object_id: page_id.as_str().to_owned(),
                    revision: 1,
                    ciphertext: page_envelope.canonical_json(),
                    deleted: false,
                },
                CliSyncObject {
                    folder_id: "home".to_owned(),
                    object_id: asset_id.as_str().to_owned(),
                    revision: 1,
                    ciphertext: asset_envelope.canonical_json(),
                    deleted: true,
                },
            ],
            control_records: Vec::new(),
        };

        let error = materialize_remote_projection(MaterializeRemoteProjectionContext {
            env: &env,
            root,
            actor_npub: "npub-owner",
            metadata: Some(&CliBrainMetadata::default()),
            export: &export,
            bootstrap: &bootstrap,
            mounted_folders: &[],
            path_overrides: &BTreeMap::new(),
            session_keys: &session_keys,
            prior_state: Some(&prior_state),
        })
        .unwrap_err();

        assert!(error.to_string().contains("legacy Asset path collision"));
        assert_eq!(
            fs::read(root.join("home/docs/collision.md")).unwrap(),
            prior_local_asset
        );
    }

    #[test]
    fn folder_conventions_stay_stable_when_the_first_page_is_materialized() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        write_json_file(
            &root.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: "finite-brain-working-tree-state-v1".to_owned(),
                folder_roots: Vec::new(),
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        let folder_key = FolderKey::from_bytes([4; 32]);
        let mut session_keys = SessionFolderKeyring::default();
        session_keys.insert("brain", "home", 1, folder_key.clone());
        let env = CliEnvironment {
            cwd: root.to_path_buf(),
            config_dir: root.join("config"),
            server_url: None,
            public_base_url: None,
            working_tree_root: None,
            now: Some("2026-07-25T18:00:00Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(root.join("finite-home")),
            embedding_provider: None,
        };
        let export = CliEncryptedBrainExport {
            brain: CliExportBrain {
                id: "brain".to_owned(),
                kind: "personal".to_owned(),
                name: "Brain".to_owned(),
                owner_user_id: Some("npub-owner".to_owned()),
            },
            folders: vec![CliExportFolder {
                id: "home".to_owned(),
                path: "home".to_owned(),
                access: "owner".to_owned(),
                current_key_version: 1,
                accessible: true,
            }],
            objects: Vec::new(),
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },

            pending_wraps: Vec::new(),
        };
        let empty_bootstrap = CliSyncBootstrap {
            latest_sequence: 0,
            objects: Vec::new(),
            control_records: Vec::new(),
        };

        materialize_remote_projection(MaterializeRemoteProjectionContext {
            env: &env,
            root,
            actor_npub: "npub-owner",
            metadata: Some(&CliBrainMetadata::default()),
            export: &export,
            bootstrap: &empty_bootstrap,
            mounted_folders: &[],
            path_overrides: &BTreeMap::new(),
            session_keys: &session_keys,
            prior_state: None,
        })
        .unwrap();

        let empty_instructions = fs::read_to_string(root.join("home/AGENTS.md")).unwrap();
        let expected_conventions = [
            "raw/.keep",
            "wiki/.keep",
            "inventory/.keep",
            "datasets/.keep",
            "output/.keep",
        ];
        for convention in expected_conventions {
            assert!(root.join("home").join(convention).is_file());
        }

        fs::create_dir_all(root.join("home/compiled")).unwrap();
        fs::write(
            root.join("home/compiled/.keep"),
            "# compiled\n\nAgent convention directory for Folder `home`.\n",
        )
        .unwrap();
        fs::write(
            root.join("home/compiled/user-authored.md"),
            "# Preserve me\n",
        )
        .unwrap();

        let object_id = ObjectId::new("obj_firstpage001").unwrap();
        let page_path = SafeRelativePath::new("page_path", "wiki/first.md").unwrap();
        let plaintext = encode_folder_object_page_plaintext(&page_path, "# First\n").unwrap();
        let aad = FolderObjectAad {
            brain_id: BrainId::new("brain").unwrap(),
            folder_id: FolderId::new("home").unwrap(),
            object_id: object_id.clone(),
            key_version: 1,
        };
        let envelope = encrypt_folder_object(&folder_key, &aad, &plaintext).unwrap();
        let populated_bootstrap = CliSyncBootstrap {
            latest_sequence: 1,
            objects: vec![CliSyncObject {
                folder_id: "home".to_owned(),
                object_id: object_id.as_str().to_owned(),
                revision: 1,
                ciphertext: envelope.canonical_json(),
                deleted: false,
            }],
            control_records: Vec::new(),
        };

        for _ in 0..2 {
            materialize_remote_projection(MaterializeRemoteProjectionContext {
                env: &env,
                root,
                actor_npub: "npub-owner",
                metadata: Some(&CliBrainMetadata::default()),
                export: &export,
                bootstrap: &populated_bootstrap,
                mounted_folders: &[],
                path_overrides: &BTreeMap::new(),
                session_keys: &session_keys,
                prior_state: None,
            })
            .unwrap();

            assert_eq!(
                fs::read_to_string(root.join("home/AGENTS.md")).unwrap(),
                empty_instructions
            );
            for convention in expected_conventions {
                assert!(root.join("home").join(convention).is_file());
            }
            assert_eq!(
                fs::read_to_string(root.join("home/wiki/first.md")).unwrap(),
                "# First\n"
            );
            assert!(!root.join("home/compiled/.keep").exists());
            assert_eq!(
                fs::read_to_string(root.join("home/compiled/user-authored.md")).unwrap(),
                "# Preserve me\n"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn obsolete_marker_cleanup_stays_anchored_to_the_opened_compiled_directory() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let root = temp.path().join("tree");
        let external = temp.path().join("external");
        let compiled = root.join("home/compiled");
        let moved_compiled = root.join("home/compiled-before-swap");
        fs::create_dir_all(&compiled).unwrap();
        fs::create_dir_all(&external).unwrap();
        let legacy_marker = "# compiled\n\nAgent convention directory for Folder `home`.\n";
        fs::write(compiled.join(".keep"), legacy_marker).unwrap();
        fs::write(external.join(".keep"), legacy_marker).unwrap();
        let compiled_fd = open_compiled_directory(&root, "home").unwrap().unwrap();
        let quarantine_name = quarantine_legacy_marker(&compiled_fd).unwrap();

        fs::rename(&compiled, &moved_compiled).unwrap();
        symlink(&external, &compiled).unwrap();
        fs::write(moved_compiled.join(".keep"), "# Replacement\n").unwrap();
        verify_and_remove_quarantined_marker(
            &compiled_fd,
            &quarantine_name,
            legacy_marker.as_bytes(),
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(external.join(".keep")).unwrap(),
            legacy_marker
        );
        assert_eq!(
            fs::read_to_string(moved_compiled.join(".keep")).unwrap(),
            "# Replacement\n"
        );
        assert!(!moved_compiled.join(quarantine_name).exists());
    }

    #[test]
    fn materialize_remote_projection_mounts_source_folder_into_destination_tree() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        write_json_file(
            &root.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: "finite-brain-working-tree-state-v1".to_owned(),
                folder_roots: Vec::new(),
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        let folder_key = FolderKey::from_bytes([8; 32]);
        let mut session_keys = SessionFolderKeyring::default();
        session_keys.insert("source", "shared-lab", 1, folder_key.clone());
        let env = CliEnvironment {
            cwd: root.to_path_buf(),
            config_dir: root.join("config"),
            server_url: None,
            public_base_url: None,
            working_tree_root: None,
            now: Some("2026-06-26T23:30:00Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(root.join("finite-home")),
            embedding_provider: None,
        };
        let object_id = ObjectId::new("obj_mounted00001").unwrap();
        let page_path = SafeRelativePath::new("page_path", "compiled/share-brief.md").unwrap();
        let plaintext = encode_folder_object_page_plaintext(&page_path, "# Share Brief\n").unwrap();
        let aad = FolderObjectAad {
            brain_id: BrainId::new("source").unwrap(),
            folder_id: FolderId::new("shared-lab").unwrap(),
            object_id: object_id.clone(),
            key_version: 1,
        };
        let envelope = encrypt_folder_object(&folder_key, &aad, &plaintext).unwrap();
        let destination_export = CliEncryptedBrainExport {
            brain: CliExportBrain {
                id: "dest".to_owned(),
                kind: "organization".to_owned(),
                name: "Destination".to_owned(),
                owner_user_id: None,
            },
            folders: vec![CliExportFolder {
                id: "general".to_owned(),
                path: "general".to_owned(),
                access: "all_members".to_owned(),
                current_key_version: 1,
                accessible: true,
            }],
            objects: Vec::new(),
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },

            pending_wraps: Vec::new(),
        };
        let source_export = CliEncryptedBrainExport {
            brain: CliExportBrain {
                id: "source".to_owned(),
                kind: "organization".to_owned(),
                name: "Source".to_owned(),
                owner_user_id: None,
            },
            folders: vec![CliExportFolder {
                id: "shared-lab".to_owned(),
                path: "shared-lab".to_owned(),
                access: "restricted".to_owned(),
                current_key_version: 1,
                accessible: true,
            }],
            objects: Vec::new(),
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },

            pending_wraps: Vec::new(),
        };
        let mounted = MountedFolderMaterializeContext {
            mount: CliMountedFolder {
                mount_id: "mount-source-shared-lab".to_owned(),
                source_brain_id: "source".to_owned(),
                source_folder_id: "shared-lab".to_owned(),
                display_name: "Shared Lab".to_owned(),
                state: "available".to_owned(),
            },
            export: source_export,
            display_path: "shared-lab".to_owned(),
            bootstrap: CliSyncBootstrap {
                latest_sequence: 11,
                objects: vec![CliSyncObject {
                    folder_id: "shared-lab".to_owned(),
                    object_id: object_id.as_str().to_owned(),
                    revision: 3,
                    ciphertext: envelope.canonical_json(),
                    deleted: false,
                }],
                control_records: Vec::new(),
            },
        };

        materialize_remote_projection(MaterializeRemoteProjectionContext {
            env: &env,
            root,
            actor_npub: "npub-dest",
            metadata: Some(&CliBrainMetadata::default()),
            export: &destination_export,
            bootstrap: &CliSyncBootstrap {
                latest_sequence: 2,
                objects: Vec::new(),
                control_records: Vec::new(),
            },
            mounted_folders: &[mounted],
            path_overrides: &BTreeMap::new(),
            session_keys: &session_keys,
            prior_state: None,
        })
        .unwrap();

        assert_eq!(
            fs::read_to_string(root.join("shared-lab/compiled/share-brief.md")).unwrap(),
            "# Share Brief\n"
        );
        let mounted_instructions = fs::read_to_string(root.join("shared-lab/AGENTS.md")).unwrap();
        assert!(mounted_instructions.contains("wiki/"));
        assert!(!mounted_instructions.contains("compiled/"));
        for marker in [
            "raw/.keep",
            "wiki/.keep",
            "inventory/.keep",
            "datasets/.keep",
            "output/.keep",
        ] {
            assert!(root.join("shared-lab").join(marker).is_file());
        }
        let state = read_working_tree_state(root).unwrap();
        let root_entry = state
            .folder_roots
            .iter()
            .find(|root| root.path == "shared-lab")
            .unwrap();
        assert_eq!(root_entry.folder_id, "shared-lab");
        assert_eq!(root_entry.source_brain_id.as_deref(), Some("source"));
        let object_entry = state
            .objects
            .iter()
            .find(|object| object.path == "compiled/share-brief.md")
            .unwrap();
        assert_eq!(object_entry.source_brain_id.as_deref(), Some("source"));

        let intents = plan_working_tree_change_intents(
            &state,
            &[WorkingTreeChange::Upsert {
                path: SafeRelativePath::new("change_path", "shared-lab/compiled/share-brief.md")
                    .unwrap(),
                markdown: "# Updated\n".to_owned(),
            }],
        );
        assert_eq!(
            intents[0].source_brain_id.as_ref().map(ToString::to_string),
            Some("source".to_owned())
        );
        assert_eq!(
            intents[0].folder_id,
            Some(FolderId::new("shared-lab").unwrap())
        );
        assert_eq!(intents[0].base_revision, Some(3));
    }

    #[test]
    fn historical_session_keys_do_not_make_current_folder_readable() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        let persisted_plaintext = "# Persisted until explicit removal\n";
        fs::create_dir_all(root.join("home/notes")).unwrap();
        fs::write(root.join("home/notes/persisted.md"), persisted_plaintext).unwrap();
        write_json_file(
            &root.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: "finite-brain-working-tree-state-v1".to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "home".to_owned(),
                    source_brain_id: None,
                    path: "home".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: vec![WorkingTreeObjectManifestEntry {
                    folder_id: "home".to_owned(),
                    source_brain_id: None,
                    path: "notes/persisted.md".to_owned(),
                    object_id: "obj_persisted001".to_owned(),
                    revision: 1,
                    key_version: 1,
                    content_type: "text/markdown".to_owned(),
                    content_hash: sha256_hex(persisted_plaintext.as_bytes()),
                }],
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        let mut session_keys = SessionFolderKeyring::default();
        session_keys.insert("brain", "home", 1, FolderKey::from_bytes([1; 32]));
        let env = CliEnvironment {
            cwd: root.to_path_buf(),
            config_dir: root.join("config"),
            server_url: None,
            public_base_url: None,
            working_tree_root: None,
            now: Some("2026-06-26T23:30:00Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(root.join("finite-home")),
            embedding_provider: None,
        };
        let export = CliEncryptedBrainExport {
            brain: CliExportBrain {
                id: "brain".to_owned(),
                kind: "personal".to_owned(),
                name: "Brain".to_owned(),
                owner_user_id: Some("npub-owner".to_owned()),
            },
            folders: vec![CliExportFolder {
                id: "home".to_owned(),
                path: "home".to_owned(),
                access: "owner".to_owned(),
                current_key_version: 2,
                accessible: true,
            }],
            objects: Vec::new(),
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },

            pending_wraps: Vec::new(),
        };
        let bootstrap = CliSyncBootstrap {
            latest_sequence: 0,
            objects: Vec::new(),
            control_records: Vec::new(),
        };

        materialize_remote_projection(MaterializeRemoteProjectionContext {
            env: &env,
            root,
            actor_npub: "npub-owner",
            metadata: Some(&CliBrainMetadata::default()),
            export: &export,
            bootstrap: &bootstrap,
            mounted_folders: &[],
            path_overrides: &BTreeMap::new(),
            session_keys: &session_keys,
            prior_state: None,
        })
        .unwrap();

        let state = read_working_tree_state(root).unwrap();
        assert_eq!(state.folder_roots.len(), 1);
        assert_eq!(state.folder_roots[0].folder_id, "home");
        assert!(!state.folder_roots[0].can_read);
        assert!(state.folder_roots[0].metadata_only);
        assert_eq!(state.objects.len(), 1);
        assert_eq!(state.objects[0].object_id, "obj_persisted001");
        assert_eq!(
            fs::read_to_string(root.join("home/notes/persisted.md")).unwrap(),
            persisted_plaintext
        );
    }

    #[allow(dead_code)]
    fn _directory_manifest() -> BrainDirectoryManifest {
        BrainDirectoryManifest {
            version: "finite-brain-directory-v1".to_owned(),
            brain: BrainDirectoryBrainSummary {
                id: "brain".to_owned(),
                kind: "personal".to_owned(),
                name: "Brain".to_owned(),
                owner_npub: Some("npub-owner".to_owned()),
            },
            working_tree: BrainDirectoryPath {
                path: ".".to_owned(),
            },
            encrypted_sync: BrainDirectoryPath {
                path: ".finitebrain/encrypted-sync".to_owned(),
            },
            portability: BrainDirectoryPortability {
                owned_by_agent_runtime: true,
                owned_by_app_surface: false,
            },
            created_at: "2026-06-26T23:30:00Z".to_owned(),
            updated_at: "2026-06-26T23:30:00Z".to_owned(),
        }
    }
}
