use crate::*;

pub(crate) async fn folder_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
) -> Result<Json<FolderMetadataResponse>, ApiError> {
    let actor_npub = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain(&brain_id)?
    };
    ensure_metadata_visible(&stored, &actor_npub)?;
    let response = metadata_response_for_actor(stored, Vec::new(), &actor_npub)
        .folders
        .into_iter()
        .find(|folder| folder.id == folder_id.as_str())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Folder not found"))?;
    Ok(Json(response))
}

pub(crate) async fn grant_account_cohort_folder_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<GrantAccountCohortFolderAccessResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let actor_id = UserId::new(actor.clone())?;
    let request: GrantAccountCohortFolderAccessRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let preview_request = PreviewBrainInvitationRequest {
        target_email: request.target_email.clone(),
        folder_only: true,
        initial_folder_access: vec![folder_id.to_string()],
        expires_at: request.expires_at.clone(),
        approved_exclusions: request.approved_exclusions.clone(),
    };
    let (preview, account_id) =
        build_invitation_preview(&state, &actor, &brain_id, preview_request, false).await?;
    if preview.plan_id != request.plan_id {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Folder access plan is stale; review the returned preflight again",
        ));
    }
    let expected_exclusions = preview
        .excluded
        .iter()
        .map(|excluded| canonical_email(&excluded.nip05))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let approved_exclusions = request
        .approved_exclusions
        .iter()
        .map(|nip05| canonical_email(nip05))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if expected_exclusions != approved_exclusions
        || request.approved_exclusions.len() != approved_exclusions.len()
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "the exact reduced participant set requires explicit approval",
        ));
    }
    if !preview.capacity.fits {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "the preflight participant set exceeds Brain capacity",
        ));
    }
    let expected_grants = preview
        .participants
        .iter()
        .map(|participant| {
            (
                folder_id.to_string(),
                preview.key_versions[0].key_version,
                participant.npub.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    let provided_grants = request
        .participant_grants
        .iter()
        .map(|request| {
            canonical_npub_from_public_key_input(&request.grant.recipient_npub).map(|recipient| {
                (
                    request.folder_id.clone(),
                    request.grant.key_version,
                    recipient,
                )
            })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if provided_grants != expected_grants
        || request.participant_grants.len() != expected_grants.len()
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "participant grants must exactly match the Folder access preflight",
        ));
    }
    let created_at = server_timestamp(&state);
    let grants =
        bootstrap_grant_requests_to_metadata(&request.participant_grants, &actor, &created_at)?;
    let human_npub = preview
        .participants
        .iter()
        .find(|participant| participant.relationship == "human")
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "cohort human is missing"))?
        .npub
        .clone();
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::GrantFolderAccess,
        Some(&folder_id),
        Some(&human_npub),
        Some(preview.key_versions[0].key_version),
    )?;
    let control_records = admin_mutation_control_records(&grants, &actor, &event, &payload)?;
    let participants = preview
        .participants
        .iter()
        .map(|participant| {
            Ok(StoredCohortParticipant {
                relationship: participant.relationship.clone(),
                name: participant.name.clone(),
                nip05: participant.nip05.clone(),
                npub: UserId::new(participant.npub.clone())?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let cohort = BootstrapAccountCohort {
        account_id,
        human_email: preview.target_email.clone(),
        roster_revision: preview.roster_revision,
        participants,
    };
    let operation_id = generated_link_id(
        "folder-cohort-access",
        &[brain_id.as_str(), folder_id.as_str(), &preview.plan_id],
        16,
    );
    let (outcome, mut metadata) = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let outcome = store.grant_account_cohort_folder_access(
            &brain_id,
            &folder_id,
            &operation_id,
            &cohort,
            &actor_id,
            &grants,
            &control_records,
            &created_at,
        )?;
        let stored = store.load_brain(&brain_id)?;
        let mut metadata = metadata_response(stored);
        enrich_metadata_identities(&store, &mut metadata)?;
        (outcome, metadata)
    };
    metadata.identities.retain(|identity| {
        preview
            .participants
            .iter()
            .any(|participant| participant.npub == identity.npub)
            || metadata.members.contains(&identity.npub)
    });
    for participant in &preview.participants {
        state.publish_access_update_for(&brain_id, &participant.npub);
    }
    Ok(Json(GrantAccountCohortFolderAccessResponse {
        brain_id: brain_id.to_string(),
        folder_id: folder_id.to_string(),
        target_email: preview.target_email,
        outcome: match outcome {
            GrantAccountCohortFolderAccessOutcome::Granted => "granted",
            GrantAccountCohortFolderAccessOutcome::AlreadyApplied => "already_applied",
        }
        .to_owned(),
        participants: preview.participants,
        excluded: preview.excluded,
        metadata,
    }))
}

fn account_cohort_folder_removal_preview(
    brain_id: &BrainId,
    folder_id: &FolderId,
    target_email: &str,
    plan: &AccountCohortFolderRemovalPlan,
) -> Result<PreviewAccountCohortFolderRemovalResponse, ApiError> {
    let participants = plan
        .participants
        .iter()
        .map(|participant| InvitationPlanParticipantResponse {
            relationship: participant.relationship.clone(),
            name: participant.name.clone(),
            nip05: participant.nip05.clone(),
            npub: participant.npub.to_string(),
            ready: true,
        })
        .collect::<Vec<_>>();
    let binding = serde_json::json!({
        "brainId": brain_id.as_str(),
        "folderId": folder_id.as_str(),
        "targetEmail": target_email,
        "cohortIds": plan.cohort_ids,
        "sourceOrigins": plan.source_origins,
        "removedParticipantNpubs": plan.removed_participant_npubs,
        "independentlyRetainedNpubs": plan.independently_retained_npubs,
        "requiredRecipientNpubs": plan.required_recipient_npubs,
        "currentKeyVersion": plan.current_key_version,
        "newKeyVersion": plan.new_key_version,
    });
    let bytes = serde_json::to_vec(&binding).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "plan serialization failed",
        )
    })?;
    Ok(PreviewAccountCohortFolderRemovalResponse {
        plan_id: format!("cohort-removal-plan-{:x}", Sha256::digest(bytes)),
        brain_id: brain_id.to_string(),
        folder_id: folder_id.to_string(),
        target_email: target_email.to_owned(),
        participants,
        removed_participant_npubs: plan
            .removed_participant_npubs
            .iter()
            .map(ToString::to_string)
            .collect(),
        independently_retained_npubs: plan
            .independently_retained_npubs
            .iter()
            .map(ToString::to_string)
            .collect(),
        required_recipient_npubs: plan
            .required_recipient_npubs
            .iter()
            .map(ToString::to_string)
            .collect(),
        current_key_version: plan.current_key_version,
        new_key_version: plan.new_key_version,
    })
}

pub(crate) async fn preview_account_cohort_folder_removal_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<PreviewAccountCohortFolderRemovalResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: PreviewAccountCohortFolderRemovalRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let target_email = canonical_email(&request.target_email)?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let plan = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        store.plan_account_cohort_folder_access_removal(&brain_id, &folder_id, &target_email)?
    };
    Ok(Json(account_cohort_folder_removal_preview(
        &brain_id,
        &folder_id,
        &target_email,
        &plan,
    )?))
}

pub(crate) async fn remove_account_cohort_folder_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<RemoveAccountCohortFolderAccessResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let actor_id = UserId::new(actor.clone())?;
    let request: RemoveAccountCohortFolderAccessRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let target_email = canonical_email(&request.target_email)?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let plan = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        store.plan_account_cohort_folder_access_removal(&brain_id, &folder_id, &target_email)?
    };
    let preview =
        account_cohort_folder_removal_preview(&brain_id, &folder_id, &target_email, &plan)?;
    if request.plan_id != preview.plan_id || request.new_key_version != plan.new_key_version {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Folder access removal plan is stale; review a fresh preflight",
        ));
    }
    let updated_at = server_timestamp(&state);
    let grants =
        grant_requests_to_metadata(&request.grants, &folder_id, &actor, None, &updated_at)?;
    let human_npub = plan
        .participants
        .iter()
        .find(|participant| participant.relationship == "human")
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "cohort human is missing"))?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::RemoveFolderAccess,
        Some(&folder_id),
        Some(human_npub.npub.as_str()),
        Some(plan.new_key_version),
    )?;
    let control_records = admin_mutation_control_records(&grants, &actor, &event, &payload)?;
    let reencrypted_records = rotation_records_from_requests(
        &brain_id,
        &folder_id,
        &actor,
        plan.new_key_version,
        request.reencrypted_records,
    )?;
    let metadata = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.remove_account_cohort_folder_access(
            &brain_id,
            &folder_id,
            &plan,
            &actor_id,
            &grants,
            &reencrypted_records,
            &control_records,
            &updated_at,
        )?;
        let stored = store.load_brain(&brain_id)?;
        let mut metadata = metadata_response(stored);
        enrich_metadata_identities(&store, &mut metadata)?;
        metadata
    };
    for npub in plan
        .removed_participant_npubs
        .iter()
        .chain(plan.independently_retained_npubs.iter())
    {
        state.publish_access_update_for(&brain_id, npub.as_str());
    }
    Ok(Json(RemoveAccountCohortFolderAccessResponse {
        brain_id: brain_id.to_string(),
        folder_id: folder_id.to_string(),
        target_email,
        removed_participant_npubs: preview.removed_participant_npubs,
        independently_retained_npubs: preview.independently_retained_npubs,
        new_key_version: plan.new_key_version,
        metadata,
    }))
}
use finite_brain_core::BRAIN_CAPACITY_ENVELOPE;
use finite_brain_store::FolderDeletionExpectation;

pub(crate) async fn delete_folder_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<FolderDeleteResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: FolderDeleteRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let expectation = {
        let folder_ids = &request.expected_folder_ids;
        let object_count = request.expected_object_count;
        if folder_ids.is_empty() || folder_ids.len() > BRAIN_CAPACITY_ENVELOPE.folders {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "expectedFolderIds is outside the accepted Folder envelope",
            ));
        }
        let parsed = folder_ids
            .iter()
            .map(|folder_id| FolderId::new(folder_id.clone()))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if parsed.len() != folder_ids.len() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "expectedFolderIds contains duplicate Folder identities",
            ));
        }
        if object_count > BRAIN_CAPACITY_ENVELOPE.current_objects {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "expectedObjectCount is outside the accepted object envelope",
            ));
        }
        FolderDeletionExpectation {
            folder_ids: parsed,
            object_count,
        }
    };
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let submitted_event = Event::from_json(request.deletion_event.to_string()).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "deletionEvent must be a valid signed Nostr event",
        )
    })?;
    let submitted_event_id = submitted_event.id.to_hex();
    let current_key_version = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_direct_delete_authority(&stored, &actor)?;
        if let Some(folder) = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
        {
            folder.current_key_version
        } else if let Some(replay) = store.folder_deletion_replay(&brain_id, &folder_id)? {
            if replay.deletion_event_id != submitted_event_id || replay.actor_npub.as_str() != actor
            {
                return Err(ApiError::from(StoreError::BrokenInvariant {
                    reason: "Folder identity was already permanently deleted".to_owned(),
                }));
            }
            replay.root_key_version
        } else {
            return Err(ApiError::from(StoreError::MissingFolder {
                folder_id: folder_id.to_string(),
            }));
        }
    };
    let (event, payload) = validate_admin_access_change_value(
        request.deletion_event,
        &brain_id,
        &actor,
        AdminAccessAction::DeleteFolder,
        Some(&folder_id),
        None,
        Some(current_key_version),
    )?;
    let event_id = event.id.to_hex();
    let deleted_at = payload.created_at.clone();
    let payload_json = serde_json::json!({
        "recordType": "folder_subtree_tombstone",
        "folderId": folder_id,
        "deletionEvent": event,
    })
    .to_string();
    let actor = UserId::new(actor)?;
    let outcome = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.delete_folder_subtree(
            &brain_id,
            &folder_id,
            &actor,
            current_key_version,
            &event_id,
            &payload_json,
            &deleted_at,
            APP_SPECIFIC_KIND,
            Some(&expectation),
        )?
    };
    if !outcome.duplicate {
        state.publish_brain_update(
            brain_id.as_str(),
            outcome.sequence,
            BrainUpdateReason::AccessUpdated,
        );
    }
    Ok(Json(FolderDeleteResponse {
        sequence: outcome.sequence,
        duplicate: outcome.duplicate,
        folder_count: outcome.folder_count,
        object_count: outcome.object_count,
        deleted_folder_ids: outcome
            .deleted_folder_ids
            .into_iter()
            .map(|folder_id| folder_id.to_string())
            .collect(),
    }))
}

pub(crate) async fn create_folder_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: CreateFolderRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let folder = Folder {
        id: FolderId::new(request.folder_id)?,
        name: DisplayName::new("folder_name", request.name)?,
        role: request.role,
        access: request.access,
        parent_folder_id: request.parent_folder_id.map(FolderId::new).transpose()?,
        path: SafeRelativePath::new("folder_path", request.path)?,
        current_key_version: 1,
    };
    let access_user_ids = resolve_user_id_set(&state, request.access_user_ids).await?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::SetFolderAccessMode,
        Some(&folder.id),
        None,
        Some(1),
    )?;
    let event_json = event.as_json();
    let grant_created_at = server_timestamp(&state);
    let grants = grant_requests_to_metadata(
        &request.grants,
        &folder.id,
        &actor,
        Some(event_json),
        &grant_created_at,
    )?;
    let control_records = admin_mutation_control_records(&grants, &actor, &event, &payload)?;

    let notification_state = state.clone();
    let notification_brain_id = brain_id.clone();
    let response = run_as_admin(state, brain_id, actor, |store, brain_id| {
        store.create_folder_with_control_records(
            brain_id,
            &folder,
            &access_user_ids,
            &grants,
            &control_records,
        )
    })?;
    // A newly created all-members Folder changes every member's authoritative
    // view. Broadcast the hint; stream-time authorization still filters it to
    // actors who can currently see this Brain.
    notification_state.publish_access_update(&notification_brain_id);
    Ok(Json(response))
}

pub(crate) async fn finish_folder_setup_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: FinishFolderSetupRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let current_key_version = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        folder_current_key_version(&stored, &folder_id)?
    };
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::SetFolderAccessMode,
        Some(&folder_id),
        None,
        Some(current_key_version),
    )?;
    let event_json = event.as_json();
    let grant_created_at = server_timestamp(&state);
    let grants = grant_requests_to_metadata(
        &request.grants,
        &folder_id,
        &actor,
        Some(event_json),
        &grant_created_at,
    )?;
    let control_records = admin_mutation_control_records(&grants, &actor, &event, &payload)?;

    let notification_state = state.clone();
    let notification_brain_id = brain_id.clone();
    let response = run_as_admin(state, brain_id, actor, |store, brain_id| {
        store.finish_folder_setup_with_control_records(
            brain_id,
            &folder_id,
            &grants,
            &control_records,
        )
    })?;
    notification_state.publish_access_update(&notification_brain_id);
    Ok(Json(response))
}

pub(crate) async fn grant_folder_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id, target_npub)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> Result<Json<GrantFolderAccessResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    reject_legacy_finite_vip_principal_write(&target_npub)?;
    let request: GrantFolderAccessRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;
    let actor_id = UserId::new(actor.clone())?;
    let (current_key_version, peer_restore, authenticated_human_intent) = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let peer_change = personal_peer_agent_change(&stored, &actor_id, &target);
        let peer_restore = peer_change
            && stored
                .account_agent_exclusions
                .contains(&(target.clone(), folder_id.to_string()));
        let authenticated_human_intent = match (peer_restore, request.authenticated_human_intent) {
            (true, Some(value)) => Some(validate_authenticated_human_intent_value(
                &state,
                value,
                &stored,
                &brain_id,
                &actor_id,
                &target,
                "restore",
                Some(&folder_id),
                state.auth_now_unix_seconds(),
            )?),
            (true, None) => {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "restoring a peer Personal Brain Agent requires Authenticated Human Intent",
                ));
            }
            (false, Some(_)) => {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Authenticated Human Intent was supplied for an operation that does not consume it",
                ));
            }
            (false, None) => None,
        };
        (
            folder_current_key_version(&stored, &folder_id)?,
            peer_restore,
            authenticated_human_intent,
        )
    };
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::GrantFolderAccess,
        Some(&folder_id),
        Some(target.as_str()),
        Some(current_key_version),
    )?;
    let grant_created_at = server_timestamp(&state);
    let mut grant_request = request.grant;
    grant_request.recipient_npub = target.as_str().to_owned();
    let grant = grant_request_to_metadata(
        &grant_request,
        &folder_id,
        &actor,
        Some(event.as_json()),
        &grant_created_at,
    )?;
    let control_records = [
        folder_key_grant_sync_record(&grant)?,
        admin_access_change_sync_record(&actor, &event, &payload)?,
    ];

    let (metadata, outcome) = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let outcome = if peer_restore {
            store.restore_personal_agent_folder_access_with_control_records(
                &brain_id,
                &folder_id,
                &target,
                &grant,
                &control_records,
                authenticated_human_intent.as_ref().ok_or_else(|| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "missing validated Authenticated Human Intent",
                    )
                })?,
                &grant_created_at,
            )?;
            GrantFolderAccessOutcome::Granted
        } else {
            store.grant_folder_access_with_control_records(
                &brain_id,
                &folder_id,
                &target,
                &grant,
                &control_records,
            )?
        };
        let stored = store.load_brain(&brain_id)?;
        let mut metadata = metadata_response(stored);
        enrich_metadata_identities(&store, &mut metadata)?;
        (metadata, outcome)
    };
    let outcome = match outcome {
        GrantFolderAccessOutcome::Granted => GrantFolderAccessResponseOutcome::Granted,
        GrantFolderAccessOutcome::AlreadyHasAccess => {
            GrantFolderAccessResponseOutcome::AlreadyHasAccess
        }
    };
    state.publish_access_update_for(&brain_id, target.as_str());
    Ok(Json(GrantFolderAccessResponse { metadata, outcome }))
}

pub(crate) async fn remove_folder_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id, target_npub)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    reject_legacy_finite_vip_principal_write(&target_npub)?;
    let request: RemoveFolderAccessRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    validate_folder_rotation_fanout(
        FolderRotationOperation::FolderAccessRemoval,
        [FolderRotationFanout {
            grants: request.grants.len(),
            reencrypted_records: request.reencrypted_records.len(),
        }],
    )?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;
    let actor_id = UserId::new(actor.clone())?;
    let authenticated_human_intent = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        if personal_peer_agent_change(&stored, &actor_id, &target) {
            let value = request.authenticated_human_intent.ok_or_else(|| {
                ApiError::new(
                    StatusCode::FORBIDDEN,
                    "restricting a peer Personal Brain Agent requires Authenticated Human Intent",
                )
            })?;
            Some(validate_authenticated_human_intent_value(
                &state,
                value,
                &stored,
                &brain_id,
                &actor_id,
                &target,
                "restrict",
                Some(&folder_id),
                state.auth_now_unix_seconds(),
            )?)
        } else if request.authenticated_human_intent.is_some() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Authenticated Human Intent was supplied for an operation that does not consume it",
            ));
        } else {
            None
        }
    };
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::RemoveFolderAccess,
        Some(&folder_id),
        Some(target.as_str()),
        Some(request.new_key_version),
    )?;
    let event_json = event.as_json();
    let grant_created_at = server_timestamp(&state);
    let updated_at = grant_created_at.clone();
    let grants = grant_requests_to_metadata(
        &request.grants,
        &folder_id,
        &actor,
        Some(event_json),
        &grant_created_at,
    )?;
    let mut reencrypted_records = Vec::new();
    for record in request.reencrypted_records {
        if record.key_version != request.new_key_version {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "rotation record keyVersion must match newKeyVersion",
            ));
        }
        let object_id = ObjectId::new(record.object_id)?;
        let write_request = ObjectWriteRequest {
            base_revision: record.base_revision,
            key_version: record.key_version,
            cipher: record.cipher,
            ciphertext: record.ciphertext,
            revision_event: record.revision_event,
        };
        let (record, _) = validate_object_revision_record(
            &brain_id,
            &folder_id,
            &object_id,
            &actor,
            write_request,
            FolderObjectOperation::Update,
        )?;
        reencrypted_records.push(record);
    }
    let control_records = admin_mutation_control_records(&grants, &actor, &event, &payload)?;

    let notification_state = state.clone();
    let notification_brain_id = brain_id.clone();
    let response = run_as_admin(state, brain_id, actor, |store, brain_id| {
        store.rotate_folder_key_for_access_removal_with_control_records_and_intent(
            brain_id,
            &folder_id,
            &target,
            request.new_key_version,
            &grants,
            &reencrypted_records,
            &updated_at,
            &control_records,
            authenticated_human_intent.as_ref(),
        )
    })?;
    notification_state.publish_access_update_for(&notification_brain_id, target.as_str());
    Ok(Json(response))
}
