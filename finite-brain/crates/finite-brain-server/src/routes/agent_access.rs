use crate::*;

pub(crate) async fn replace_personal_agent_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<VaultMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: ReplacePersonalAgentRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    validate_folder_rotation_fanout(
        FolderRotationOperation::PersonalAgent,
        request
            .rotations
            .iter()
            .map(|rotation| FolderRotationFanout {
                grants: rotation.grants.len(),
                reencrypted_records: rotation.reencrypted_records.len(),
            }),
    )?;
    let vault_id = VaultId::new(vault_id)?;
    let actor_id = UserId::new(actor.clone())?;
    {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        if stored.vault.kind != VaultKind::Personal
            || stored.vault.owner_user_id.as_ref() != Some(&actor_id)
        {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "only the Personal Vault owner may replace its Personal Agent",
            ));
        }
    }
    let replacement_identity = request
        .agent_email
        .as_deref()
        .map(|email| resolve_managed_agent_email(&state, email))
        .transpose()?;
    let replacement = match replacement_identity.as_ref() {
        Some(identity) => Some(UserId::new(identity.npub.clone())?),
        None => None,
    };
    let mut rotations = Vec::with_capacity(request.rotations.len());
    for rotation in request.rotations {
        let folder_id = FolderId::new(rotation.folder_id)?;
        let (event, payload) = validate_admin_access_change_value(
            rotation.access_change_event,
            &vault_id,
            &actor,
            AdminAccessAction::RotateFolderKey,
            Some(&folder_id),
            replacement.as_ref().map(UserId::as_str),
            Some(rotation.new_key_version),
        )?;
        let grants = grant_requests_to_metadata(
            &rotation.grants,
            &folder_id,
            &actor,
            Some(event.as_json()),
            &server_timestamp(&state),
        )?;
        let records = rotation_records_from_requests(
            &vault_id,
            &folder_id,
            &actor,
            rotation.new_key_version,
            rotation.reencrypted_records,
        )?;
        let mut control_records = grants
            .iter()
            .map(folder_key_grant_sync_record)
            .map(|record| match record {
                Ok(SyncRecordInput::Control(record)) => Ok(record),
                Ok(_) => unreachable!("Folder Key Grant helper always returns a control record"),
                Err(error) => Err(error),
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        control_records.push(
            match admin_access_change_sync_record(&actor, &event, &payload)? {
                SyncRecordInput::Control(record) => record,
                _ => unreachable!("admin access helper always returns a control record"),
            },
        );
        rotations.push(PersonalAgentFolderRotation {
            folder_id,
            new_key_version: rotation.new_key_version,
            grants,
            reencrypted_records: records,
            control_records,
        });
    }
    if let Some(identity) = replacement_identity {
        record_resolved_identity(&state, identity)?;
    }
    let updated_at = server_timestamp(&state);
    let mut store = state.store.lock().map_err(lock_error)?;
    store.replace_personal_agent(
        &vault_id,
        &actor_id,
        replacement.as_ref(),
        &rotations,
        &updated_at,
    )?;
    let stored = store.load_vault(&vault_id)?;
    let mut response = metadata_response(stored);
    enrich_metadata_identities(&store, &mut response)?;
    Ok(Json(response))
}

pub(crate) async fn bootstrap_personal_vault_for_agent_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Json<BootstrapPersonalVaultForAgentResponse>, ApiError> {
    let agent_actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let _request: BootstrapPersonalVaultForAgentRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let agent_npub = UserId::new(agent_actor)?;
    let principals = resolve_account_agent_principals(&state, &agent_npub)?;
    let owner_key =
        NostrPublicKey::parse(principals.owner_npub.as_str()).map_err(nostr_identity_error)?;
    let vault_id = VaultId::new(format!("personal-{}", &owner_key.to_hex()[..16]))?;
    let output = bootstrap_personal_vault(
        vault_id.as_str(),
        "Personal vault",
        principals.owner_npub.to_string(),
    )?;
    let created_at = server_timestamp(&state);
    let identity_aliases = account_agent_identity_aliases(&principals, &created_at)?;
    {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.create_personal_vault_bootstrap_with_identities(
            &output,
            &[],
            &principals.agent_npub,
            &principals.agent_npub,
            &created_at,
            &identity_aliases,
        )?;
    }

    let mut vault = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        let mut response = metadata_response(stored);
        enrich_metadata_identities(&store, &mut response)?;
        response
    };
    vault
        .identities
        .sort_by(|left, right| left.npub.cmp(&right.npub));
    Ok(Json(BootstrapPersonalVaultForAgentResponse { vault }))
}
