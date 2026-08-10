use crate::*;

async fn permanent_agent_departure_plan(
    state: &ServerState,
    brain_id: &BrainId,
    fact_id: &str,
    human_email: &str,
    actor: &UserId,
) -> Result<
    (
        CorePermanentAgentDepartureFactResponse,
        PermanentAgentDeparturePlan,
        PreviewPermanentAgentDepartureResponse,
    ),
    ApiError,
> {
    let human_email = canonical_email(human_email)?;
    let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Brain account-agent authority is not configured",
        )
    })?;
    let departures: CorePermanentAgentDepartureFactsResponse = post_authority_json(
        &format!(
            "{}/api/core/v1/brain/permanent-agent-departures",
            authorities.core_base_url
        ),
        "Authorization",
        &format!("Bearer {}", authorities.core_token),
        &serde_json::json!({ "verifiedEmail": human_email }),
        "Finite Core permanent Agent departures",
    )
    .await?;
    if departures.account_id.trim().is_empty()
        || canonical_email(&departures.human_mailbox)? != human_email
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Finite Core returned mismatched departure facts",
        ));
    }
    let fact = departures
        .facts
        .into_iter()
        .find(|fact| fact.fact_id == fact_id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "departure fact not found"))?;
    if fact.account_id != departures.account_id
        || canonical_email(&fact.human_mailbox)? != human_email
        || !matches!(
            fact.departure_kind.as_str(),
            "unlinked" | "retired" | "deleted"
        )
        || fact.principal_binding_reference.trim().is_empty()
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Finite Core returned an invalid departure fact",
        ));
    }
    OffsetDateTime::parse(&fact.occurred_at, &Rfc3339).map_err(|_| {
        ApiError::new(
            StatusCode::CONFLICT,
            "Finite Core departure timestamp is invalid",
        )
    })?;
    let identity = resolve_and_record_identity(state, &fact.managed_agent_nip05).await?;
    let agent_npub = UserId::new(identity.npub)?;
    let plan = {
        let store = state.store.lock().map_err(lock_error)?;
        store.plan_permanent_agent_departure(
            brain_id,
            &fact.account_id,
            &human_email,
            &fact.managed_agent_nip05,
            &agent_npub,
            actor,
        )?
    };
    let folders = plan
        .folders
        .iter()
        .map(|folder| PermanentAgentDepartureFolderResponse {
            folder_id: folder.folder_id.to_string(),
            current_key_version: folder.current_key_version,
            new_key_version: folder.new_key_version,
            required_recipient_npubs: folder
                .required_recipient_npubs
                .iter()
                .map(ToString::to_string)
                .collect(),
        })
        .collect::<Vec<_>>();
    let binding = serde_json::json!({
        "brainId": brain_id.as_str(),
        "fact": &fact,
        "agentNpub": agent_npub.as_str(),
        "folders": folders,
        "actor": actor.as_str(),
    });
    let bytes = serde_json::to_vec(&binding).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "departure plan serialization failed",
        )
    })?;
    let response = PreviewPermanentAgentDepartureResponse {
        plan_id: format!("departure-plan-{:x}", Sha256::digest(bytes)),
        fact_id: fact.fact_id.clone(),
        account_id: fact.account_id.clone(),
        human_email,
        agent_nip05: fact.managed_agent_nip05.clone(),
        agent_npub: agent_npub.to_string(),
        departure_kind: fact.departure_kind.clone(),
        occurred_at: fact.occurred_at.clone(),
        folders,
    };
    Ok((fact, plan, response))
}

pub(crate) async fn preview_permanent_agent_departure_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, fact_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<PreviewPermanentAgentDepartureResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: PreviewPermanentAgentDepartureRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let (_, _, response) =
        permanent_agent_departure_plan(&state, &brain_id, &fact_id, &request.human_email, &actor)
            .await?;
    Ok(Json(response))
}

pub(crate) async fn apply_permanent_agent_departure_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, fact_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<ApplyPermanentAgentDepartureResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: ApplyPermanentAgentDepartureRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    if let Some((agent_npub, rotated_folders)) = {
        let store = state.store.lock().map_err(lock_error)?;
        store.applied_permanent_agent_departure(&brain_id, &fact_id)?
    } {
        let metadata = {
            let store = state.store.lock().map_err(lock_error)?;
            let stored = store.load_brain(&brain_id)?;
            ensure_brain_admin(&stored, actor.as_str())?;
            let mut metadata = metadata_response(stored);
            enrich_metadata_identities(&store, &mut metadata)?;
            metadata
        };
        return Ok(Json(ApplyPermanentAgentDepartureResponse {
            fact_id,
            agent_npub: agent_npub.to_string(),
            outcome: "already_applied".to_owned(),
            rotated_folder_ids: rotated_folders
                .into_iter()
                .map(|folder| folder.to_string())
                .collect(),
            metadata,
        }));
    }
    let (fact, plan, preview) =
        permanent_agent_departure_plan(&state, &brain_id, &fact_id, &request.human_email, &actor)
            .await?;
    if request.plan_id != preview.plan_id {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "permanent Agent departure plan is stale; retry preflight",
        ));
    }
    let expected = plan
        .folders
        .iter()
        .map(|folder| {
            (
                folder.folder_id.to_string(),
                (
                    folder.new_key_version,
                    folder.required_recipient_npubs.clone(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if request.rotations.len() != expected.len() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "departure rotations must exactly match the preflight",
        ));
    }
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        actor.as_str(),
        AdminAccessAction::RemoveMember,
        None,
        Some(plan.agent_npub.as_str()),
        None,
    )?;
    let now = server_timestamp(&state);
    let mut rotations = Vec::with_capacity(request.rotations.len());
    let mut control_records = Vec::new();
    for rotation in request.rotations {
        let folder_id = FolderId::new(rotation.folder_id)?;
        let Some((expected_version, expected_recipients)) = expected.get(folder_id.as_str()) else {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "departure rotation contains an unexpected Folder",
            ));
        };
        let grants = grant_requests_to_metadata(
            &rotation.grants,
            &folder_id,
            actor.as_str(),
            Some(event.as_json()),
            &now,
        )?;
        let provided = grants
            .iter()
            .map(|grant| grant.recipient_npub.clone())
            .collect::<BTreeSet<_>>();
        if rotation.new_key_version != *expected_version
            || provided != *expected_recipients
            || grants.len() != expected_recipients.len()
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "departure rotation grants must exactly match the preflight",
            ));
        }
        let records = rotation_records_from_requests(
            &brain_id,
            &folder_id,
            actor.as_str(),
            rotation.new_key_version,
            rotation.reencrypted_records,
        )?;
        for grant in &grants {
            control_records.push(folder_key_grant_sync_record(grant)?);
        }
        rotations.push(MemberFolderRotation {
            folder_id,
            new_key_version: rotation.new_key_version,
            grants,
            reencrypted_records: records,
        });
    }
    control_records.push(admin_access_change_sync_record(
        actor.as_str(),
        &event,
        &payload,
    )?);
    let outcome = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.apply_permanent_agent_departure(
            &brain_id,
            &fact.fact_id,
            &fact.account_id,
            &fact.human_mailbox,
            &fact.managed_agent_nip05,
            &plan.agent_npub,
            &fact.departure_kind,
            &fact.occurred_at,
            &actor,
            &rotations,
            &control_records,
            &now,
        )?
    };
    let metadata = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        let mut metadata = metadata_response(stored);
        enrich_metadata_identities(&store, &mut metadata)?;
        metadata
    };
    state.publish_access_update_for(&brain_id, plan.agent_npub.as_str());
    Ok(Json(ApplyPermanentAgentDepartureResponse {
        fact_id,
        agent_npub: plan.agent_npub.to_string(),
        outcome: match outcome {
            ApplyPermanentAgentDepartureOutcome::Applied => "applied",
            ApplyPermanentAgentDepartureOutcome::AlreadyApplied => "already_applied",
        }
        .to_owned(),
        rotated_folder_ids: rotations
            .iter()
            .map(|rotation| rotation.folder_id.to_string())
            .collect(),
        metadata,
    }))
}

async fn personal_agent_brain_access_plan(
    state: &ServerState,
    brain_id: &BrainId,
    target_agent_npub: &UserId,
    operation: &str,
    actor: &UserId,
) -> Result<
    (
        PersonalAgentBrainAccessPlan,
        PreviewPersonalAgentBrainAccessResponse,
    ),
    ApiError,
> {
    if !matches!(operation, "restrict" | "restore") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "operation must be restrict or restore",
        ));
    }
    let plan = {
        let store = state.store.lock().map_err(lock_error)?;
        store.plan_personal_agent_brain_access(brain_id, target_agent_npub, operation, actor)?
    };
    let fresh_roster_revision = if operation == "restore" {
        let cohort =
            resolve_bootstrap_account_cohort(state, &plan.human_email, &plan.human_npub).await?;
        if !cohort.participants.iter().any(|participant| {
            participant.relationship == "account_agent" && participant.npub == *target_agent_npub
        }) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "the target Agent is not eligible in fresh Core and Identity facts",
            ));
        }
        cohort.roster_revision
    } else {
        0
    };
    let folders = plan
        .folders
        .iter()
        .map(|folder| PermanentAgentDepartureFolderResponse {
            folder_id: folder.folder_id.to_string(),
            current_key_version: folder.current_key_version,
            new_key_version: folder.new_key_version,
            required_recipient_npubs: folder
                .required_recipient_npubs
                .iter()
                .map(ToString::to_string)
                .collect(),
        })
        .collect::<Vec<_>>();
    let binding = serde_json::json!({
        "brainId": brain_id.as_str(),
        "humanNpub": plan.human_npub.as_str(),
        "humanEmail": plan.human_email,
        "targetAgentNpub": target_agent_npub.as_str(),
        "operation": operation,
        "folders": folders,
        "actor": actor.as_str(),
        "freshRosterRevision": fresh_roster_revision,
    });
    let plan_id = format!(
        "peer-agent-access-plan-{:x}",
        Sha256::digest(serde_json::to_vec(&binding).map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "peer Agent access plan serialization failed",
            )
        })?)
    );
    Ok((
        plan,
        PreviewPersonalAgentBrainAccessResponse {
            plan_id,
            brain_id: brain_id.to_string(),
            human_email: binding["humanEmail"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            target_agent_npub: target_agent_npub.to_string(),
            operation: operation.to_owned(),
            folders,
        },
    ))
}

pub(crate) async fn preview_personal_agent_brain_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<PreviewPersonalAgentBrainAccessResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: PreviewPersonalAgentBrainAccessRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let target = UserId::new(canonical_npub_from_public_key_input(&target_npub)?)?;
    let (_, response) =
        personal_agent_brain_access_plan(&state, &brain_id, &target, &request.operation, &actor)
            .await?;
    Ok(Json(response))
}

pub(crate) async fn restrict_personal_agent_brain_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<PersonalAgentBrainAccessResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: RestrictPersonalAgentBrainAccessRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let target = UserId::new(canonical_npub_from_public_key_input(&target_npub)?)?;
    let intent = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, actor.as_str())?;
        if !personal_peer_agent_change(&stored, &actor, &target) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "whole-Brain peer access requires two ready Personal Brain Agents",
            ));
        }
        validate_authenticated_human_intent_value(
            request.authenticated_human_intent,
            &stored,
            &brain_id,
            &actor,
            &target,
            "restrict",
            None,
            state.auth_now_unix_seconds(),
        )?
    };
    let (plan, preview) =
        personal_agent_brain_access_plan(&state, &brain_id, &target, "restrict", &actor).await?;
    if request.plan_id != preview.plan_id || request.rotations.len() != plan.folders.len() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "peer Agent Brain restriction plan is stale; retry preflight",
        ));
    }
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        actor.as_str(),
        AdminAccessAction::RestrictPersonalAgent,
        None,
        Some(target.as_str()),
        None,
    )?;
    let expected = plan
        .folders
        .iter()
        .map(|folder| (folder.folder_id.clone(), folder))
        .collect::<BTreeMap<_, _>>();
    let now = server_timestamp(&state);
    let mut rotations = Vec::with_capacity(request.rotations.len());
    let mut controls = Vec::new();
    for rotation in request.rotations {
        let folder_id = FolderId::new(rotation.folder_id)?;
        let expected_folder = expected.get(&folder_id).ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "restriction rotation contains an unexpected Folder",
            )
        })?;
        let grants = grant_requests_to_metadata(
            &rotation.grants,
            &folder_id,
            actor.as_str(),
            Some(event.as_json()),
            &now,
        )?;
        let recipients = grants
            .iter()
            .map(|grant| grant.recipient_npub.clone())
            .collect::<BTreeSet<_>>();
        if rotation.new_key_version != expected_folder.new_key_version
            || recipients != expected_folder.required_recipient_npubs
            || grants.len() != recipients.len()
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "restriction rotation grants do not match the preflight",
            ));
        }
        let records = rotation_records_from_requests(
            &brain_id,
            &folder_id,
            actor.as_str(),
            rotation.new_key_version,
            rotation.reencrypted_records,
        )?;
        controls.extend(
            grants
                .iter()
                .map(folder_key_grant_sync_record)
                .collect::<Result<Vec<_>, _>>()?,
        );
        rotations.push(MemberFolderRotation {
            folder_id,
            new_key_version: rotation.new_key_version,
            grants,
            reencrypted_records: records,
        });
    }
    controls.push(admin_access_change_sync_record(
        actor.as_str(),
        &event,
        &payload,
    )?);
    {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.restrict_personal_agent_brain_access(
            &brain_id, &target, &actor, &rotations, &controls, &intent, &now,
        )?;
    }
    let metadata = metadata_for_admin(&state, &brain_id, actor.as_str())?;
    state.publish_access_update_for(&brain_id, target.as_str());
    Ok(Json(PersonalAgentBrainAccessResponse {
        outcome: "restricted".to_owned(),
        target_agent_npub: target.to_string(),
        operation: "restrict".to_owned(),
        affected_folder_ids: rotations
            .iter()
            .map(|rotation| rotation.folder_id.to_string())
            .collect(),
        metadata,
    }))
}

pub(crate) async fn restore_personal_agent_brain_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<PersonalAgentBrainAccessResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: RestorePersonalAgentBrainAccessRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let target = UserId::new(canonical_npub_from_public_key_input(&target_npub)?)?;
    let intent = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, actor.as_str())?;
        validate_authenticated_human_intent_value(
            request.authenticated_human_intent,
            &stored,
            &brain_id,
            &actor,
            &target,
            "restore",
            None,
            state.auth_now_unix_seconds(),
        )?
    };
    let (plan, preview) =
        personal_agent_brain_access_plan(&state, &brain_id, &target, "restore", &actor).await?;
    if request.plan_id != preview.plan_id {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "peer Agent Brain restoration plan is stale; retry preflight",
        ));
    }
    let expected = plan
        .folders
        .iter()
        .map(|folder| {
            (
                folder.folder_id.to_string(),
                (folder.current_key_version, target.to_string()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let provided = request
        .participant_grants
        .iter()
        .map(|request| {
            Ok((
                request.folder_id.clone(),
                (
                    request.grant.key_version,
                    canonical_npub_from_public_key_input(&request.grant.recipient_npub)?,
                ),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, ApiError>>()?;
    if provided != expected || request.participant_grants.len() != expected.len() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "restoration grants must exactly match the fresh preflight",
        ));
    }
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        actor.as_str(),
        AdminAccessAction::RestorePersonalAgent,
        None,
        Some(target.as_str()),
        None,
    )?;
    let now = server_timestamp(&state);
    let grants =
        bootstrap_grant_requests_to_metadata(&request.participant_grants, actor.as_str(), &now)?;
    let mut controls = grants
        .iter()
        .map(folder_key_grant_sync_record)
        .collect::<Result<Vec<_>, _>>()?;
    controls.push(admin_access_change_sync_record(
        actor.as_str(),
        &event,
        &payload,
    )?);
    {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.restore_personal_agent_brain_access(
            &brain_id, &target, &actor, &grants, &controls, &intent, &now,
        )?;
    }
    let metadata = metadata_for_admin(&state, &brain_id, actor.as_str())?;
    state.publish_access_update_for(&brain_id, target.as_str());
    Ok(Json(PersonalAgentBrainAccessResponse {
        outcome: "restored".to_owned(),
        target_agent_npub: target.to_string(),
        operation: "restore".to_owned(),
        affected_folder_ids: plan
            .folders
            .iter()
            .map(|folder| folder.folder_id.to_string())
            .collect(),
        metadata,
    }))
}

fn metadata_for_admin(
    state: &ServerState,
    brain_id: &BrainId,
    actor: &str,
) -> Result<BrainMetadataResponse, ApiError> {
    let store = state.store.lock().map_err(lock_error)?;
    let stored = store.load_brain(brain_id)?;
    ensure_brain_admin(&stored, actor)?;
    let mut metadata = metadata_response(stored);
    enrich_metadata_identities(&store, &mut metadata)?;
    Ok(metadata)
}

async fn account_cohort_reconciliation_plan(
    state: &ServerState,
    brain_id: &BrainId,
    human_email: &str,
    folder_id: Option<&FolderId>,
    actor: &UserId,
) -> Result<(BootstrapAccountCohort, AccountCohortReconciliationPlan), ApiError> {
    let human_email = canonical_email(human_email)?;
    let resolved_human = resolve_identity_input(state, &human_email).await?;
    let human_npub = UserId::new(resolved_human.npub)?;
    let cohort = resolve_bootstrap_account_cohort(state, &human_email, &human_npub).await?;
    let mut plan = {
        let store = state.store.lock().map_err(lock_error)?;
        store.plan_account_cohort_reconciliation(brain_id, &cohort, folder_id, actor)?
    };
    let bytes = serde_json::to_vec(&plan).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "reconciliation plan serialization failed",
        )
    })?;
    plan.operation_id = format!("reconcile-{:x}", Sha256::digest(bytes));
    Ok((cohort, plan))
}

pub(crate) async fn preview_account_cohort_reconciliation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<AccountCohortReconciliationPlan>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: PreviewAccountCohortReconciliationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = request.folder_id.map(FolderId::new).transpose()?;
    let (_, plan) = account_cohort_reconciliation_plan(
        &state,
        &brain_id,
        &request.human_email,
        folder_id.as_ref(),
        &actor,
    )
    .await?;
    Ok(Json(plan))
}

pub(crate) async fn commit_account_cohort_reconciliation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<CommitAccountCohortReconciliationResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: CommitAccountCohortReconciliationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = request.folder_id.clone().map(FolderId::new).transpose()?;
    if let Some(plan) = {
        let store = state.store.lock().map_err(lock_error)?;
        store.committed_account_cohort_reconciliation(&request.plan_id)?
    } {
        if plan.brain_id != brain_id
            || canonical_email(&plan.human_email)? != canonical_email(&request.human_email)?
            || plan.folder_id != folder_id
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "reconciliation retry does not match the committed receipt",
            ));
        }
        let metadata = metadata_for_admin(&state, &brain_id, actor.as_str())?;
        return Ok(Json(CommitAccountCohortReconciliationResponse {
            outcome: "already_committed".to_owned(),
            plan,
            rollback_boundary: "restore the declared pre-reconciliation Brain database backup"
                .to_owned(),
            metadata,
        }));
    }
    let (cohort, plan) = account_cohort_reconciliation_plan(
        &state,
        &brain_id,
        &request.human_email,
        folder_id.as_ref(),
        &actor,
    )
    .await?;
    if plan.operation_id != request.plan_id {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "reconciliation plan is stale; run dry-run again",
        ));
    }
    if let Some(blocker) = plan.blocker.as_ref() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("reconciliation is blocked: {blocker}"),
        ));
    }
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        actor.as_str(),
        AdminAccessAction::ReconcileAccountCohort,
        folder_id.as_ref(),
        Some(plan.human_npub.as_str()),
        None,
    )?;
    let now = server_timestamp(&state);
    let grants = request
        .participant_grants
        .iter()
        .map(|request| {
            let folder_id = FolderId::new(request.folder_id.clone())?;
            grant_request_to_metadata(
                &request.grant,
                &folder_id,
                actor.as_str(),
                Some(event.as_json()),
                &now,
            )
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let mut control_records = grants
        .iter()
        .map(folder_key_grant_sync_record)
        .collect::<Result<Vec<_>, _>>()?;
    control_records.push(admin_access_change_sync_record(
        actor.as_str(),
        &event,
        &payload,
    )?);
    let outcome = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.commit_account_cohort_reconciliation(
            &plan,
            &cohort,
            &actor,
            &grants,
            &control_records,
            &request.backup_reference,
            &now,
        )?
    };
    let metadata = metadata_for_admin(&state, &brain_id, actor.as_str())?;
    for participant in &plan.participants {
        state.publish_access_update_for(&brain_id, participant.npub.as_str());
    }
    Ok(Json(CommitAccountCohortReconciliationResponse {
        outcome: match outcome {
            CommitAccountCohortReconciliationOutcome::Committed => "committed",
            CommitAccountCohortReconciliationOutcome::AlreadyCommitted => "already_committed",
        }
        .to_owned(),
        plan,
        rollback_boundary: "restore the declared pre-reconciliation Brain database backup"
            .to_owned(),
        metadata,
    }))
}

fn personal_brain_agent_admission_response(
    brain_id: &BrainId,
    plan: &PersonalBrainAgentAdmissionPlan,
    status: &str,
) -> Result<PersonalBrainAgentAdmissionResponse, ApiError> {
    let agents = plan
        .agents
        .iter()
        .map(|agent| InvitationPlanParticipantResponse {
            relationship: agent.relationship.clone(),
            name: agent.name.clone(),
            nip05: agent.nip05.clone(),
            npub: agent.npub.to_string(),
            ready: status == "ready",
        })
        .collect::<Vec<_>>();
    let key_versions = plan
        .folder_key_versions
        .iter()
        .map(
            |(folder_id, key_version)| InvitationPlanKeyVersionResponse {
                folder_id: folder_id.to_string(),
                key_version: *key_version,
            },
        )
        .collect::<Vec<_>>();
    let binding = serde_json::json!({
        "brainId": brain_id.as_str(),
        "humanEmail": plan.human_email,
        "rosterRevision": plan.roster_revision,
        "agents": agents,
        "keyVersions": key_versions,
    });
    let bytes = serde_json::to_vec(&binding).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "plan serialization failed",
        )
    })?;
    Ok(PersonalBrainAgentAdmissionResponse {
        plan_id: format!("personal-agent-plan-{:x}", Sha256::digest(bytes)),
        brain_id: brain_id.to_string(),
        human_email: plan.human_email.clone(),
        roster_revision: plan.roster_revision,
        status: status.to_owned(),
        agents,
        key_versions,
    })
}

async fn prepare_personal_brain_agent_admission_plan(
    state: &ServerState,
    brain_id: &BrainId,
    actor: &UserId,
) -> Result<(PersonalBrainAgentAdmissionPlan, BootstrapAccountCohort), ApiError> {
    let (owner, human_email) = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(brain_id)?;
        ensure_brain_admin(&stored, actor.as_str())?;
        if stored.brain.kind != BrainKind::Personal {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Personal Brain agent admission requires a Personal Brain",
            ));
        }
        let owner = stored.brain.owner_user_id.ok_or_else(|| {
            ApiError::new(StatusCode::CONFLICT, "Personal Brain owner is missing")
        })?;
        let human_email = store.personal_brain_human_email(brain_id)?.ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "Personal Brain account cohort must be reconciled before adding agents",
            )
        })?;
        (owner, human_email)
    };
    let cohort = resolve_bootstrap_account_cohort(state, &human_email, &owner).await?;
    let now = server_timestamp(state);
    let plan = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.stage_personal_brain_agent_admissions(brain_id, &cohort, &now)?
    };
    Ok((plan, cohort))
}

pub(crate) async fn prepare_personal_brain_agent_admissions_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<PersonalBrainAgentAdmissionResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state, &headers, &method, &uri, None,
    )?)?;
    let brain_id = BrainId::new(brain_id)?;
    let (plan, cohort) =
        prepare_personal_brain_agent_admission_plan(&state, &brain_id, &actor).await?;
    let aliases = bootstrap_cohort_identity_aliases(&cohort, &server_timestamp(&state))?;
    {
        let mut store = state.store.lock().map_err(lock_error)?;
        for alias in &aliases {
            store.record_identity_alias(alias)?;
        }
    }
    let status = if plan.agents.is_empty() {
        "ready"
    } else {
        "setting_up"
    };
    Ok(Json(personal_brain_agent_admission_response(
        &brain_id, &plan, status,
    )?))
}

pub(crate) async fn commit_personal_brain_agent_admissions_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<PersonalBrainAgentAdmissionResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: CommitPersonalBrainAgentAdmissionsRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let (plan, cohort) =
        prepare_personal_brain_agent_admission_plan(&state, &brain_id, &actor).await?;
    let preview = personal_brain_agent_admission_response(&brain_id, &plan, "setting_up")?;
    if request.plan_id != preview.plan_id {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Personal Brain agent admission plan is stale; retry preparation",
        ));
    }
    let expected = plan
        .agents
        .iter()
        .flat_map(|agent| {
            plan.folder_key_versions
                .iter()
                .map(move |(folder, version)| {
                    (folder.to_string(), *version, agent.npub.to_string())
                })
        })
        .collect::<BTreeSet<_>>();
    let provided = request
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
    if expected != provided || request.participant_grants.len() != expected.len() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Personal Brain admission grants must exactly match the prepared plan",
        ));
    }
    let now = server_timestamp(&state);
    let grants =
        bootstrap_grant_requests_to_metadata(&request.participant_grants, actor.as_str(), &now)?;
    let control_records = grants
        .iter()
        .map(folder_key_grant_sync_record)
        .collect::<Result<Vec<_>, _>>()?;
    let aliases = bootstrap_cohort_identity_aliases(&cohort, &now)?;
    {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.commit_personal_brain_agent_admissions(
            &brain_id,
            &plan,
            &actor,
            &grants,
            &control_records,
            &now,
        )?;
        for alias in &aliases {
            store.record_identity_alias(alias)?;
        }
    }
    for agent in &plan.agents {
        state.publish_access_update_for(&brain_id, agent.npub.as_str());
    }
    Ok(Json(personal_brain_agent_admission_response(
        &brain_id, &plan, "ready",
    )?))
}

pub(crate) async fn replace_personal_agent_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let _actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let _brain_id = BrainId::new(brain_id)?;
    Err(ApiError::new(
        StatusCode::UPGRADE_REQUIRED,
        "the singular Personal Agent replace/vacate workflow has been removed; update the client and use Personal Brain agent admission or scoped access controls",
    ))
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
    let principals = resolve_account_agent_principals(&state, &agent_npub).await?;
    let owner_key =
        NostrPublicKey::parse(principals.owner_npub.as_str()).map_err(nostr_identity_error)?;
    let brain_id = BrainId::new(format!("personal-{}", &owner_key.to_hex()[..16]))?;
    let existing = {
        let store = state.store.lock().map_err(lock_error)?;
        match store.load_brain(&brain_id) {
            Ok(stored) => {
                if stored.brain.owner_user_id.as_ref() != Some(&principals.owner_npub)
                    || !stored.personal_brain_agents.iter().any(|agent| {
                        agent.agent_npub == principals.agent_npub && agent.status == "ready"
                    })
                {
                    return Err(ApiError::new(
                        StatusCode::FORBIDDEN,
                        "the authenticated Agent is not ready in this account's Personal Brain",
                    ));
                }
                let mut response = metadata_response(stored);
                enrich_metadata_identities(&store, &mut response)?;
                Some(response)
            }
            Err(StoreError::MissingBrain { .. }) => None,
            Err(error) => return Err(error.into()),
        }
    };
    if let Some(mut brain) = existing {
        brain
            .identities
            .sort_by(|left, right| left.npub.cmp(&right.npub));
        return Ok(Json(BootstrapPersonalBrainForAgentResponse { brain }));
    }
    let output = bootstrap_personal_brain(
        brain_id.as_str(),
        "Personal Brain",
        principals.owner_npub.to_string(),
    )?;
    let cohort =
        resolve_bootstrap_account_cohort(&state, &principals.owner_email, &principals.owner_npub)
            .await?;
    let created_at = server_timestamp(&state);
    let identity_aliases = bootstrap_cohort_identity_aliases(&cohort, &created_at)?;
    {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.create_personal_brain_cohort_bootstrap_with_identities(
            &output,
            &[],
            &principals.agent_npub,
            &created_at,
            &identity_aliases,
            &cohort,
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
