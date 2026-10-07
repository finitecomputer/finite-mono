use crate::*;
use finite_brain_store::{MemberFolderRotation, MemberMountRotation};

/// Rename the display name using the existing signed administrative contract.
pub(crate) async fn rename_brain_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: AdminEventRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::RenameBrain,
        None,
        None,
        None,
    )?;
    let raw_name = payload.note.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "rename-brain requires a signed display name in note",
        )
    })?;
    if raw_name.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Brain name cannot be blank",
        ));
    }
    let name = finite_brain_core::DisplayName::new("brain_name", raw_name)?;
    let record = admin_access_change_sync_record(&actor, &event, &payload)?;
    let notification_state = state.clone();
    let notification_brain_id = brain_id.clone();
    let response = run_as_admin(state, brain_id, actor, |store, brain_id| {
        store.rename_brain(brain_id, &name, &record)
    })?;
    notification_state.publish_access_update(&notification_brain_id);
    Ok(Json(response))
}

pub(crate) async fn list_brains_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<VisibleBrainsResponse>, ApiError> {
    let actor_npub = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor_npub)?;
    let brains = {
        let store = state.store.lock().map_err(lock_error)?;
        store.list_visible_brains(&actor)?
    };

    Ok(Json(visible_brains_response(brains)))
}

pub(crate) async fn create_brain_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor_npub = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: CreateBrainRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;

    // Account-bound agent selection by email, and the initial Organization
    // Brain agent, were resolved through Core/Identity, which this server no
    // longer consults (auth kernel cut). Agents join an Organization Brain by
    // npub invitation or capability Invite Token after creation.
    if request.personal_agent_email.is_some()
        || request.initial_agent_email.is_some()
        || request.initial_agent_npub.is_some()
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "account-bound agent selection is no longer resolved by the Brain server; invite the agent by npub or capability Invite Token after creation",
        ));
    }

    let organization_requester = match request.kind {
        CreateBrainKind::Personal => {
            return create_personal_brain(&state, &actor_npub, request).map(Json);
        }
        _ if request.personal_agent_npub.is_some() || request.personal_agent_consent.is_some() => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Personal Agent fields are only valid for a Personal Brain",
            ));
        }
        // The requester is declared provenance, not an authority-verified
        // identity: the Agent Runtime's turn-scoped lease remains the
        // client-side guard, and every initial admin can add admins anyway.
        CreateBrainKind::Organization => request
            .requesting_user_npub
            .as_deref()
            .map(canonical_requesting_user_npub)
            .transpose()?
            .map(UserId::new)
            .transpose()?,
    };

    let output = if let Some(requester) = organization_requester {
        bootstrap_organization_brain_with_requester(
            request.brain_id,
            request.name,
            actor_npub.clone(),
            requester.as_str().to_owned(),
        )?
    } else {
        bootstrap_organization_brain(request.brain_id, request.name, actor_npub.clone())?
    };
    let brain_id = output.brain.id.clone();
    let grants = if request.bootstrap_grants.is_empty() {
        grants_for_required(&output.required_key_grants, &brain_id, &actor_npub)
    } else {
        validate_bootstrap_grant_requests(&request.bootstrap_grants, &output.required_key_grants)?;
        bootstrap_grant_requests_to_metadata(
            &request.bootstrap_grants,
            &actor_npub,
            &server_timestamp(&state),
        )?
    };

    let stored = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.create_brain_bootstrap(&output, &grants)?;
        store.load_brain(&brain_id)?
    };

    let mut response = metadata_response(stored);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_metadata_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

/// Create the signing owner's Personal Brain with the Agent that consented to
/// serve it. Both keys sign: the owner signs this request and the Agent signs
/// the consent, so nobody can claim another person's Agent.
fn create_personal_brain(
    state: &ServerState,
    actor_npub: &str,
    request: CreateBrainRequest,
) -> Result<BrainMetadataResponse, ApiError> {
    if request.requesting_user_npub.is_some() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Organization Brain requester identity is only valid for an Organization Brain",
        ));
    }
    if !request.bootstrap_grants.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "a Personal Brain starts without Folders, so it takes no bootstrap grants",
        ));
    }
    let missing = || {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "Personal Brain creation requires personalAgentNpub and that Agent's signed personalAgentConsent",
        )
    };
    let agent_npub =
        canonical_personal_agent_npub(request.personal_agent_npub.as_deref().ok_or_else(missing)?)?;
    let consent_value = request.personal_agent_consent.ok_or_else(missing)?;
    let event = Event::from_json(consent_value.to_string()).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "personalAgentConsent is not a signed Nostr event",
        )
    })?;
    let consent = verify_personal_agent_consent_event(&event).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid personalAgentConsent: {error}"),
        )
    })?;
    let forbidden = |reason: &str| ApiError::new(StatusCode::FORBIDDEN, reason.to_owned());
    if consent.agent_npub != agent_npub {
        return Err(forbidden(
            "personalAgentConsent was signed by a different Agent",
        ));
    }
    if consent.owner_npub != actor_npub {
        return Err(forbidden(
            "personalAgentConsent names a different owner than the signer",
        ));
    }
    if consent.brain_id != request.brain_id {
        return Err(forbidden(
            "personalAgentConsent names a different Personal Brain",
        ));
    }
    if consent.brain_server != state.public_base_url.trim_end_matches('/') {
        return Err(forbidden(
            "personalAgentConsent names a different Brain server",
        ));
    }
    let now = state.auth_now_unix_seconds();
    if now > consent.expires_at {
        return Err(forbidden("personalAgentConsent has expired"));
    }
    if event.created_at.as_secs() > now.saturating_add(state.max_auth_skew_seconds) {
        return Err(forbidden("personalAgentConsent is signed in the future"));
    }

    let output = bootstrap_personal_brain(request.brain_id, request.name, actor_npub)?;
    let brain_id = output.brain.id.clone();
    let owner = UserId::new(actor_npub)?;
    let agent = UserId::new(agent_npub)?;
    let created_at = server_timestamp(state);
    let mut store = state.store.lock().map_err(lock_error)?;
    store.create_personal_brain_bootstrap(&output, &[], &agent, &owner, &created_at)?;
    let mut response = metadata_response(store.load_brain(&brain_id)?);
    enrich_metadata_identities(&store, &mut response)?;
    Ok(response)
}

fn canonical_personal_agent_npub(value: &str) -> Result<String, ApiError> {
    NostrPublicKey::parse(value)
        .and_then(|public_key| public_key.to_npub())
        .map_err(|error| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("invalid personalAgentNpub: {error}"),
            )
        })
}

fn canonical_requesting_user_npub(value: &str) -> Result<String, ApiError> {
    let public_key = NostrPublicKey::parse(value).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid Organization Brain requester identity: {error}"),
        )
    })?;
    public_key.to_npub().map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid Organization Brain requester identity: {error}"),
        )
    })
}

pub(crate) async fn brain_metadata_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor_npub = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;

    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain(&brain_id)?
    };
    ensure_metadata_visible(&stored, &actor_npub)?;
    let actor_is_admin = ensure_brain_admin(&stored, &actor_npub).is_ok();
    let mounted_folders = {
        let store = state.store.lock().map_err(lock_error)?;
        store.mounted_folder_projection(&brain_id, &UserId::new(actor_npub.clone())?)?
    };

    let mut response = metadata_response_for_actor(stored, mounted_folders, &actor_npub);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_metadata_identities(&store, &mut response)?;
        if actor_is_admin {
            attach_pending_approvals(&store, &mut response, &brain_id)?;
            attach_pending_wraps(&store, &mut response, &brain_id)?;
        }
    }
    Ok(Json(response))
}

pub(crate) async fn encrypted_brain_export_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<EncryptedBrainExportResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor_id = UserId::new(actor.clone())?;
    let brain_id = BrainId::new(brain_id)?;
    let (export, pending_wraps) = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_metadata_visible(&stored, &actor)?;
        let export = store.encrypted_brain_export(&brain_id, &actor_id)?;
        // Pending grant wraps ride the export only for key-holding clients
        // (Brain admin standing); everyone else gets the export exactly as
        // before and older clients ignore the field.
        let pending_wraps = if ensure_brain_admin(&stored, &actor).is_ok() {
            store
                .pending_grant_wraps(&brain_id)?
                .into_iter()
                .map(pending_grant_wrap_response)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        (export, pending_wraps)
    };
    Ok(Json(encrypted_brain_export_response_with_wraps(
        export,
        pending_wraps,
    )))
}

pub(crate) async fn brain_search_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_metadata_visible(&stored, &actor)?;
    }
    Err(ApiError::new(
        StatusCode::BAD_REQUEST,
        "plaintext search is client-side only over decrypted accessible content",
    ))
}

pub(crate) async fn add_member_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: AdminEventRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::AddMember,
        None,
        Some(target.as_str()),
        None,
    )?;
    let control_records = admin_mutation_control_records(&[], &actor, &event, &payload)?;
    let notification_state = state.clone();
    let notification_brain_id = brain_id.clone();
    let response = run_as_admin(state, brain_id, actor, |store, brain_id| {
        store.add_member_with_control_records(brain_id, &target, &control_records)
    })?;
    notification_state.publish_access_update_for(&notification_brain_id, target.as_str());
    Ok(Json(response))
}

/// One-step repair for a half-onboarded member: idempotently ensure the
/// target's Brain Membership server-side, then report every Folder the target
/// is entitled to read and whether a current Folder Key Grant exists for
/// them. Missing grants are left to the caller, who holds the Folder Keys.
pub(crate) async fn ensure_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<EnsureAccessResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: EnsureAccessRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let target_identity = resolve_and_record_identity(&state, &request.target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;

    let membership = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let already_present = stored.brain.owner_user_id.as_ref() == Some(&target)
            || stored.brain.admins.contains(&target)
            || store.member_exists(&brain_id, &target)?;
        if already_present {
            "alreadyMember"
        } else {
            let Some(event_value) = request.access_change_event.clone() else {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "accessChangeEvent is required to add the missing Brain Membership",
                ));
            };
            let (event, payload) = validate_admin_access_change_value(
                event_value,
                &brain_id,
                &actor,
                AdminAccessAction::AddMember,
                None,
                Some(target.as_str()),
                None,
            )?;
            let control_records = admin_mutation_control_records(&[], &actor, &event, &payload)?;
            store.add_member_with_control_records(&brain_id, &target, &control_records)?;
            "added"
        }
    };
    if membership == "added" {
        state.publish_access_update_for(&brain_id, target.as_str());
    }

    // Re-load after the membership mutation so the receipt is authoritative.
    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain(&brain_id)?
    };
    let is_owner = stored.brain.owner_user_id.as_ref() == Some(&target);
    let is_admin = stored.brain.admins.contains(&target);
    let brain_role = if is_owner {
        "owner"
    } else if is_admin {
        "admin"
    } else {
        "member"
    };
    let full_access = is_owner || is_admin;
    let mut folders = Vec::new();
    for folder in &stored.brain.folders {
        let entitled = full_access
            || folder.access == FolderAccessMode::AllMembers
            || stored
                .folder_access
                .get(&folder.id)
                .is_some_and(|users| users.contains(&target));
        if !entitled {
            continue;
        }
        let present = stored.grants.iter().any(|grant| {
            grant.folder_id == folder.id
                && grant.key_version == folder.current_key_version
                && grant.recipient_npub == target
        });
        folders.push(EnsureAccessFolderStatus {
            folder_id: folder.id.to_string(),
            path: folder.path.to_string(),
            key_version: folder.current_key_version,
            grant: if present {
                EnsureAccessGrantState::Present
            } else {
                EnsureAccessGrantState::Missing
            },
        });
    }
    let missing_count = folders
        .iter()
        .filter(|folder| folder.grant == EnsureAccessGrantState::Missing)
        .count();
    Ok(Json(EnsureAccessResponse {
        brain_id: brain_id.to_string(),
        target_npub: target.to_string(),
        membership: membership.to_owned(),
        brain_role: brain_role.to_owned(),
        state: if missing_count == 0 {
            "complete".to_owned()
        } else {
            "grantsMissing".to_owned()
        },
        folders,
        missing_count,
    }))
}

pub(crate) async fn remove_member_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: RemoveMemberRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    validate_folder_rotation_fanout(
        FolderRotationOperation::MemberRemoval,
        request
            .rotations
            .iter()
            .map(|rotation| FolderRotationFanout {
                grants: rotation.grants.len(),
                reencrypted_records: rotation.reencrypted_records.len(),
            })
            .chain(
                request
                    .mount_rotations
                    .iter()
                    .map(|rotation| FolderRotationFanout {
                        grants: rotation.grants.len(),
                        reencrypted_records: rotation.reencrypted_records.len(),
                    }),
            ),
    )?;
    let brain_id = BrainId::new(brain_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::RemoveMember,
        None,
        Some(target.as_str()),
        None,
    )?;
    let updated_at = server_timestamp(&state);
    let mut control_records_by_brain = BTreeMap::<BrainId, Vec<SyncRecordInput>>::new();
    let mut rotations = Vec::with_capacity(request.rotations.len());
    for rotation in request.rotations {
        let folder_id = FolderId::new(rotation.folder_id)?;
        let grants =
            grant_requests_to_metadata(&rotation.grants, &folder_id, &actor, None, &updated_at)?;
        let control_records = control_records_by_brain
            .entry(brain_id.clone())
            .or_default();
        for grant in &grants {
            control_records.push(folder_key_grant_sync_record(grant)?);
        }
        let records = rotation_records_from_requests(
            &brain_id,
            &folder_id,
            &actor,
            rotation.new_key_version,
            rotation.reencrypted_records,
        )?;
        rotations.push(MemberFolderRotation {
            folder_id,
            new_key_version: rotation.new_key_version,
            grants,
            reencrypted_records: records,
        });
    }
    let mut mount_rotations = Vec::with_capacity(request.mount_rotations.len());
    for rotation in request.mount_rotations {
        let connection = {
            let store = state.store.lock().map_err(lock_error)?;
            store.load_shared_folder_connection(&rotation.mount_id)?
        };
        let grants = grant_requests_to_metadata(
            &rotation.grants,
            &connection.source_folder_id,
            &actor,
            None,
            &updated_at,
        )?;
        let control_records = control_records_by_brain
            .entry(connection.source_brain_id.clone())
            .or_default();
        for grant in &grants {
            control_records.push(folder_key_grant_sync_record(grant)?);
        }
        let records = rotation_records_from_requests(
            &connection.source_brain_id,
            &connection.source_folder_id,
            &actor,
            rotation.new_key_version,
            rotation.reencrypted_records,
        )?;
        mount_rotations.push(MemberMountRotation {
            connection_id: rotation.mount_id,
            revoke_mount: rotation.revoke_mount,
            new_key_version: rotation.new_key_version,
            grants,
            reencrypted_records: records,
        });
    }
    control_records_by_brain
        .entry(brain_id.clone())
        .or_default()
        .push(admin_access_change_sync_record(&actor, &event, &payload)?);
    let notification_brain_ids = control_records_by_brain.keys().cloned().collect::<Vec<_>>();
    let actor_user_id = UserId::new(&actor)?;
    let notification_state = state.clone();
    let response = run_as_admin(state, brain_id, actor, |store, brain_id| {
        store.remove_member_with_rotations_and_control_records(
            brain_id,
            &actor_user_id,
            &target,
            &rotations,
            &mount_rotations,
            &updated_at,
            &control_records_by_brain,
        )
    })?;
    for affected_brain_id in notification_brain_ids {
        notification_state.publish_access_update_for(&affected_brain_id, target.as_str());
    }
    Ok(Json(response))
}

pub(crate) async fn add_admin_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: AdminEventRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::AddAdmin,
        None,
        Some(target.as_str()),
        None,
    )?;
    let control_records = admin_mutation_control_records(&[], &actor, &event, &payload)?;
    let notification_state = state.clone();
    let notification_brain_id = brain_id.clone();
    let response = run_as_admin(state, brain_id, actor, |store, brain_id| {
        store.add_admin_with_control_records(brain_id, &target, &control_records)
    })?;
    notification_state.publish_access_update_for(&notification_brain_id, target.as_str());
    Ok(Json(response))
}

pub(crate) async fn remove_admin_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: AdminEventRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::RemoveAdmin,
        None,
        Some(target.as_str()),
        None,
    )?;
    let control_records = admin_mutation_control_records(&[], &actor, &event, &payload)?;
    let notification_state = state.clone();
    let notification_brain_id = brain_id.clone();
    let response = run_as_admin(state, brain_id, actor, |store, brain_id| {
        store.remove_admin_with_control_records(brain_id, &target, &control_records)
    })?;
    notification_state.publish_access_update_for(&notification_brain_id, target.as_str());
    Ok(Json(response))
}

pub(crate) async fn list_brain_invitations_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<BrainInvitationListResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let now = server_timestamp(&state);
    let invitations = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let mut responses = store
            .list_brain_invitations(&brain_id)?
            .into_iter()
            .filter(|invitation| !invitation.folder_only)
            .map(brain_invitation_response)
            .collect::<Vec<_>>();
        for response in &mut responses {
            mark_invitation_expired(response, &now);
            enrich_brain_invitation_identities(&store, response)?;
            attach_invitation_public_url(&state, response);
        }
        responses
    };
    Ok(Json(BrainInvitationListResponse { invitations }))
}

/// List the caller's own pending npub-targeted Brain Invitations, including
/// expired ones marked with `expired: true` so an expired Invite surfaces as
/// expired instead of silently disappearing. Identity-hiding: the exact
/// target sees their invitations, everyone else sees an empty list. Email
/// Invite Bootstraps are not bound to an npub until claim, so they are
/// excluded.
pub(crate) async fn list_my_invitations_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<MyInvitationListResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let mut invitations = Vec::new();
    {
        let store = state.store.lock().map_err(lock_error)?;
        for invitation in store.list_pending_brain_invitations_for_target(&actor)? {
            let expired = timestamp_expired(&invitation.expires_at, &now);
            let brain_display_name = store.load_brain(&invitation.brain_id)?.brain.name;
            let inviter_display =
                known_identity_responses(&store, [invitation.created_by_npub.to_string()])?
                    .into_iter()
                    .next()
                    .map(|identity| identity.display)
                    .unwrap_or_else(|| invitation.created_by_npub.to_string());
            let public_instructions_url = Some(absolute_public_url(
                &state,
                &public_invite_instructions_path(&invitation.invite_code),
            ));
            invitations.push(MyInvitationResponse {
                id: invitation.id,
                invite_code: invitation.invite_code,
                brain_id: invitation.brain_id.to_string(),
                brain_display_name: brain_display_name.to_string(),
                inviter_display,
                folder_scope: invitation
                    .initial_folder_access
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                expires_at: invitation.expires_at,
                expired,
                public_instructions_url,
                origin_kind: invitation.origin_kind.as_str().to_owned(),
                origin_ref: invitation.origin_ref,
            });
        }
    }
    Ok(Json(MyInvitationListResponse { invitations }))
}

pub(crate) async fn create_brain_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<BrainInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: CreateBrainInvitationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    if request.folder_only && !uri.path().split('/').any(|segment| segment == "folders") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Folder Invitations must be created through the Folder invitation collection",
        ));
    }
    let brain_id = BrainId::new(brain_id)?;
    let actor_user_id = UserId::new(actor.clone())?;
    let created_at = server_timestamp(&state);
    let target_input = invitation_target_input(&request)?;

    let npub_target = if let Ok(public_key) = NostrPublicKey::parse(&target_input) {
        Some(public_key.to_npub().map_err(nostr_identity_error)?)
    } else if email_like(&target_input) {
        // Any email-shaped target resolves through public NIP-05 only; there
        // is no account authority to consult (auth kernel).
        match resolve_and_record_identity(&state, &target_input).await {
            Ok(identity) => Some(identity.npub),
            Err(error) if error.status == StatusCode::NOT_FOUND => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };

    let Some(target_npub) = npub_target else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invitation target must be an npub, hex key, or resolvable NIP-05 identifier; to invite by email, create a capability Invite Token with email delivery",
        ));
    };
    let invitation = {
        let target = UserId::new(target_npub)?;
        let initial_folder_access = selected_folder_ids(&request.initial_folder_access)?;
        let id = generated_link_id(
            "invitation",
            &[
                brain_id.as_str(),
                target.as_str(),
                actor_user_id.as_str(),
                request.expires_at.as_str(),
                created_at.as_str(),
            ],
            16,
        );
        let invite_code = generated_link_id(
            "invite",
            &[
                brain_id.as_str(),
                target.as_str(),
                actor_user_id.as_str(),
                request.expires_at.as_str(),
                created_at.as_str(),
                "code",
            ],
            16,
        );
        let accept_path = format!("/v1/brain-invitation-links/{invite_code}/accept");
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        store.create_brain_invitation(
            &brain_id,
            &id,
            &target,
            &invite_code,
            &accept_path,
            &initial_folder_access,
            &actor_user_id,
            &request.expires_at,
            &created_at,
        )?
    };

    let delivery_status = deliver_email_invitation(&state, &invitation)?;
    let mut response = brain_invitation_response(invitation);
    response.delivery_status = delivery_status;
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_brain_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn get_brain_invitation_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invite_code): AxumPath<String>,
) -> Result<Json<BrainInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_available_brain_invitation_by_code(&invite_code, &actor, &now)?
    };
    let mut response = brain_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_brain_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn public_brain_invitation_instructions_handler(
    State(state): State<ServerState>,
    AxumPath(invite_code): AxumPath<String>,
) -> Result<Response, ApiError> {
    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain_invitation_by_code(&invite_code)?
    };
    // Only npub-targeted invitations have public instructions. Legacy
    // email-bootstrap codes (a removed flow) get the identity-hiding
    // unavailable response, same as unknown codes.
    if invitation.target_kind != BrainInvitationTargetKind::Npub {
        return Err(StoreError::UnavailableLink {
            kind: "brain invitation",
        }
        .into());
    }
    // E1: the npub variant reveals the target npub and invitation id, so it
    // resolves only while the invitation is still acceptable; terminal and
    // expired invitations get the identity-hiding unavailable response.
    let now = server_timestamp(&state);
    if invitation.status != LinkStatus::Pending || timestamp_expired(&invitation.expires_at, &now) {
        return Err(StoreError::UnavailableLink {
            kind: "brain invitation",
        }
        .into());
    }
    let text =
        npub_invite_instructions_text(&state, &invitation).ok_or(StoreError::UnavailableLink {
            kind: "brain invitation",
        })?;
    Ok(text_response(text))
}

pub(crate) async fn accept_brain_invitation_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invite_code): AxumPath<String>,
) -> Result<Json<BrainInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.accept_brain_invitation_by_code(&invite_code, &actor, &now)?
    };
    let mut response = brain_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_brain_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}
