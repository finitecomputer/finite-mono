use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use finite_brain_core::portability::{
    MAX_WORKING_TREE_ASSET_BYTES, OkfOmittedFolder, OpenedAsset, OpenedPage,
    VaultWorkingTreeStateManifest, WorkingTreeChange, WorkingTreeChangeIntent,
    WorkingTreeFolderRoot, WorkingTreeIntentAction, WorkingTreeIntentContent,
    WorkingTreeIntentRoute, WorkingTreeMaterializeInput, WorkingTreeObjectManifestEntry,
    WorkingTreeProjection, materialize_vault_working_tree, plan_working_tree_change_intents,
};
use finite_brain_core::{
    DisplayName, EncryptedFolderObjectEnvelope, Folder, FolderAccessMode, FolderId, FolderKey,
    FolderObjectAad, FolderObjectOperation, FolderObjectRevisionPayload, FolderRole, ObjectId,
    RevisionValidation, SafeRelativePath, TombstoneValidation, UserId, Vault, VaultId, VaultKind,
    encrypt_folder_object, open_folder_object, sha256_hex,
};
use finite_nostr::{GiftWrapValidation, NostrPublicKey, open_gift_wrap};
use nostr::{Event, Keys, Kind, Tag};
use serde::Deserialize;

#[cfg(test)]
use crate::initialize_private_working_tree;
use crate::{
    APP_SPECIFIC_KIND, AgentState, CliEnvironment, CliError, ConflictEntry, ConflictState,
    SessionFolderKeyring, SyncChangeReport, SyncOnceReport, current_tree_root, deterministic_id,
    load_signer, read_agent_state, read_working_tree_state, server_url_for_command, sign_event,
    signed_json_request, signed_json_request_to_server, tag_vec, timestamp, timestamp_from_unix,
    unix_timestamp, write_agent_state, write_json_file, write_private_file_atomic,
};

const CIPHER_AES_256_GCM: &str = "AES-256-GCM";
const FOLDER_OBJECT_PAGE_VERSION: &str = "finite-folder-object-page-v1";
const SYNC_RECORDS_PAGE_LIMIT: u64 = 1_000;
const MAX_WORKING_TREE_FILE_COUNT: usize = 10_000;
const MAX_WORKING_TREE_RECURSION_DEPTH: usize = 32;

pub(crate) fn run_working_tree_sync(
    env: &CliEnvironment,
    args: &[String],
    activity_kind: &str,
) -> Result<SyncOnceReport, CliError> {
    let root = current_tree_root(env)?;
    let agent_state = read_agent_state(&root)?;
    let prior_tree_state = read_working_tree_state(&root)?;
    let server_url = server_url_for_command(env, args)?;
    let auth = load_signer(env)?;
    let export = fetch_encrypted_export(env, &server_url, &agent_state.vault_id)?;
    let mounted_exports =
        fetch_mounted_folder_sync_contexts(env, &server_url, &agent_state.vault_id, &export)?;
    let mut session_keys = SessionFolderKeyring::default();
    open_export_folder_key_grants_into_session(&auth, &export, &mut session_keys)?;
    for mounted in &mounted_exports {
        open_export_folder_key_grants_into_session(&auth, &mounted.export, &mut session_keys)?;
    }
    let opened_grants = session_keys.len();
    let newly_readable_keys = newly_readable_session_key_count(
        &prior_tree_state,
        &export,
        &mounted_exports,
        &session_keys,
    );
    let local_result = push_local_working_tree_changes(
        env,
        &root,
        &server_url,
        &agent_state,
        &export,
        &mounted_exports,
        &session_keys,
    )?;
    let force_bootstrap_reason = sync_bootstrap_reason(&local_result, newly_readable_keys);
    let remote_result = if let Some(reason) = force_bootstrap_reason {
        fetch_bootstrap_remote_sync(env, &server_url, &agent_state.vault_id, reason)?
    } else {
        fetch_incremental_remote_sync(
            env,
            &root,
            &server_url,
            &agent_state.vault_id,
            prior_tree_state.sync.latest_sequence,
        )?
    };
    let mounted_materializations =
        fetch_mounted_folder_materializations(env, &server_url, mounted_exports)?;
    write_sync_evidence(&root, &export, &remote_result.bootstrap)?;

    materialize_remote_projection(MaterializeRemoteProjectionContext {
        env,
        root: &root,
        actor_npub: &auth.npub,
        export: &export,
        bootstrap: &remote_result.bootstrap,
        mounted_folders: &mounted_materializations,
        path_overrides: &local_result.path_overrides,
        session_keys: &session_keys,
    })?;
    restore_conflicted_files(
        &root,
        &local_result.conflicted_markdown,
        &local_result.conflicted_assets,
    )?;

    let applied_tree_state = read_working_tree_state(&root)?;
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
        "blocked-local-conflicts".to_owned()
    } else if local_result.pushed_count > 0 {
        "pushed-local-changes".to_owned()
    } else if !remote_changes.is_empty()
        || newly_readable_keys > 0
        || (remote_result.used_bootstrap
            && latest_sequence > prior_tree_state.sync.latest_sequence
            && active_remote_object_count > 0)
    {
        "applied-remote-records".to_owned()
    } else {
        "caught-up".to_owned()
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
    })?;

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
    })
}

pub(crate) fn open_vault_session_folder_keys(
    env: &CliEnvironment,
    args: &[String],
    vault_id: &str,
) -> Result<SessionFolderKeyring, CliError> {
    let path = format!("/_admin/vaults/{vault_id}/export");
    let response = signed_json_request(env, args, "GET", &path, None)?;
    let export: CliEncryptedVaultExport = serde_json::from_value(response)?;
    if export.vault.id != vault_id {
        return Err(CliError::InvalidInput(format!(
            "encrypted export returned vault {} while opening {vault_id}",
            export.vault.id
        )));
    }
    let auth = load_signer(env)?;
    let mut keyring = SessionFolderKeyring::default();
    open_export_folder_key_grants_into_session(&auth, &export, &mut keyring)?;
    Ok(keyring)
}

fn newly_readable_session_key_count(
    prior_tree_state: &finite_brain_core::portability::VaultWorkingTreeStateManifest,
    export: &CliEncryptedVaultExport,
    mounted_exports: &[MountedFolderSyncContext],
    session_keys: &SessionFolderKeyring,
) -> usize {
    let primary = export.folders.iter().filter(|folder| {
        session_keys.contains(&export.vault.id, &folder.id, folder.current_key_version)
            && !prior_tree_state.folder_roots.iter().any(|root| {
                root.source_vault_id.is_none() && root.folder_id == folder.id && root.can_read
            })
    });
    let mounted = mounted_exports.iter().filter(|mounted| {
        mounted.source_folder().is_some_and(|folder| {
            session_keys.contains(
                &mounted.export.vault.id,
                &folder.id,
                folder.current_key_version,
            ) && !prior_tree_state.folder_roots.iter().any(|root| {
                root.source_vault_id.as_deref() == Some(mounted.export.vault.id.as_str())
                    && root.folder_id == folder.id
                    && root.can_read
            })
        })
    });
    primary.count() + mounted.count()
}

pub(crate) fn pending_working_tree_change_count(root: &Path) -> Result<usize, CliError> {
    let tree_state = read_working_tree_state(root)?;
    Ok(scan_working_tree_changes(root, &tree_state)?.len())
}

fn fetch_encrypted_export(
    env: &CliEnvironment,
    server_url: &str,
    vault_id: &str,
) -> Result<CliEncryptedVaultExport, CliError> {
    let path = format!("/_admin/vaults/{vault_id}/export");
    let response = signed_json_request_to_server(env, server_url, "GET", &path, None)?;
    serde_json::from_value(response).map_err(CliError::from)
}

fn fetch_sync_bootstrap(
    env: &CliEnvironment,
    server_url: &str,
    vault_id: &str,
) -> Result<CliSyncBootstrap, CliError> {
    let path = format!("/_admin/vaults/{vault_id}/sync/bootstrap");
    let response = signed_json_request_to_server(env, server_url, "GET", &path, None)?;
    serde_json::from_value(response).map_err(CliError::from)
}

fn fetch_bootstrap_remote_sync(
    env: &CliEnvironment,
    server_url: &str,
    vault_id: &str,
    reason: String,
) -> Result<RemoteSyncResult, CliError> {
    Ok(RemoteSyncResult {
        bootstrap: fetch_sync_bootstrap(env, server_url, vault_id)?,
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
    vault_id: &str,
    after_sequence: u64,
) -> Result<RemoteSyncResult, CliError> {
    let pull = match fetch_all_sync_records(env, server_url, vault_id, after_sequence) {
        Ok(pull) => pull,
        Err(error) if is_rebootstrap_required_error(&error) => {
            return fetch_bootstrap_remote_sync(
                env,
                server_url,
                vault_id,
                format!("incremental cursor {after_sequence} expired; fetched bootstrap"),
            );
        }
        Err(error) if is_sync_records_route_unavailable(&error) => {
            return fetch_bootstrap_remote_sync(
                env,
                server_url,
                vault_id,
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
                fetch_bootstrap_remote_sync(env, server_url, vault_id, reason.clone())?;
            result.records = records;
            result.report_reason = Some(reason);
            Ok(result)
        }
    }
}

fn fetch_all_sync_records(
    env: &CliEnvironment,
    server_url: &str,
    vault_id: &str,
    after_sequence: u64,
) -> Result<IncrementalSyncPull, CliError> {
    let mut after = after_sequence;
    let mut records = Vec::new();
    loop {
        let page = fetch_sync_records_page(env, server_url, vault_id, after)?;
        if page.vault_id != vault_id {
            return Err(CliError::InvalidInput(format!(
                "sync records response vault {} did not match requested vault {vault_id}",
                page.vault_id
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
    vault_id: &str,
    after_sequence: u64,
) -> Result<CliSyncPull, CliError> {
    let path = format!(
        "/_admin/vaults/{vault_id}/sync/records?after={after_sequence}&limit={SYNC_RECORDS_PAGE_LIMIT}"
    );
    let response = signed_json_request_to_server(env, server_url, "GET", &path, None)?;
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
    matches!(error, CliError::Http(message) if message.contains("410") || message.contains("rebootstrap required"))
}

fn is_sync_records_route_unavailable(error: &CliError) -> bool {
    matches!(error, CliError::Http(message) if message.contains("404"))
}

fn apply_incremental_records(
    root: &Path,
    after_sequence: u64,
    latest_sequence: u64,
    records: &[CliSyncRecord],
) -> Result<CliSyncBootstrap, String> {
    if latest_sequence < after_sequence {
        return Err(format!(
            "sync records latest sequence {latest_sequence} is older than cursor {after_sequence}"
        ));
    }
    let base = incremental_base_bootstrap(root, after_sequence)?;
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
                        ciphertext: record_payload_ciphertext(record),
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

fn record_payload_ciphertext(record: &CliSyncRecord) -> String {
    serde_json::from_str::<serde_json::Value>(&record.payload_json)
        .ok()
        .and_then(|value| {
            value
                .get("ciphertext")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| record.payload_json.clone())
}

fn sync_record_reports(
    records: &[CliSyncRecord],
    prior_state: &finite_brain_core::portability::VaultWorkingTreeStateManifest,
    applied_state: &finite_brain_core::portability::VaultWorkingTreeStateManifest,
    status: &str,
    reason: Option<&str>,
) -> Vec<SyncChangeReport> {
    records
        .iter()
        .map(|record| SyncChangeReport {
            status: status.to_owned(),
            action: sync_record_action(record),
            actor_npub: Some(record.actor_npub.clone()),
            sequence: Some(record.sequence),
            path: sync_record_path(record, prior_state, applied_state),
            from_path: None,
            folder_id: record.folder_id.clone(),
            source_vault_id: None,
            object_id: record.object_id.clone(),
            route: "sync-record".to_owned(),
            reason: reason.map(ToOwned::to_owned),
        })
        .collect()
}

fn sync_record_action(record: &CliSyncRecord) -> String {
    match record.record_type.as_str() {
        "folder_object_revision" => {
            if sync_record_base_revision_is_none(record) {
                "create".to_owned()
            } else {
                "update".to_owned()
            }
        }
        "folder_object_tombstone" => "delete".to_owned(),
        other => other.to_owned(),
    }
}

fn sync_record_base_revision_is_none(record: &CliSyncRecord) -> bool {
    serde_json::from_str::<serde_json::Value>(&record.payload_json)
        .ok()
        .and_then(|value| value.get("baseRevision").cloned())
        .is_none_or(|value| value.is_null())
}

fn sync_record_path(
    record: &CliSyncRecord,
    prior_state: &finite_brain_core::portability::VaultWorkingTreeStateManifest,
    applied_state: &finite_brain_core::portability::VaultWorkingTreeStateManifest,
) -> Option<String> {
    let folder_id = record.folder_id.as_deref()?;
    let object_id = record.object_id.as_deref()?;
    working_tree_path_for_record(applied_state, folder_id, object_id)
        .or_else(|| working_tree_path_for_record(prior_state, folder_id, object_id))
}

fn working_tree_path_for_record(
    state: &finite_brain_core::portability::VaultWorkingTreeStateManifest,
    folder_id: &str,
    object_id: &str,
) -> Option<String> {
    let object = state.objects.iter().find(|object| {
        object.source_vault_id.is_none()
            && object.folder_id == folder_id
            && object.object_id == object_id
    })?;
    let folder = state
        .folder_roots
        .iter()
        .find(|folder| folder.source_vault_id.is_none() && folder.folder_id == folder_id)?;
    Some(format!("{}/{}", folder.path, object.path))
}

fn fetch_vault_metadata_for_sync(
    env: &CliEnvironment,
    server_url: &str,
    vault_id: &str,
) -> Result<CliVaultMetadata, CliError> {
    let path = format!("/_admin/vaults/{vault_id}/metadata");
    let response = signed_json_request_to_server(env, server_url, "GET", &path, None)?;
    serde_json::from_value(response).map_err(CliError::from)
}

fn fetch_mounted_folder_sync_contexts(
    env: &CliEnvironment,
    server_url: &str,
    vault_id: &str,
    export: &CliEncryptedVaultExport,
) -> Result<Vec<MountedFolderSyncContext>, CliError> {
    if export.vault.kind != "organization" {
        return Ok(Vec::new());
    }
    let metadata = match fetch_vault_metadata_for_sync(env, server_url, vault_id) {
        Ok(metadata) => metadata,
        Err(CliError::Http(_)) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut used_paths = export
        .folders
        .iter()
        .map(|folder| folder.path.clone())
        .collect::<BTreeSet<_>>();
    let mut contexts = Vec::new();
    for mount in metadata
        .mounted_folders
        .into_iter()
        .filter(|mount| mount.state == "available")
    {
        let source_export = fetch_encrypted_export(env, server_url, &mount.source_vault_id)?;
        let display_path = mounted_folder_display_path(&mut used_paths, &mount, &source_export)?;
        contexts.push(MountedFolderSyncContext {
            mount,
            export: source_export,
            display_path,
        });
    }
    Ok(contexts)
}

fn fetch_mounted_folder_materializations(
    env: &CliEnvironment,
    server_url: &str,
    mounted_exports: Vec<MountedFolderSyncContext>,
) -> Result<Vec<MountedFolderMaterializeContext>, CliError> {
    mounted_exports
        .into_iter()
        .map(|mounted| {
            let bootstrap = fetch_sync_bootstrap(env, server_url, &mounted.export.vault.id)?;
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
    source_export: &CliEncryptedVaultExport,
) -> Result<String, CliError> {
    let source_folder = source_export
        .folders
        .iter()
        .find(|folder| folder.id == mount.source_folder_id)
        .ok_or_else(|| CliError::NotFound(format!("folder {}", mount.source_folder_id)))?;
    let candidates = [
        source_folder.path.clone(),
        mount.display_name.clone(),
        format!("{}/{}", mount.source_vault_id, source_folder.path),
        format!("{}/{}", mount.source_vault_id, mount.source_folder_id),
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
    export: &CliEncryptedVaultExport,
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
    conflicted_assets: &BTreeMap<String, Vec<u8>>,
) -> Result<(), CliError> {
    for (relative_path, markdown) in conflicted_markdown {
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, markdown)?;
    }
    for (relative_path, bytes) in conflicted_assets {
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
    }
    Ok(())
}

fn open_export_folder_key_grants_into_session(
    auth: &crate::LocalSigner,
    export: &CliEncryptedVaultExport,
    session_keys: &mut SessionFolderKeyring,
) -> Result<usize, CliError> {
    let opened = opened_export_folder_key_grants(auth, export)?;
    let mut opened_count = 0;
    for grant in opened {
        let folder_key =
            FolderKey::from_base64(&grant.folder_key).map_err(|_| CliError::GrantOpening {
                vault_id: grant.vault_id.clone(),
                folder_id: grant.folder_id.clone(),
                key_version: grant.key_version,
                reason: "opened grant did not contain a valid Folder Key".to_owned(),
            })?;
        if session_keys.insert(
            grant.vault_id,
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
    export: &CliEncryptedVaultExport,
) -> Result<Vec<CliFolderKeyGrantPlaintext>, CliError> {
    let keys = auth.keys.clone();
    let recipient = NostrPublicKey::parse(&auth.npub)
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    let validation = GiftWrapValidation::new(recipient);
    let mut opened = Vec::new();
    for grant in &export.key_grants {
        if grant.recipient_npub != auth.npub {
            continue;
        }
        let unusable_grant = || {
            CliError::GrantOpening {
                vault_id: export.vault.id.clone(),
                folder_id: grant.folder_id.clone(),
                key_version: grant.key_version,
                reason: "the local signer could not validate and decrypt it; verify this Member Identity has a valid current grant"
                    .to_owned(),
            }
        };
        let event =
            Event::from_json(grant.wrapped_event_json.clone()).map_err(|_| unusable_grant())?;
        let opened_wrap =
            open_gift_wrap(&keys, &event, &validation).map_err(|_| unusable_grant())?;
        let plaintext =
            serde_json::from_str::<CliFolderKeyGrantPlaintext>(&opened_wrap.rumor.content)
                .map_err(|_| unusable_grant())?;
        if plaintext.version != "finite-folder-key-grant-v1"
            || plaintext.vault_id != export.vault.id
            || plaintext.folder_id != grant.folder_id
            || plaintext.key_version != grant.key_version
            || plaintext.issuer_npub != grant.issuer_npub
            || plaintext.recipient_npub != auth.npub
        {
            return Err(unusable_grant());
        }
        FolderKey::from_base64(&plaintext.folder_key).map_err(|_| unusable_grant())?;
        opened.push(plaintext);
    }

    Ok(opened)
}

fn push_local_working_tree_changes(
    env: &CliEnvironment,
    root: &Path,
    server_url: &str,
    agent_state: &AgentState,
    export: &CliEncryptedVaultExport,
    mounted_exports: &[MountedFolderSyncContext],
    session_keys: &SessionFolderKeyring,
) -> Result<LocalSyncResult, CliError> {
    let tree_state = read_working_tree_state(root)?;
    let changes = scan_working_tree_changes(root, &tree_state)?;
    if changes.is_empty() {
        return Ok(LocalSyncResult::default());
    }

    let intents = plan_working_tree_change_intents(&tree_state, &changes);
    let mut current_key_version_by_folder = export
        .folders
        .iter()
        .map(|folder| {
            (
                (export.vault.id.clone(), folder.id.clone()),
                folder.current_key_version,
            )
        })
        .collect::<BTreeMap<_, _>>();
    for mounted in mounted_exports {
        if let Some(folder) = mounted.source_folder() {
            current_key_version_by_folder.insert(
                (mounted.export.vault.id.clone(), folder.id.clone()),
                folder.current_key_version,
            );
        }
    }
    let signing_keys = load_signer(env)?.keys;
    let actor_npub = NostrPublicKey::from_protocol(signing_keys.public_key())
        .to_npub()
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;

    let submit_context = SubmitIntentContext {
        env,
        server_url,
        agent_state,
        signing_keys: &signing_keys,
        actor_npub: &actor_npub,
        session_keys,
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
                    let route_vault_id = intent
                        .source_vault_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| agent_state.vault_id.clone());
                    result.path_overrides.insert(
                        (
                            route_vault_id,
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
                conflicts.push(conflict_for_change(change, intent, reason, timestamp(env)));
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
                    timestamp(env),
                ));
            }
            Err(error) => return Err(error),
        }
    }

    if !conflicts.is_empty() {
        mutate_agent_state_at_root(root, timestamp(env), |state, now| {
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
        source_vault_id: intent.source_vault_id.as_ref().map(ToString::to_string),
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
    let route_vault_id = intent
        .source_vault_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| context.agent_state.vault_id.clone());
    let Some(current_key_version) = context
        .current_key_version_by_folder
        .get(&(route_vault_id.clone(), folder_id.to_string()))
        .copied()
    else {
        return Ok(SubmitIntentOutcome::Conflict(format!(
            "folder {folder_id} is missing from encrypted export for vault {route_vault_id}"
        )));
    };
    let current_session_key =
        context
            .session_keys
            .get(&route_vault_id, &folder_id.to_string(), current_key_version);
    if current_session_key.is_none() {
        return Ok(SubmitIntentOutcome::Conflict(format!(
            "current Folder Key v{current_key_version} unavailable for {route_vault_id}/{folder_id}"
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
                vault_id: VaultId::new(route_vault_id.clone())
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
                    vault_id: &route_vault_id,
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
                    "/_admin/vaults/{}/folders/{}/objects/{}/move",
                    route_vault_id,
                    folder_id,
                    object_id.as_str()
                ),
                _ => format!(
                    "/_admin/vaults/{}/folders/{}/objects/{}",
                    route_vault_id,
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
                &route_vault_id,
                folder_id,
                object_id,
                base_revision,
            )?;
            let body = serde_json::json!({
                "baseRevision": base_revision,
                "tombstoneEvent": event
            });
            let route = format!(
                "/_admin/vaults/{}/folders/{}/objects/{}",
                route_vault_id,
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
    pub(crate) vault_id: &'a str,
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
        vault_id: VaultId::new(input.vault_id.to_owned())
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
    vault_id: &str,
    folder_id: &FolderId,
    object_id: &ObjectId,
    base_revision: u64,
) -> Result<serde_json::Value, CliError> {
    let created_at_unix = unix_timestamp();
    let expected = TombstoneValidation {
        vault_id: VaultId::new(vault_id.to_owned())
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
                input.vault_id,
                input.folder_id,
                input.object_id.as_str(),
                input.revision
            ),
        ])?,
        tag_vec(["vault", &input.vault_id.to_string()])?,
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
                input.vault_id,
                input.folder_id,
                input.object_id.as_str(),
                input.revision
            ),
        ])?,
        tag_vec(["vault", &input.vault_id.to_string()])?,
        tag_vec(["folder", &input.folder_id.to_string()])?,
        tag_vec(["object", input.object_id.as_str()])?,
        tag_vec(["operation", "delete"])?,
    ])
}

struct MaterializeRemoteProjectionContext<'a> {
    env: &'a CliEnvironment,
    root: &'a Path,
    actor_npub: &'a str,
    export: &'a CliEncryptedVaultExport,
    bootstrap: &'a CliSyncBootstrap,
    mounted_folders: &'a [MountedFolderMaterializeContext],
    path_overrides: &'a BTreeMap<(String, String, String), String>,
    session_keys: &'a SessionFolderKeyring,
}

fn materialize_remote_projection(
    context: MaterializeRemoteProjectionContext<'_>,
) -> Result<(), CliError> {
    let MaterializeRemoteProjectionContext {
        env,
        root,
        actor_npub,
        export,
        bootstrap,
        mounted_folders,
        path_overrides,
        session_keys,
    } = context;
    let prior_state = read_working_tree_state(root)?;
    let vault = vault_from_export(export)?;
    let mut prior_paths = prior_state
        .objects
        .iter()
        .map(|entry| {
            (
                (
                    entry
                        .source_vault_id
                        .clone()
                        .unwrap_or_else(|| export.vault.id.clone()),
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
    let mut opened_assets = Vec::new();
    let mut readable_folder_routes = BTreeSet::new();
    {
        let mut append_context = OpenedObjectsAppendContext {
            session_keys,
            prior_paths: &prior_paths,
            opened_pages: &mut opened_pages,
            opened_assets: &mut opened_assets,
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
        if session_keys.contains(&export.vault.id, &folder.id, folder.current_key_version) {
            readable_folder_routes.insert((export.vault.id.clone(), folder.id.clone()));
        }
    }
    for mounted in mounted_folders {
        if let Some(folder) = mounted.source_folder()
            && session_keys.contains(
                &mounted.export.vault.id,
                &folder.id,
                folder.current_key_version,
            )
        {
            readable_folder_routes.insert((mounted.export.vault.id.clone(), folder.id.clone()));
        }
    }

    let locked_folders = export
        .folders
        .iter()
        .filter(|folder| {
            !readable_folder_routes.contains(&(export.vault.id.clone(), folder.id.clone()))
        })
        .map(|folder| {
            Ok(OkfOmittedFolder {
                folder_id: FolderId::new(folder.id.clone())
                    .map_err(|error| CliError::InvalidInput(error.to_string()))?,
                source_vault_id: None,
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

    let mut projection = materialize_vault_working_tree(WorkingTreeMaterializeInput {
        generated_at: timestamp(env),
        generated_by_npub: UserId::new(actor_npub.to_owned())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        vault,
        opened_pages,
        opened_assets,
        locked_folders,
        latest_sequence: bootstrap.latest_sequence,
    })
    .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    add_empty_readable_folders(&mut projection, export, None, &readable_folder_routes, None)?;
    for mounted in mounted_folders {
        add_empty_readable_folders(
            &mut projection,
            &mounted.export,
            Some(&mounted.export.vault.id),
            &readable_folder_routes,
            Some((&mounted.mount.source_folder_id, &mounted.display_path)),
        )?;
    }
    preserve_unreadable_prior_projection(
        &prior_state,
        &mut projection,
        &export.vault.id,
        &readable_folder_routes,
    )?;
    remove_stale_object_files(root, &prior_state.objects, &projection.state.objects)?;
    write_projection_files(root, &projection.files, &projection.binary_files)?;
    Ok(())
}

fn preserve_unreadable_prior_projection(
    prior_state: &VaultWorkingTreeStateManifest,
    projection: &mut WorkingTreeProjection,
    primary_vault_id: &str,
    readable_folder_routes: &BTreeSet<(String, String)>,
) -> Result<(), CliError> {
    let is_unreadable = |source_vault_id: Option<&str>, folder_id: &str| {
        let source_vault_id = source_vault_id.unwrap_or(primary_vault_id);
        !readable_folder_routes.contains(&(source_vault_id.to_owned(), folder_id.to_owned()))
    };

    for root in &prior_state.folder_roots {
        let route = (root.source_vault_id.clone(), root.folder_id.clone());
        if !is_unreadable(root.source_vault_id.as_deref(), &root.folder_id) {
            continue;
        }
        if let Some(candidate) = projection.state.folder_roots.iter_mut().find(|candidate| {
            (
                candidate.source_vault_id.clone(),
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
        let route_is_unreadable =
            is_unreadable(object.source_vault_id.as_deref(), &object.folder_id);
        let object_key = (
            object.source_vault_id.clone(),
            object.folder_id.clone(),
            object.object_id.clone(),
        );
        if route_is_unreadable
            && !projection.state.objects.iter().any(|candidate| {
                (
                    candidate.source_vault_id.clone(),
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
            .then(left.source_vault_id.cmp(&right.source_vault_id))
            .then(left.folder_id.cmp(&right.folder_id))
    });
    projection.state.objects.sort_by(|left, right| {
        left.source_vault_id
            .cmp(&right.source_vault_id)
            .then(left.folder_id.cmp(&right.folder_id))
            .then(left.path.cmp(&right.path))
    });
    projection.files.insert(
        ".finitebrain/working-tree-state.json".to_owned(),
        serde_json::to_string_pretty(&projection.state)?,
    );
    Ok(())
}

struct OpenedObjectsAppendContext<'a> {
    session_keys: &'a SessionFolderKeyring,
    prior_paths: &'a BTreeMap<(String, String, String), String>,
    opened_pages: &'a mut Vec<OpenedPage>,
    opened_assets: &'a mut Vec<OpenedAsset>,
    readable_folder_routes: &'a mut BTreeSet<(String, String)>,
}

fn append_opened_objects_from_bootstrap(
    export: &CliEncryptedVaultExport,
    bootstrap: &CliSyncBootstrap,
    only_folder_id: Option<&str>,
    display_path_override: Option<&str>,
    context: &mut OpenedObjectsAppendContext<'_>,
) -> Result<(), CliError> {
    let source_vault_id = VaultId::new(export.vault.id.clone())
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    for object in bootstrap.objects.iter().filter(|object| {
        !object.deleted && only_folder_id.is_none_or(|folder_id| folder_id == object.folder_id)
    }) {
        let envelope = EncryptedFolderObjectEnvelope::from_json(&object.ciphertext)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let Some(folder_key) =
            context
                .session_keys
                .get(&export.vault.id, &object.folder_id, envelope.key_version)
        else {
            continue;
        };
        let aad = FolderObjectAad {
            vault_id: source_vault_id.clone(),
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
                export.vault.id.clone(),
                object.folder_id.clone(),
                object.object_id.clone(),
            ))
            .cloned()
            .unwrap_or_else(|| format!("{}.md", object.object_id));
        context
            .readable_folder_routes
            .insert((export.vault.id.clone(), object.folder_id.clone()));
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
                    source_vault_id: display_path_override.map(|_| source_vault_id.clone()),
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
            CliDecodedFolderObjectPlaintext::Asset {
                path,
                bytes,
                content_type,
            } => {
                context.opened_assets.push(OpenedAsset {
                    folder_id,
                    source_vault_id: display_path_override.map(|_| source_vault_id.clone()),
                    object_id,
                    folder_display_path,
                    asset_path: SafeRelativePath::new("asset_path", path)
                        .map_err(|error| CliError::InvalidInput(error.to_string()))?,
                    bytes,
                    revision: object.revision,
                    key_version: envelope.key_version,
                    content_type,
                });
            }
        }
    }
    Ok(())
}

fn add_empty_readable_folders(
    projection: &mut WorkingTreeProjection,
    export: &CliEncryptedVaultExport,
    source_vault_id: Option<&str>,
    readable_folder_routes: &BTreeSet<(String, String)>,
    only_folder_and_path: Option<(&str, &str)>,
) -> Result<(), CliError> {
    let existing = projection
        .state
        .folder_roots
        .iter()
        .map(|root| (root.source_vault_id.clone(), root.folder_id.clone()))
        .collect::<BTreeSet<_>>();
    for folder in export.folders.iter().filter(|folder| {
        only_folder_and_path.is_none_or(|(folder_id, _)| folder_id == folder.id)
            && readable_folder_routes.contains(&(export.vault.id.clone(), folder.id.clone()))
            && !existing.contains(&(source_vault_id.map(ToOwned::to_owned), folder.id.clone()))
    }) {
        let folder_path = only_folder_and_path
            .map(|(_, display_path)| display_path.to_owned())
            .unwrap_or_else(|| folder.path.clone());
        projection.state.folder_roots.push(WorkingTreeFolderRoot {
            folder_id: folder.id.clone(),
            source_vault_id: source_vault_id.map(ToOwned::to_owned),
            path: folder_path.clone(),
            can_read: true,
            metadata_only: false,
        });
        projection.files.insert(
            format!("{}/AGENTS.md", folder_path),
            format!(
                "# Folder Agent Instructions\n\nFolder id: `{}`\n\nUse `raw/` for source captures, `raw/assets/` for non-Markdown Assets, `wiki/` for durable synthesized pages, `inventory/` for source candidates and open questions, `datasets/` for manifests and query recipes, and `output/` for generated artifacts. Pair every Asset with a Markdown Source Note before citing it from synthesized work.\n",
                folder.id
            ),
        );
        projection.files.insert(
            format!("{}/_index.md", folder_path),
            format!("# {}\n\n", folder_path),
        );
        for convention in [
            "raw",
            "raw/assets",
            "wiki",
            "inventory",
            "datasets",
            "output",
        ] {
            projection.files.insert(
                format!("{}/{convention}/.keep", folder_path),
                format!(
                    "# {convention}\n\nAgent convention directory for Folder `{}`.\n",
                    folder.id
                ),
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

fn vault_from_export(export: &CliEncryptedVaultExport) -> Result<Vault, CliError> {
    let kind = match export.vault.kind.as_str() {
        "personal" => VaultKind::Personal,
        "organization" => VaultKind::Organization,
        other => {
            return Err(CliError::InvalidInput(format!(
                "unknown vault kind {other}"
            )));
        }
    };
    Ok(Vault {
        id: VaultId::new(export.vault.id.clone())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        kind,
        name: DisplayName::new("vault_name", export.vault.name.clone())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        owner_user_id: export
            .vault
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
                Ok(finite_brain_core::VaultMember {
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
            "vault-ops" => FolderRole::VaultOps,
            "general" => FolderRole::General,
            _ => FolderRole::Folder,
        },
        access: parse_folder_access(&folder.access)?,
        parent_folder_id: None,
        path: SafeRelativePath::new("folder_path", folder.path.clone())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        current_key_version: folder.current_key_version,
        shared_folder_source: folder.shared_folder_source,
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
    state: &finite_brain_core::portability::VaultWorkingTreeStateManifest,
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
        let markdown_sources = read_folder_markdown_sources(root, &folder.path, &file_paths)?;
        for relative_path in file_paths {
            if is_generated_folder_file(&folder.path, &relative_path) {
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
            } else {
                let bytes = read_working_tree_asset_bytes(root, &relative_path)?;
                let local_asset_path = folder_local_path(&folder.path, &relative_path)?;
                let has_source_note = markdown_sources
                    .values()
                    .any(|markdown| markdown_mentions_asset_path(markdown, &local_asset_path));
                let violates_asset_convention =
                    !local_asset_path.starts_with("raw/assets/") || !has_source_note;
                if !matches!(
                    known.get(&relative_path),
                    Some(object) if object.content_hash == sha256_hex(&bytes)
                        && !violates_asset_convention
                ) {
                    changes.push(WorkingTreeChange::UpsertAsset {
                        path: SafeRelativePath::new("change_path", relative_path)
                            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
                        bytes,
                        content_type: content_type_for_path(&local_asset_path).to_owned(),
                        has_source_note,
                    });
                }
            }
        }
    }

    for (relative_path, _) in known {
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
    state: &finite_brain_core::portability::VaultWorkingTreeStateManifest,
    object: &WorkingTreeObjectManifestEntry,
) -> Option<String> {
    state
        .folder_roots
        .iter()
        .find(|folder| {
            folder.folder_id == object.folder_id && folder.source_vault_id == object.source_vault_id
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

fn read_folder_markdown_sources(
    root: &Path,
    folder_path: &str,
    file_paths: &[String],
) -> Result<BTreeMap<String, String>, CliError> {
    let mut sources = BTreeMap::new();
    for relative_path in file_paths {
        if is_generated_folder_file(folder_path, relative_path) || !is_markdown_path(relative_path)
        {
            continue;
        }
        let local_path = folder_local_path(folder_path, relative_path)?;
        sources.insert(local_path, fs::read_to_string(root.join(relative_path))?);
    }
    Ok(sources)
}

fn markdown_mentions_asset_path(markdown: &str, asset_path: &str) -> bool {
    let mut search_start = 0;
    while let Some(offset) = markdown[search_start..].find(asset_path) {
        let start = search_start + offset;
        let end = start + asset_path.len();
        let before = markdown[..start].chars().next_back();
        let after = markdown[end..].chars().next();
        if !is_source_path_char(before) && !is_source_path_char(after) {
            return true;
        }
        search_start = end;
    }
    false
}

fn is_source_path_char(character: Option<char>) -> bool {
    character.is_some_and(|value| {
        value.is_ascii_alphanumeric() || matches!(value, '/' | '.' | '_' | '-' | '%' | '+')
    })
}

fn read_working_tree_asset_bytes(root: &Path, relative_path: &str) -> Result<Vec<u8>, CliError> {
    let path = root.join(relative_path);
    let size = fs::metadata(&path)?.len();
    if size > MAX_WORKING_TREE_ASSET_BYTES as u64 {
        return Err(CliError::InvalidInput(format!(
            "working tree asset {relative_path} exceeds size limit {MAX_WORKING_TREE_ASSET_BYTES}"
        )));
    }
    let bytes = fs::read(path)?;
    if bytes.len() > MAX_WORKING_TREE_ASSET_BYTES {
        return Err(CliError::InvalidInput(format!(
            "working tree asset {relative_path} exceeds size limit {MAX_WORKING_TREE_ASSET_BYTES}"
        )));
    }
    Ok(bytes)
}

fn is_markdown_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        == Some("md")
}

fn folder_local_path(folder_path: &str, relative_path: &str) -> Result<String, CliError> {
    relative_path
        .strip_prefix(folder_path)
        .and_then(|rest| rest.strip_prefix('/'))
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            CliError::InvalidInput(format!(
                "path {relative_path} is outside Folder root {folder_path}"
            ))
        })
}

fn content_type_for_path(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("txt") => "text/plain",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        _ => "application/octet-stream",
    }
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
                    object.source_vault_id.clone(),
                    object.folder_id.clone(),
                    object.object_id.clone(),
                ),
                object.path.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for old in old_objects {
        let key = (
            old.source_vault_id.clone(),
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

fn folder_path_for_removed_object(
    root: &Path,
    object: &WorkingTreeObjectManifestEntry,
) -> Result<Option<PathBuf>, CliError> {
    let state = read_working_tree_state(root)?;
    Ok(state
        .folder_roots
        .iter()
        .find(|folder| {
            folder.folder_id == object.folder_id && folder.source_vault_id == object.source_vault_id
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
        if relative_path.starts_with(".finitebrain/") {
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
    matches!(error, CliError::Http(message) if message.contains("409"))
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
    conflicted_assets: BTreeMap<String, Vec<u8>>,
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
    export: CliEncryptedVaultExport,
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
    export: CliEncryptedVaultExport,
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
        WorkingTreeChange::UpsertAsset { path, bytes, .. } => {
            result
                .conflicted_assets
                .insert(path.to_string(), bytes.clone());
        }
        WorkingTreeChange::Rename { .. } | WorkingTreeChange::Delete { .. } => {}
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
        CliDecodedFolderObjectPlaintext::Asset { path, .. } => Err(CliError::InvalidInput(
            format!("folder object asset plaintext is not a Markdown Page: {path}"),
        )),
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
        let asset: CliFolderObjectAssetPlaintext =
            serde_json::from_value(value).map_err(CliError::from)?;
        let asset_path = SafeRelativePath::new("asset_path", asset.path)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        if asset.size > MAX_WORKING_TREE_ASSET_BYTES as u64
            || asset.bytes_base64.len() > MAX_WORKING_TREE_ASSET_BYTES * 2
        {
            return Err(CliError::InvalidInput(format!(
                "folder object asset exceeds size limit {MAX_WORKING_TREE_ASSET_BYTES}"
            )));
        }
        let bytes = BASE64_STANDARD
            .decode(asset.bytes_base64.as_bytes())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        if bytes.len() > MAX_WORKING_TREE_ASSET_BYTES {
            return Err(CliError::InvalidInput(format!(
                "folder object asset exceeds size limit {MAX_WORKING_TREE_ASSET_BYTES}"
            )));
        }
        if asset.size != bytes.len() as u64 {
            return Err(CliError::InvalidInput(
                "folder object asset size does not match decoded bytes".to_owned(),
            ));
        }
        let actual_hash = sha256_hex(&bytes);
        if asset.content_hash != actual_hash {
            return Err(CliError::InvalidInput(
                "folder object asset hash does not match decoded bytes".to_owned(),
            ));
        }
        return Ok(CliDecodedFolderObjectPlaintext::Asset {
            path: asset_path.to_string(),
            bytes,
            content_type: asset.content_type,
        });
    }
    Ok(CliDecodedFolderObjectPlaintext::Page {
        path: fallback_path,
        markdown: text,
    })
}

enum CliDecodedFolderObjectPlaintext {
    Page {
        path: String,
        markdown: String,
    },
    Asset {
        path: String,
        bytes: Vec<u8>,
        content_type: String,
    },
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
struct CliEncryptedVaultExport {
    vault: CliExportVault,
    folders: Vec<CliExportFolder>,
    key_grants: Vec<CliFolderKeyGrant>,
    access_state: CliExportAccessState,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliExportVault {
    id: String,
    kind: String,
    name: String,
    owner_user_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliExportFolder {
    id: String,
    path: String,
    access: String,
    current_key_version: u32,
    shared_folder_source: bool,
    accessible: bool,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliFolderKeyGrant {
    folder_id: String,
    key_version: u32,
    issuer_npub: String,
    recipient_npub: String,
    wrapped_event_json: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliExportAccessState {
    members: Vec<String>,
    admins: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliSyncBootstrap {
    latest_sequence: u64,
    objects: Vec<CliSyncObject>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliSyncPull {
    vault_id: String,
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

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliVaultMetadata {
    #[serde(default)]
    mounted_folders: Vec<CliMountedFolder>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CliMountedFolder {
    mount_id: String,
    source_vault_id: String,
    source_folder_id: String,
    display_name: String,
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CliFolderKeyGrantPlaintext {
    version: String,
    vault_id: String,
    folder_id: String,
    key_version: u32,
    folder_key: String,
    issuer_npub: String,
    recipient_npub: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use finite_brain_core::portability::{
        VaultDirectoryManifest, VaultDirectoryPath, VaultDirectoryPortability,
        VaultDirectoryVaultSummary, VaultWorkingTreeStateManifest, WorkingTreeSyncState,
    };
    use finite_brain_core::{DisplayName, validate_revision_event};
    use tempfile::TempDir;

    #[test]
    fn scan_detects_markdown_create_update_and_delete() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("General/_wiki")).unwrap();
        fs::write(root.join("General/existing.md"), "# Changed\n").unwrap();
        fs::write(root.join("General/new.md"), "# New\n").unwrap();
        fs::write(root.join("General/AGENTS.md"), "# Generated\n").unwrap();
        fs::write(root.join("General/_wiki/index.md"), "# Generated\n").unwrap();
        let state = VaultWorkingTreeStateManifest {
            version: "finite-vault-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "general".to_owned(),
                source_vault_id: None,
                path: "General".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: vec![
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_vault_id: None,
                    path: "existing.md".to_owned(),
                    object_id: "obj_existing00000".to_owned(),
                    revision: 1,
                    key_version: 1,
                    content_type: "text/markdown".to_owned(),
                    content_hash: sha256_hex("# Old\n".as_bytes()),
                },
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_vault_id: None,
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
    fn scan_detects_asset_pairs_and_reports_invalid_assets() {
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
        let state = VaultWorkingTreeStateManifest {
            version: "finite-vault-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "general".to_owned(),
                source_vault_id: None,
                path: "General".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: vec![
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_vault_id: None,
                    path: "raw/source-note.md".to_owned(),
                    object_id: "obj_sourcenote000".to_owned(),
                    revision: 1,
                    key_version: 1,
                    content_type: "text/markdown".to_owned(),
                    content_hash: sha256_hex(source_note.as_bytes()),
                },
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_vault_id: None,
                    path: "raw/assets/existing.pdf".to_owned(),
                    object_id: "obj_assetexisting".to_owned(),
                    revision: 2,
                    key_version: 1,
                    content_type: "application/pdf".to_owned(),
                    content_hash: sha256_hex(b"old-pdf"),
                },
                WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_vault_id: None,
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

        assert_eq!(changes.len(), 4);
        assert!(changes.iter().any(|change| matches!(
            change,
            WorkingTreeChange::UpsertAsset {
                path,
                bytes,
                content_type,
                has_source_note
            } if path.as_str() == "General/raw/assets/existing.pdf"
                && bytes == b"changed-pdf"
                && content_type == "application/pdf"
                && *has_source_note
        )));
        assert!(changes.iter().any(|change| matches!(
            change,
            WorkingTreeChange::UpsertAsset {
                path,
                bytes,
                content_type,
                has_source_note
            } if path.as_str() == "General/raw/assets/new.pdf"
                && bytes == b"new-pdf"
                && content_type == "application/pdf"
                && *has_source_note
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

        let existing = by_path.get("General/raw/assets/existing.pdf").unwrap();
        assert_eq!(existing.action, WorkingTreeIntentAction::Update);
        assert_eq!(existing.base_revision, Some(2));
        assert!(matches!(
            existing.content.as_ref(),
            Some(WorkingTreeIntentContent::AssetBytes {
                content_type,
                bytes,
            }) if content_type == "application/pdf"
                && bytes == b"changed-pdf"
        ));
        assert_eq!(
            by_path.get("General/raw/assets/new.pdf").unwrap().action,
            WorkingTreeIntentAction::Create
        );
        assert!(matches!(
            by_path.get("General/raw/assets/missing-note.pdf").unwrap(),
            WorkingTreeChangeIntent {
                action: WorkingTreeIntentAction::Unresolved,
                reason: Some(reason),
                ..
            } if reason.contains("Source Note")
        ));
        assert!(matches!(
            by_path.get("General/stray.bin").unwrap(),
            WorkingTreeChangeIntent {
                action: WorkingTreeIntentAction::Unresolved,
                reason: Some(reason),
                ..
            } if reason.contains("raw/assets")
        ));
    }

    #[test]
    fn scan_requires_exact_asset_source_note_tokens() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("General/raw/assets")).unwrap();
        fs::write(
            root.join("General/raw/source-note.md"),
            "# Source Notes\n\n- Almost: raw/assets/file.pdf.bak\n",
        )
        .unwrap();
        fs::write(root.join("General/raw/assets/file.pdf"), b"asset").unwrap();
        let state = VaultWorkingTreeStateManifest {
            version: "finite-vault-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "general".to_owned(),
                source_vault_id: None,
                path: "General".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: vec![WorkingTreeObjectManifestEntry {
                folder_id: "general".to_owned(),
                source_vault_id: None,
                path: "raw/assets/file.pdf".to_owned(),
                object_id: "obj_assetfile0000".to_owned(),
                revision: 1,
                key_version: 1,
                content_type: "application/pdf".to_owned(),
                content_hash: sha256_hex(b"asset"),
            }],
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
            } if reason.contains("Source Note")
        ));
    }

    #[test]
    fn scan_rejects_oversized_assets_before_planning() {
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
        let state = VaultWorkingTreeStateManifest {
            version: "finite-vault-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "general".to_owned(),
                source_vault_id: None,
                path: "General".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: Vec::new(),
            sync: WorkingTreeSyncState { latest_sequence: 1 },
        };

        let error = scan_working_tree_changes(root, &state).unwrap_err();

        assert!(error.to_string().contains("size limit"));
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
            vault_id: VaultId::new("vault").unwrap(),
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
                vault_id: "vault",
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
            vault_id: VaultId::new("vault").unwrap(),
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
            working_tree_root: None,
            now: Some("2026-06-26T23:30:00Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(temp.path().join("finite-home")),
        };
        let keys = Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")
            .unwrap();
        let actor_npub = NostrPublicKey::from_protocol(keys.public_key())
            .to_npub()
            .unwrap();
        let agent_state = AgentState::new("vault", "2026-06-26T23:30:00Z");
        let mut session_keys = SessionFolderKeyring::default();
        session_keys.insert("vault", "general", 1, FolderKey::from_bytes([1; 32]));
        let current_key_version_by_folder =
            BTreeMap::from([(("vault".to_owned(), "general".to_owned()), 2)]);
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
            source_vault_id: None,
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
    fn asset_plaintext_round_trips_with_hash_and_content_type() {
        let path = SafeRelativePath::new("asset_path", "raw/assets/source.pdf").unwrap();
        let plaintext =
            encode_folder_object_asset_plaintext(&path, b"%PDF test\n", "application/pdf").unwrap();

        match decode_folder_object_plaintext(plaintext.into_bytes(), "fallback.md".to_owned())
            .unwrap()
        {
            CliDecodedFolderObjectPlaintext::Asset {
                path,
                bytes,
                content_type,
            } => {
                assert_eq!(path, "raw/assets/source.pdf");
                assert_eq!(bytes, b"%PDF test\n");
                assert_eq!(content_type, "application/pdf");
            }
            CliDecodedFolderObjectPlaintext::Page { .. } => panic!("expected asset plaintext"),
        }
    }

    #[test]
    fn empty_readable_folders_stay_materialized() {
        let vault = Vault {
            id: VaultId::new("vault").unwrap(),
            kind: VaultKind::Personal,
            name: DisplayName::new("vault_name", "Vault").unwrap(),
            owner_user_id: Some(UserId::new("npub-owner").unwrap()),
            folders: vec![Folder {
                id: FolderId::new("home").unwrap(),
                name: DisplayName::new("folder_name", "home").unwrap(),
                role: FolderRole::PersonalHome,
                access: FolderAccessMode::Owner,
                parent_folder_id: None,
                path: SafeRelativePath::new("folder_path", "home").unwrap(),
                current_key_version: 1,
                shared_folder_source: false,
            }],
            members: Vec::new(),
            admins: Vec::new(),
        };
        let mut projection = materialize_vault_working_tree(WorkingTreeMaterializeInput {
            generated_at: "2026-06-26T23:30:00Z".to_owned(),
            generated_by_npub: UserId::new("npub-owner").unwrap(),
            vault,
            opened_pages: Vec::new(),
            opened_assets: Vec::new(),
            locked_folders: Vec::new(),
            latest_sequence: 0,
        })
        .unwrap();
        let export = CliEncryptedVaultExport {
            vault: CliExportVault {
                id: "vault".to_owned(),
                kind: "personal".to_owned(),
                name: "Vault".to_owned(),
                owner_user_id: Some("npub-owner".to_owned()),
            },
            folders: vec![CliExportFolder {
                id: "home".to_owned(),
                path: "home".to_owned(),
                access: "owner".to_owned(),
                current_key_version: 1,
                shared_folder_source: false,
                accessible: true,
            }],
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },
        };
        let readable = BTreeSet::from([("vault".to_owned(), "home".to_owned())]);

        add_empty_readable_folders(&mut projection, &export, None, &readable, None).unwrap();

        assert_eq!(projection.state.folder_roots.len(), 1);
        assert_eq!(projection.state.folder_roots[0].folder_id, "home");
        assert!(projection.files.contains_key("home/AGENTS.md"));
        assert!(projection.files.contains_key("home/raw/.keep"));
        assert!(projection.files.contains_key("home/raw/assets/.keep"));
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
    fn stale_object_cleanup_removes_old_path_after_move() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        fs::create_dir_all(root.join("General")).unwrap();
        fs::write(root.join("General/old.md"), "# Old\n").unwrap();
        let state = VaultWorkingTreeStateManifest {
            version: "finite-vault-working-tree-state-v1".to_owned(),
            folder_roots: vec![WorkingTreeFolderRoot {
                folder_id: "general".to_owned(),
                source_vault_id: None,
                path: "General".to_owned(),
                can_read: true,
                metadata_only: false,
            }],
            objects: vec![WorkingTreeObjectManifestEntry {
                folder_id: "general".to_owned(),
                source_vault_id: None,
                path: "old.md".to_owned(),
                object_id: "obj_same0000000".to_owned(),
                revision: 1,
                key_version: 1,
                content_type: "text/markdown".to_owned(),
                content_hash: sha256_hex("# Old\n".as_bytes()),
            }],
            sync: WorkingTreeSyncState { latest_sequence: 1 },
        };
        write_json_file(&root.join(".finitebrain/working-tree-state.json"), &state).unwrap();
        let new_objects = vec![WorkingTreeObjectManifestEntry {
            folder_id: "general".to_owned(),
            source_vault_id: None,
            path: "new.md".to_owned(),
            object_id: "obj_same0000000".to_owned(),
            revision: 2,
            key_version: 1,
            content_type: "text/markdown".to_owned(),
            content_hash: sha256_hex("# New\n".as_bytes()),
        }];

        remove_stale_object_files(root, &state.objects, &new_objects).unwrap();

        assert!(!root.join("General/old.md").exists());
    }

    #[test]
    fn materialize_remote_projection_uses_encrypted_page_path_without_prior_state() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        write_json_file(
            &root.join(".finitebrain/working-tree-state.json"),
            &VaultWorkingTreeStateManifest {
                version: "finite-vault-working-tree-state-v1".to_owned(),
                folder_roots: Vec::new(),
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        let folder_key = FolderKey::from_bytes([3; 32]);
        let mut session_keys = SessionFolderKeyring::default();
        session_keys.insert("vault", "home", 1, folder_key.clone());
        let env = CliEnvironment {
            cwd: root.to_path_buf(),
            config_dir: root.join("config"),
            working_tree_root: None,
            now: Some("2026-06-26T23:30:00Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(root.join("finite-home")),
        };
        let object_id = ObjectId::new("obj_remote000001").unwrap();
        let page_path = SafeRelativePath::new("page_path", "docs/from-envelope.md").unwrap();
        let plaintext = encode_folder_object_page_plaintext(&page_path, "# Remote\n").unwrap();
        let aad = FolderObjectAad {
            vault_id: VaultId::new("vault").unwrap(),
            folder_id: FolderId::new("home").unwrap(),
            object_id: object_id.clone(),
            key_version: 1,
        };
        let envelope = encrypt_folder_object(&folder_key, &aad, &plaintext).unwrap();
        let export = CliEncryptedVaultExport {
            vault: CliExportVault {
                id: "vault".to_owned(),
                kind: "personal".to_owned(),
                name: "Vault".to_owned(),
                owner_user_id: Some("npub-owner".to_owned()),
            },
            folders: vec![CliExportFolder {
                id: "home".to_owned(),
                path: "home".to_owned(),
                access: "owner".to_owned(),
                current_key_version: 1,
                shared_folder_source: false,
                accessible: true,
            }],
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },
        };
        let bootstrap = CliSyncBootstrap {
            latest_sequence: 7,
            objects: vec![CliSyncObject {
                folder_id: "home".to_owned(),
                object_id: object_id.as_str().to_owned(),
                revision: 2,
                ciphertext: envelope.canonical_json(),
                deleted: false,
            }],
        };

        materialize_remote_projection(MaterializeRemoteProjectionContext {
            env: &env,
            root,
            actor_npub: "npub-owner",
            export: &export,
            bootstrap: &bootstrap,
            mounted_folders: &[],
            path_overrides: &BTreeMap::new(),
            session_keys: &session_keys,
        })
        .unwrap();

        assert_eq!(
            fs::read_to_string(root.join("home/docs/from-envelope.md")).unwrap(),
            "# Remote\n"
        );
        let state = read_working_tree_state(root).unwrap();
        assert_eq!(state.objects.len(), 1);
        assert_eq!(state.objects[0].path, "docs/from-envelope.md");
        assert_eq!(state.sync.latest_sequence, 7);
    }

    #[test]
    fn materialize_remote_projection_mounts_source_folder_into_destination_tree() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        initialize_private_working_tree(root).unwrap();
        write_json_file(
            &root.join(".finitebrain/working-tree-state.json"),
            &VaultWorkingTreeStateManifest {
                version: "finite-vault-working-tree-state-v1".to_owned(),
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
            working_tree_root: None,
            now: Some("2026-06-26T23:30:00Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(root.join("finite-home")),
        };
        let object_id = ObjectId::new("obj_mounted00001").unwrap();
        let page_path = SafeRelativePath::new("page_path", "compiled/share-brief.md").unwrap();
        let plaintext = encode_folder_object_page_plaintext(&page_path, "# Share Brief\n").unwrap();
        let aad = FolderObjectAad {
            vault_id: VaultId::new("source").unwrap(),
            folder_id: FolderId::new("shared-lab").unwrap(),
            object_id: object_id.clone(),
            key_version: 1,
        };
        let envelope = encrypt_folder_object(&folder_key, &aad, &plaintext).unwrap();
        let destination_export = CliEncryptedVaultExport {
            vault: CliExportVault {
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
                shared_folder_source: false,
                accessible: true,
            }],
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },
        };
        let source_export = CliEncryptedVaultExport {
            vault: CliExportVault {
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
                shared_folder_source: true,
                accessible: true,
            }],
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },
        };
        let mounted = MountedFolderMaterializeContext {
            mount: CliMountedFolder {
                mount_id: "mount-source-shared-lab".to_owned(),
                source_vault_id: "source".to_owned(),
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
            },
        };

        materialize_remote_projection(MaterializeRemoteProjectionContext {
            env: &env,
            root,
            actor_npub: "npub-dest",
            export: &destination_export,
            bootstrap: &CliSyncBootstrap {
                latest_sequence: 2,
                objects: Vec::new(),
            },
            mounted_folders: &[mounted],
            path_overrides: &BTreeMap::new(),
            session_keys: &session_keys,
        })
        .unwrap();

        assert_eq!(
            fs::read_to_string(root.join("shared-lab/compiled/share-brief.md")).unwrap(),
            "# Share Brief\n"
        );
        let state = read_working_tree_state(root).unwrap();
        let root_entry = state
            .folder_roots
            .iter()
            .find(|root| root.path == "shared-lab")
            .unwrap();
        assert_eq!(root_entry.folder_id, "shared-lab");
        assert_eq!(root_entry.source_vault_id.as_deref(), Some("source"));
        let object_entry = state
            .objects
            .iter()
            .find(|object| object.path == "compiled/share-brief.md")
            .unwrap();
        assert_eq!(object_entry.source_vault_id.as_deref(), Some("source"));

        let intents = plan_working_tree_change_intents(
            &state,
            &[WorkingTreeChange::Upsert {
                path: SafeRelativePath::new("change_path", "shared-lab/compiled/share-brief.md")
                    .unwrap(),
                markdown: "# Updated\n".to_owned(),
            }],
        );
        assert_eq!(
            intents[0].source_vault_id.as_ref().map(ToString::to_string),
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
            &VaultWorkingTreeStateManifest {
                version: "finite-vault-working-tree-state-v1".to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "home".to_owned(),
                    source_vault_id: None,
                    path: "home".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: vec![WorkingTreeObjectManifestEntry {
                    folder_id: "home".to_owned(),
                    source_vault_id: None,
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
        session_keys.insert("vault", "home", 1, FolderKey::from_bytes([1; 32]));
        let env = CliEnvironment {
            cwd: root.to_path_buf(),
            config_dir: root.join("config"),
            working_tree_root: None,
            now: Some("2026-06-26T23:30:00Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(root.join("finite-home")),
        };
        let export = CliEncryptedVaultExport {
            vault: CliExportVault {
                id: "vault".to_owned(),
                kind: "personal".to_owned(),
                name: "Vault".to_owned(),
                owner_user_id: Some("npub-owner".to_owned()),
            },
            folders: vec![CliExportFolder {
                id: "home".to_owned(),
                path: "home".to_owned(),
                access: "owner".to_owned(),
                current_key_version: 2,
                shared_folder_source: false,
                accessible: true,
            }],
            key_grants: Vec::new(),
            access_state: CliExportAccessState {
                members: Vec::new(),
                admins: Vec::new(),
            },
        };
        let bootstrap = CliSyncBootstrap {
            latest_sequence: 0,
            objects: Vec::new(),
        };

        materialize_remote_projection(MaterializeRemoteProjectionContext {
            env: &env,
            root,
            actor_npub: "npub-owner",
            export: &export,
            bootstrap: &bootstrap,
            mounted_folders: &[],
            path_overrides: &BTreeMap::new(),
            session_keys: &session_keys,
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
    fn _directory_manifest() -> VaultDirectoryManifest {
        VaultDirectoryManifest {
            version: "finite-vault-directory-v1".to_owned(),
            vault: VaultDirectoryVaultSummary {
                id: "vault".to_owned(),
                kind: "personal".to_owned(),
                name: "Vault".to_owned(),
                owner_npub: Some("npub-owner".to_owned()),
            },
            working_tree: VaultDirectoryPath {
                path: ".".to_owned(),
            },
            encrypted_sync: VaultDirectoryPath {
                path: ".finitebrain/encrypted-sync".to_owned(),
            },
            portability: VaultDirectoryPortability {
                owned_by_agent_runtime: true,
                owned_by_app_surface: false,
            },
            created_at: "2026-06-26T23:30:00Z".to_owned(),
            updated_at: "2026-06-26T23:30:00Z".to_owned(),
        }
    }
}
