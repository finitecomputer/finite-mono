use std::collections::BTreeSet;
use std::fs;

use finite_brain_core::portability::WorkingTreeFolderRoot;
use finite_brain_core::{
    AdminAccessAction, AdminAccessChangePayload, AdminAccessChangeValidation, BrainId, BrainKind,
    FolderAccessMode, FolderId, FolderKey, FolderKeyGrantPayload, FolderKeyRecipientPolicy,
    SafeRelativePath, UserId, required_folder_key_recipients,
};
use finite_nostr::{NostrPublicKey, build_rumor, wrap_rumor};
use nostr::{Kind, Tag};

use crate::{
    APP_SPECIFIC_KIND, BrainMetadataView, CliEnvironment, CliError, LocalSigner,
    SessionFolderKeyring, deterministic_id, find_agent_state, load_signer, mutate_agent_state,
    normalize_folder_access, open_brain_session_folder_keys, read_agent_state,
    read_working_tree_state, sign_event, signed_json_request, tag_vec, timestamp, unix_timestamp,
    write_working_tree_state,
};

pub(crate) fn fetch_brain_metadata(
    env: &CliEnvironment,
    args: &[String],
    brain_id: &str,
) -> Result<BrainMetadataView, CliError> {
    let path = format!("/_admin/brains/{brain_id}/metadata");
    let response = signed_json_request(env, args, "GET", &path, None)?;
    serde_json::from_value(response).map_err(CliError::from)
}

/// Plan and submit one convergent Organization Brain collaboration request.
/// Folder Keys are opened only in the local session keyring and converted to
/// opaque recipient-wrapped grants before crossing the HTTP boundary.
pub(crate) fn ensure_organization_admin(
    env: &CliEnvironment,
    args: &[String],
    brain_id: &str,
    target_input: &str,
) -> Result<serde_json::Value, CliError> {
    let target = resolve_identity_npub(env, args, target_input)?;
    let metadata = fetch_brain_metadata(env, args, brain_id)?;
    if metadata.kind != "organization" {
        return Err(CliError::InvalidInput(
            "collaborators ensure-admin requires an Organization Brain".to_owned(),
        ));
    }
    let keyring = open_brain_session_folder_keys(env, args, brain_id)?;
    let auth = load_signer(env)?;
    let mut folders = Vec::with_capacity(metadata.folders.len());
    let mut grants = Vec::new();
    for folder in &metadata.folders {
        folders.push(serde_json::json!({
            "folderId": folder.id,
            "keyVersion": folder.current_key_version,
            "path": folder.path,
        }));
        if let Some(folder_key) = keyring.get(brain_id, &folder.id, folder.current_key_version) {
            let grant = folder_key_grant_request(
                &auth,
                brain_id,
                &folder.id,
                folder.current_key_version,
                &target,
                folder_key,
                env,
            )?;
            grants.push(serde_json::json!({
                "folderId": folder.id,
                "id": grant["id"],
                "keyVersion": grant["keyVersion"],
                "recipientNpub": grant["recipientNpub"],
                "wrappedEventJson": grant["wrappedEventJson"],
                "createdAt": grant["createdAt"],
            }));
        }
    }
    let access_change_event = admin_access_change_event(
        env,
        brain_id,
        AdminAccessAction::AddAdmin,
        None,
        Some(&target),
        None,
    )?;
    let route = format!("/_admin/brains/{brain_id}/collaborators/ensure-admin");
    let request = signed_json_request(
        env,
        args,
        "POST",
        &route,
        Some(serde_json::json!({
            "targetNpub": target.clone(),
            "folders": folders.clone(),
            "grants": grants,
            "accessChangeEvent": access_change_event,
        })),
    );
    match request {
        Ok(response) => Ok(response),
        Err(CliError::Http(_)) => Ok(serde_json::json!({
            "brainId": brain_id,
            "targetNpub": target,
            "state": "indeterminate",
            "brainRole": "unknown",
            "folders": folders.into_iter().map(|folder| serde_json::json!({
                "folderId": folder["folderId"],
                "path": folder["path"],
                "expectedKeyVersion": folder["keyVersion"],
                "outcome": "failed",
                "reason": "transportUncertain",
                "retryable": true,
            })).collect::<Vec<_>>(),
            "readyCount": 0,
            "totalCount": metadata.folders.len(),
            "retryable": true,
        })),
        Err(error) => Err(error),
    }
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
    metadata: &BrainMetadataView,
    access: &str,
    access_users: &[String],
) -> Result<Vec<String>, CliError> {
    let brain_kind = match metadata.kind.as_str() {
        "personal" => BrainKind::Personal,
        "organization" => BrainKind::Organization,
        other => {
            return Err(CliError::InvalidInput(format!(
                "unknown brain kind {other}"
            )));
        }
    };
    let folder_access = match normalize_folder_access(access)? {
        "owner" => FolderAccessMode::Owner,
        "admin_only" => FolderAccessMode::AdminOnly,
        "all_members" => FolderAccessMode::AllMembers,
        "restricted" => FolderAccessMode::Restricted,
        other => {
            return Err(CliError::InvalidInput(format!(
                "unknown folder access mode {other}"
            )));
        }
    };
    let owner = metadata
        .owner_user_id
        .as_deref()
        .map(UserId::new)
        .transpose()
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let admins = metadata
        .admins
        .iter()
        .cloned()
        .map(UserId::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let members = metadata
        .members
        .iter()
        .cloned()
        .map(UserId::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let explicit_access_user_ids = access_users
        .iter()
        .cloned()
        .map(UserId::new)
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let personal_agent = metadata
        .personal_agent
        .as_ref()
        .map(|agent| UserId::new(agent.agent_npub.clone()))
        .transpose()
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;

    required_folder_key_recipients(FolderKeyRecipientPolicy {
        brain_kind,
        folder_access,
        owner_user_id: owner.as_ref(),
        admins: &admins,
        members: &members,
        explicit_access_user_ids: &explicit_access_user_ids,
        personal_agent_npub: personal_agent.as_ref(),
    })
    .map(|recipients| {
        recipients
            .into_iter()
            .map(|user| user.to_string())
            .collect()
    })
    .map_err(|error| CliError::InvalidInput(error.to_string()))
}

pub(crate) fn folder_key_grant_request(
    auth: &LocalSigner,
    brain_id: &str,
    folder_id: &str,
    key_version: u32,
    recipient_npub: &str,
    folder_key: &FolderKey,
    env: &CliEnvironment,
) -> Result<serde_json::Value, CliError> {
    let keys = auth.keys.clone();
    let recipient = NostrPublicKey::parse(recipient_npub)
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    let created_at = timestamp(env);
    let grant_id = deterministic_id(
        "grant",
        &[
            brain_id,
            folder_id,
            &key_version.to_string(),
            recipient_npub,
            &created_at,
        ],
    );
    let content = FolderKeyGrantPayload {
        version: "finite-folder-key-grant-v1".to_owned(),
        brain_id: brain_id.to_owned(),
        folder_id: folder_id.to_owned(),
        key_version,
        folder_key: folder_key.to_base64(),
        issuer_npub: auth.npub.clone(),
        recipient_npub: recipient_npub.to_owned(),
        created_at: created_at.clone(),
    }
    .canonical_json();
    let rumor = build_rumor(
        NostrPublicKey::from_protocol(keys.public_key()),
        Kind::Custom(APP_SPECIFIC_KIND),
        vec![
            tag_vec([
                "d",
                &format!("finite-folder-key-grant:{brain_id}:{folder_id}:{key_version}"),
            ])?,
            tag_vec(["brain", brain_id])?,
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
        "createdAt": created_at
    }))
}

pub(crate) fn opened_folder_key(
    keyring: &SessionFolderKeyring,
    brain_id: &str,
    folder_id: &str,
    key_version: u32,
) -> Result<FolderKey, CliError> {
    keyring
        .get(brain_id, folder_id, key_version)
        .cloned()
        .ok_or_else(|| {
            CliError::GrantOpening {
                brain_id: brain_id.to_owned(),
                folder_id: folder_id.to_owned(),
                key_version,
                reason: "no usable current grant was available for this operation; ensure the acting Member Identity has a valid encrypted grant"
                    .to_owned(),
            }
        })
}

pub(crate) fn admin_access_change_event(
    env: &CliEnvironment,
    brain_id: &str,
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
            brain_id,
            action.as_str(),
            folder_id.unwrap_or("-"),
            target_npub.unwrap_or("-"),
            &timestamp(env),
        ],
    );
    let validation = AdminAccessChangeValidation {
        brain_id: BrainId::new(brain_id.to_owned())
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
                "finite-brain-admin-access-change:{}:{}",
                input.brain_id, input.change_id
            ),
        ])?,
        tag_vec(["brain", &input.brain_id.to_string()])?,
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
            source_brain_id: None,
            path: path.to_owned(),
            can_read: true,
            metadata_only: false,
        });
        tree.folder_roots
            .sort_by(|left, right| left.path.cmp(&right.path));
        write_working_tree_state(&root, &tree)?;
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
        state.add_activity(now, "folder.created", format!("Folder {folder_id} created"));
        Ok(())
    })
}

pub(crate) fn update_local_folders_after_delete(
    env: &CliEnvironment,
    brain_id: &str,
    deleted_folder_ids: &[String],
) -> Result<(), CliError> {
    let Some(root) = find_agent_state(&env.cwd)? else {
        return Ok(());
    };
    let agent = read_agent_state(&root)?;
    if agent.brain_id != brain_id {
        return Ok(());
    }
    let deleted = deleted_folder_ids
        .iter()
        .map(|folder_id| {
            FolderId::new(folder_id.clone())
                .map(|folder_id| folder_id.to_string())
                .map_err(|error| CliError::InvalidInput(error.to_string()))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if deleted.is_empty() {
        return Err(CliError::InvalidInput(
            "Folder deletion response did not identify the deleted subtree".to_owned(),
        ));
    }

    let mut tree = read_working_tree_state(&root)?;
    let deleted_paths = tree
        .folder_roots
        .iter()
        .filter(|folder| folder.source_brain_id.is_none() && deleted.contains(&folder.folder_id))
        .map(|folder| {
            SafeRelativePath::new("folder_path", folder.path.clone())
                .map_err(|error| CliError::InvalidInput(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for path in deleted_paths {
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
    tree.folder_roots
        .retain(|folder| folder.source_brain_id.is_some() || !deleted.contains(&folder.folder_id));
    tree.objects
        .retain(|object| object.source_brain_id.is_some() || !deleted.contains(&object.folder_id));
    write_working_tree_state(&root, &tree)?;
    mutate_agent_state(env, |state, now| {
        state.add_activity(
            now,
            "folder.deleted",
            format!("Deleted {} Folder projection(s)", deleted.len()),
        );
        Ok(())
    })
}
