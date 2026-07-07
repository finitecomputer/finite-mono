use std::collections::BTreeSet;
use std::fs;

use finite_brain_core::portability::WorkingTreeFolderRoot;
use finite_brain_core::{
    AdminAccessAction, AdminAccessChangePayload, AdminAccessChangeValidation, FolderId, FolderKey,
    SafeRelativePath, VaultId,
};
use finite_nostr::{NostrPublicKey, build_rumor, wrap_rumor};
use nostr::{Kind, Tag};

use crate::{
    APP_SPECIFIC_KIND, CliEnvironment, CliError, LocalFolderKey, LocalSigner, UnlockedFolder,
    VaultMetadataView, current_tree_root, deterministic_id, find_agent_state, load_signer,
    mutate_agent_state, normalize_folder_access, read_agent_state, read_working_tree_state,
    sign_event, signed_json_request, tag_vec, timestamp, unix_timestamp, write_json_file,
};

pub(crate) fn fetch_vault_metadata(
    env: &CliEnvironment,
    args: &[String],
    vault_id: &str,
) -> Result<VaultMetadataView, CliError> {
    let path = format!("/_admin/vaults/{vault_id}/metadata");
    let response = signed_json_request(env, args, "GET", &path, None)?;
    serde_json::from_value(response).map_err(CliError::from)
}

pub(crate) fn resolve_identity_npub(
    env: &CliEnvironment,
    args: &[String],
    input: &str,
) -> Result<String, CliError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(CliError::InvalidInput(
            "identity input is required".to_owned(),
        ));
    }
    if let Ok(public_key) = NostrPublicKey::parse(input) {
        return public_key
            .to_npub()
            .map_err(|error| CliError::InvalidSigner(error.to_string()));
    }
    let response = signed_json_request(
        env,
        args,
        "POST",
        "/_admin/identities/resolve",
        Some(serde_json::json!({ "input": input })),
    )?;
    response
        .get("npub")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            CliError::InvalidInput("identity resolve response did not include npub".to_owned())
        })
}

pub(crate) fn folder_required_recipients(
    metadata: &VaultMetadataView,
    access: &str,
    access_users: &[String],
) -> Result<Vec<String>, CliError> {
    let mut recipients = BTreeSet::new();
    match normalize_folder_access(access)? {
        "owner" => {
            let owner = metadata.owner_user_id.clone().ok_or_else(|| {
                CliError::InvalidInput("owner access requires a personal vault".to_owned())
            })?;
            recipients.insert(owner);
        }
        "admin_only" => {
            recipients.extend(metadata.admins.iter().cloned());
        }
        "all_members" => {
            recipients.extend(metadata.admins.iter().cloned());
            recipients.extend(metadata.members.iter().cloned());
        }
        "restricted" => {
            recipients.extend(metadata.owner_user_id.iter().cloned());
            recipients.extend(metadata.admins.iter().cloned());
            recipients.extend(access_users.iter().cloned());
        }
        other => {
            return Err(CliError::InvalidInput(format!(
                "unknown folder access mode {other}"
            )));
        }
    }
    if recipients.is_empty() {
        return Err(CliError::InvalidInput(
            "folder key needs at least one recipient".to_owned(),
        ));
    }
    Ok(recipients.into_iter().collect())
}

pub(crate) fn folder_key_grant_request(
    auth: &LocalSigner,
    vault_id: &str,
    folder_id: &str,
    key_version: u32,
    recipient_npub: &str,
    folder_key: &FolderKey,
    env: &CliEnvironment,
) -> Result<serde_json::Value, CliError> {
    let keys = auth.keys.clone();
    let recipient = NostrPublicKey::parse(recipient_npub)
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    let grant_id = deterministic_id(
        "grant",
        &[
            vault_id,
            folder_id,
            &key_version.to_string(),
            recipient_npub,
            &timestamp(env),
        ],
    );
    let content = serde_json::json!({
        "version": "finite-folder-key-grant-v1",
        "vaultId": vault_id,
        "folderId": folder_id,
        "keyVersion": key_version,
        "folderKey": folder_key.to_base64(),
        "issuerNpub": auth.npub,
        "recipientNpub": recipient_npub,
        "createdAt": timestamp(env)
    })
    .to_string();
    let rumor = build_rumor(
        NostrPublicKey::from_protocol(keys.public_key()),
        Kind::Custom(APP_SPECIFIC_KIND),
        vec![
            tag_vec([
                "d",
                &format!("finite-folder-key-grant:{vault_id}:{folder_id}:{key_version}"),
            ])?,
            tag_vec(["vault", vault_id])?,
            tag_vec(["folder", folder_id])?,
            tag_vec(["keyVersion", &key_version.to_string()])?,
        ],
        content,
        unix_timestamp(),
    );
    let wrapped = wrap_rumor(&keys, recipient, rumor)
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    Ok(serde_json::json!({
        "id": grant_id,
        "keyVersion": key_version,
        "recipientNpub": recipient_npub,
        "wrappedEventJson": wrapped.as_json(),
        "createdAt": timestamp(env)
    }))
}

pub(crate) fn opened_folder_key(
    env: &CliEnvironment,
    folder_id: &str,
    key_version: u32,
) -> Result<FolderKey, CliError> {
    let root = current_tree_root(env)?;
    let state = read_agent_state(&root)?;
    let local_key = state
        .local_folder_keys
        .iter()
        .find(|key| {
            key.vault_id.as_deref().unwrap_or(state.vault_id.as_str())
                == state.vault_id.as_str()
                && key.folder_id == folder_id
                && key.key_version == key_version
        })
        .ok_or_else(|| {
            CliError::InvalidInput(format!(
                "Folder Key for {folder_id} v{key_version} is not opened locally; run fbrain open/sync as an admin first"
            ))
        })?;
    FolderKey::from_base64(&local_key.key_base64)
        .map_err(|error| CliError::InvalidInput(error.to_string()))
}

pub(crate) fn admin_access_change_event(
    env: &CliEnvironment,
    vault_id: &str,
    action: AdminAccessAction,
    folder_id: Option<&str>,
    target_npub: Option<&str>,
    key_version: Option<u32>,
) -> Result<serde_json::Value, CliError> {
    let auth = load_signer(env)?;
    let keys = auth.keys.clone();
    let change_id = deterministic_id(
        "access-change",
        &[
            vault_id,
            action.as_str(),
            folder_id.unwrap_or("-"),
            target_npub.unwrap_or("-"),
            &timestamp(env),
        ],
    );
    let validation = AdminAccessChangeValidation {
        vault_id: VaultId::new(vault_id.to_owned())
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        change_id,
        action,
        admin_npub: auth.npub,
        folder_id: folder_id
            .map(|id| FolderId::new(id.to_owned()))
            .transpose()
            .map_err(|error| CliError::InvalidInput(error.to_string()))?,
        target_npub: target_npub.map(ToOwned::to_owned),
        key_version,
        note: None,
        created_at: timestamp(env),
    };
    let payload = AdminAccessChangePayload::new(&validation);
    let event = sign_event(
        &keys,
        Kind::Custom(APP_SPECIFIC_KIND),
        payload.canonical_json(),
        admin_access_change_tags(&validation)?,
        unix_timestamp(),
        Some("admin-access-change"),
    )?;
    serde_json::from_str(&event.as_json()).map_err(CliError::from)
}

pub(crate) fn admin_access_change_tags(
    input: &AdminAccessChangeValidation,
) -> Result<Vec<Tag>, CliError> {
    let mut tags = vec![
        tag_vec([
            "d",
            &format!(
                "finite-vault-admin-access-change:{}:{}",
                input.vault_id, input.change_id
            ),
        ])?,
        tag_vec(["vault", &input.vault_id.to_string()])?,
        tag_vec(["action", input.action.as_str()])?,
    ];
    if let Some(folder_id) = &input.folder_id {
        tags.push(tag_vec(["folder", &folder_id.to_string()])?);
    }
    if let Some(target_npub) = &input.target_npub {
        let target = NostrPublicKey::parse(target_npub)
            .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
        tags.push(tag_vec(["p", &target.to_hex()])?);
    }
    if let Some(key_version) = input.key_version {
        tags.push(tag_vec(["keyVersion", &key_version.to_string()])?);
    }
    Ok(tags)
}

pub(crate) fn update_local_folder_after_create(
    env: &CliEnvironment,
    folder_id: &str,
    path: &str,
    folder_key: &FolderKey,
) -> Result<(), CliError> {
    let Some(root) = find_agent_state(&env.cwd)? else {
        return Ok(());
    };
    SafeRelativePath::new("folder_path", path.to_owned())
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let mut tree = read_working_tree_state(&root)?;
    if !tree
        .folder_roots
        .iter()
        .any(|candidate| candidate.folder_id == folder_id)
    {
        tree.folder_roots.push(WorkingTreeFolderRoot {
            folder_id: folder_id.to_owned(),
            source_vault_id: None,
            path: path.to_owned(),
            can_read: true,
            metadata_only: false,
        });
        tree.folder_roots
            .sort_by(|left, right| left.path.cmp(&right.path));
        write_json_file(&root.join(".finitebrain/working-tree-state.json"), &tree)?;
    }
    for subdir in [
        "",
        "raw",
        "raw/assets",
        "wiki",
        "inventory",
        "datasets",
        "output",
    ] {
        fs::create_dir_all(root.join(path).join(subdir))?;
    }
    mutate_agent_state(env, |state, now| {
        if !state
            .local_folder_keys
            .iter()
            .any(|key| key.folder_id == folder_id && key.key_version == 1)
        {
            state.local_folder_keys.push(LocalFolderKey {
                vault_id: Some(state.vault_id.clone()),
                folder_id: folder_id.to_owned(),
                key_version: 1,
                key_base64: folder_key.to_base64(),
                source: "created-by-fbrain".to_owned(),
                opened_at: now.clone(),
            });
        }
        if !state
            .unlocked_folders
            .iter()
            .any(|folder| folder.folder_id == folder_id)
        {
            state.unlocked_folders.push(UnlockedFolder {
                vault_id: Some(state.vault_id.clone()),
                folder_id: folder_id.to_owned(),
                key_version: 1,
                opened_at: now.clone(),
                source: "created-by-fbrain".to_owned(),
            });
        }
        state.add_activity(now, "folder.created", format!("Folder {folder_id} created"));
        Ok(())
    })
}
