use crate::*;

pub(crate) async fn replace_personal_agent_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
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
    let brain_id = BrainId::new(brain_id)?;
    let actor_id = UserId::new(actor.clone())?;
    {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        if stored.brain.kind != BrainKind::Personal
            || stored.brain.owner_user_id.as_ref() != Some(&actor_id)
        {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "only the Personal Brain owner may replace its Personal Agent",
            ));
        }
    }
    let replacement_identity = request
        .agent_email
        .as_deref()
        .map(|email| resolve_managed_agent_email(&state, email, &actor_id))
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
            &brain_id,
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
            &brain_id,
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
        &brain_id,
        &actor_id,
        replacement.as_ref(),
        &rotations,
        &updated_at,
    )?;
    let stored = store.load_brain(&brain_id)?;
    let mut response = metadata_response(stored);
    enrich_metadata_identities(&store, &mut response)?;
    Ok(Json(response))
}

pub(crate) async fn bootstrap_personal_brain_for_agent_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Json<BootstrapPersonalBrainForAgentResponse>, ApiError> {
    let agent_actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let _request: BootstrapPersonalBrainForAgentRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let agent_npub = UserId::new(agent_actor)?;
    let principals = resolve_account_agent_principals(&state, &agent_npub)?;
    let owner_key =
        NostrPublicKey::parse(principals.owner_npub.as_str()).map_err(nostr_identity_error)?;
    let brain_id = BrainId::new(format!("personal-{}", &owner_key.to_hex()[..16]))?;
    let output = bootstrap_personal_brain(
        brain_id.as_str(),
        "Personal Brain",
        principals.owner_npub.to_string(),
    )?;
    let created_at = server_timestamp(&state);
    let identity_aliases = account_agent_identity_aliases(&principals, &created_at)?;
    {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.create_personal_brain_bootstrap_with_identities(
            &output,
            &[],
            &principals.agent_npub,
            &principals.agent_npub,
            &created_at,
            &identity_aliases,
        )?;
    }

    let mut brain = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        let mut response = metadata_response(stored);
        enrich_metadata_identities(&store, &mut response)?;
        response
    };
    brain
        .identities
        .sort_by(|left, right| left.npub.cmp(&right.npub));
    Ok(Json(BootstrapPersonalBrainForAgentResponse { brain }))
}
