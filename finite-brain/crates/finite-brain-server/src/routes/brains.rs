use crate::*;
use finite_brain_store::{MemberFolderRotation, MemberMountRotation};

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
    let actor_user_id = UserId::new(actor_npub.clone())?;
    let request: CreateBrainRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;

    let requested_organization_requester = match request.kind {
        CreateBrainKind::Personal if request.requesting_user_npub.is_some() => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Organization Brain requester identity is only valid for an Organization Brain",
            ));
        }
        CreateBrainKind::Personal => None,
        CreateBrainKind::Organization => request
            .requesting_user_npub
            .as_deref()
            .map(canonical_requesting_user_npub)
            .transpose()?
            .map(UserId::new)
            .transpose()?,
    };
    let organization_requester = if request.kind == CreateBrainKind::Organization {
        if state.agent_bootstrap_authorities.is_none() {
            if requested_organization_requester.is_some() {
                return Err(ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "authenticated Organization Brain requester verification is not configured",
                ));
            }
            None
        } else {
            match (
                try_resolve_account_agent_principals(&state, &actor_user_id).await?,
                requested_organization_requester,
            ) {
                (Some(principals), Some(requester)) if requester == principals.owner_npub => {
                    Some(requester)
                }
                (Some(_), Some(_)) => {
                    return Err(ApiError::new(
                        StatusCode::FORBIDDEN,
                        "authenticated requester does not own the signing Managed Agent Principal",
                    ));
                }
                (Some(_), None) => {
                    return Err(ApiError::new(
                        StatusCode::FORBIDDEN,
                        "Managed Agent Organization Brain creation requires authenticated requester context; retry from the authenticated chat turn",
                    ));
                }
                (None, Some(_)) => {
                    return Err(ApiError::new(
                        StatusCode::FORBIDDEN,
                        "direct human Organization Brain creation cannot provide a separate requester identity",
                    ));
                }
                (None, None) => None,
            }
        }
    } else {
        None
    };

    let personal_agent = match request.kind {
        CreateBrainKind::Organization
            if request.personal_agent_email.is_some() || request.personal_agent_npub.is_some() =>
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Personal Agent identity is only valid for a Personal Brain",
            ));
        }
        CreateBrainKind::Organization => None,
        CreateBrainKind::Personal => {
            let email_identity = match request.personal_agent_email.as_deref() {
                Some(email) => Some(resolve_identity_input(&state, email).await?),
                None => None,
            };
            let npub_identity = match request.personal_agent_npub.as_deref() {
                Some(npub) => Some(resolve_identity_input(&state, npub).await?),
                None => None,
            };
            if let (Some(email), Some(npub)) = (&email_identity, &npub_identity)
                && email.npub != npub.npub
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "personalAgentEmail and personalAgentNpub resolve to different Agent Principals",
                ));
            }
            let requested_agent = email_identity
                .as_ref()
                .or(npub_identity.as_ref())
                .ok_or_else(|| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "Personal Brain creation requires a Personal Agent email or npub",
                    )
                })?;
            let requested_agent_npub = UserId::new(requested_agent.npub.clone())?;
            let principals =
                resolve_account_agent_principals(&state, &requested_agent_npub).await?;
            if principals.owner_npub != UserId::new(actor_npub.clone())? {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "selected Personal Agent does not belong to the signed owner's account",
                ));
            }
            if let Some(email) = request.personal_agent_email.as_deref()
                && canonical_email(email)? != principals.managed_agent_email
            {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "Finite Identity returned a mismatched Managed Agent email",
                ));
            }
            Some(principals)
        }
    };
    if request.kind == CreateBrainKind::Personal {
        let already_has_personal_brain = {
            let store = state.store.lock().map_err(lock_error)?;
            store
                .list_visible_brains(&actor_user_id)?
                .iter()
                .any(|brain| {
                    brain.kind == BrainKind::Personal && brain.role == VisibleBrainRole::Owner
                })
        };
        if already_has_personal_brain {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "user already has a personal brain",
            ));
        }
    }
    let initial_agent = match request.kind {
        CreateBrainKind::Personal
            if request.initial_agent_email.is_some() || request.initial_agent_npub.is_some() =>
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Initial Organization Brain agent identity is only valid for an Organization Brain",
            ));
        }
        CreateBrainKind::Personal => None,
        CreateBrainKind::Organization => {
            let email_identity = match request.initial_agent_email.as_deref() {
                Some(email) => Some(resolve_identity_input(&state, email).await?),
                None => None,
            };
            let npub_identity = match request.initial_agent_npub.as_deref() {
                Some(npub) => Some(resolve_identity_input(&state, npub).await?),
                None => None,
            };
            if let (Some(email), Some(npub)) = (&email_identity, &npub_identity)
                && email.npub != npub.npub
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "initialAgentEmail and initialAgentNpub resolve to different Agent Principals",
                ));
            }
            if let Some(requested_agent) = email_identity.as_ref().or(npub_identity.as_ref()) {
                if organization_requester.is_some() {
                    return Err(ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "requestingUserNpub and initial agent identity are mutually exclusive bootstrap paths",
                    ));
                }
                let requested_agent_npub = UserId::new(requested_agent.npub.clone())?;
                let principals =
                    resolve_account_agent_principals(&state, &requested_agent_npub).await?;
                if principals.owner_npub != UserId::new(actor_npub.clone())? {
                    return Err(ApiError::new(
                        StatusCode::FORBIDDEN,
                        "selected Organization Brain agent does not belong to the signed owner's account",
                    ));
                }
                if let Some(email) = request.initial_agent_email.as_deref()
                    && canonical_email(email)? != principals.managed_agent_email
                {
                    return Err(ApiError::new(
                        StatusCode::FORBIDDEN,
                        "Finite Identity returned a mismatched Managed Agent email",
                    ));
                }
                Some(principals)
            } else {
                None
            }
        }
    };
    let bootstrap_cohort = match request.kind {
        CreateBrainKind::Personal => match personal_agent.as_ref() {
            Some(principals) => Some(
                resolve_bootstrap_account_cohort(
                    &state,
                    &principals.owner_email,
                    &principals.owner_npub,
                )
                .await?,
            ),
            None => None,
        },
        CreateBrainKind::Organization => {
            if let Some(principals) = initial_agent.as_ref() {
                Some(
                    resolve_bootstrap_account_cohort(
                        &state,
                        &principals.owner_email,
                        &principals.owner_npub,
                    )
                    .await?,
                )
            } else if let Some(requester) = organization_requester.as_ref() {
                let principals = resolve_account_agent_principals(&state, &actor_user_id).await?;
                if &principals.owner_npub != requester {
                    return Err(ApiError::new(
                        StatusCode::FORBIDDEN,
                        "authenticated requester account changed during bootstrap",
                    ));
                }
                Some(
                    resolve_bootstrap_account_cohort(&state, &principals.owner_email, requester)
                        .await?,
                )
            } else {
                None
            }
        }
    };
    let output = match request.kind {
        CreateBrainKind::Personal => {
            bootstrap_personal_brain(request.brain_id, request.name, actor_npub.clone())?
        }
        CreateBrainKind::Organization => {
            if let Some(cohort) = bootstrap_cohort.as_ref() {
                let human = cohort
                    .participants
                    .iter()
                    .find(|participant| participant.relationship == "human")
                    .expect("bootstrap cohort resolver always returns one human");
                let agents = cohort
                    .participants
                    .iter()
                    .filter(|participant| participant.relationship == "account_agent")
                    .map(|participant| participant.npub.to_string())
                    .collect::<Vec<_>>();
                bootstrap_organization_brain_with_account_cohort(
                    request.brain_id,
                    request.name,
                    human.npub.to_string(),
                    agents,
                )?
            } else if let Some(agent) = initial_agent.as_ref() {
                bootstrap_organization_brain_with_requester(
                    request.brain_id,
                    request.name,
                    actor_npub.clone(),
                    agent.agent_npub.as_str().to_owned(),
                )?
            } else if let Some(requester) = organization_requester.as_ref() {
                bootstrap_organization_brain_with_requester(
                    request.brain_id,
                    request.name,
                    actor_npub.clone(),
                    requester.as_str().to_owned(),
                )?
            } else {
                bootstrap_organization_brain(request.brain_id, request.name, actor_npub.clone())?
            }
        }
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
        if request.kind == CreateBrainKind::Personal
            && let Some(cohort) = bootstrap_cohort.as_ref()
        {
            let created_at = server_timestamp(&state);
            let identity_aliases = bootstrap_cohort_identity_aliases(cohort, &created_at)?;
            store.create_personal_brain_cohort_bootstrap_with_identities(
                &output,
                &grants,
                &actor_user_id,
                &created_at,
                &identity_aliases,
                cohort,
            )?;
        } else if request.kind == CreateBrainKind::Organization
            && let Some(cohort) = bootstrap_cohort.as_ref()
        {
            let created_at = server_timestamp(&state);
            let identity_aliases = bootstrap_cohort_identity_aliases(cohort, &created_at)?;
            store.create_organization_brain_cohort_bootstrap_with_identities(
                &output,
                &grants,
                &identity_aliases,
                cohort,
                &created_at,
            )?;
        } else if let Some(principals) = personal_agent.as_ref() {
            let created_at = server_timestamp(&state);
            let identity_aliases = account_agent_identity_aliases(principals, &created_at)?;
            store.create_personal_brain_bootstrap_with_identities(
                &output,
                &grants,
                &principals.agent_npub,
                &UserId::new(actor_npub.clone())?,
                &created_at,
                &identity_aliases,
            )?;
        } else if let Some(principals) = initial_agent.as_ref() {
            let created_at = server_timestamp(&state);
            let identity_aliases = account_agent_identity_aliases(principals, &created_at)?;
            store.create_brain_bootstrap_with_identities(&output, &grants, &identity_aliases)?;
        } else {
            store.create_brain_bootstrap(&output, &grants)?;
        }
        store.load_brain(&brain_id)?
    };

    let mut response = metadata_response(stored);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_metadata_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn resolve_bootstrap_account_cohort(
    state: &ServerState,
    human_email: &str,
    expected_human: &UserId,
) -> Result<BootstrapAccountCohort, ApiError> {
    let human_email = canonical_email(human_email)?;
    let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Brain account-agent authority is not configured",
        )
    })?;
    let roster: CoreAccountAgentRosterResponse = post_authority_json(
        &format!(
            "{}/api/core/v1/brain/account-agent-roster",
            authorities.core_base_url
        ),
        "Authorization",
        &format!("Bearer {}", authorities.core_token),
        &serde_json::json!({ "verifiedEmail": human_email }),
        "Finite Core bootstrap account-agent roster",
    )
    .await?;
    if canonical_email(&roster.human_mailbox)? != human_email || roster.account_id.trim().is_empty()
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Finite Core returned a mismatched bootstrap roster",
        ));
    }
    let eligible = roster
        .agents
        .iter()
        .filter(|agent| agent.eligible)
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "account has no eligible agents for Brain bootstrap",
        ));
    }
    let resolved: IdentityCohortParticipantResolutionResponse = post_authority_json(
        &format!(
            "{}/api/v1/operator/brain/participant-resolution",
            authorities.identity_base_url
        ),
        "X-Finite-Operator-Token",
        &authorities.identity_token,
        &serde_json::json!({
            "workosUserId": roster.account_id,
            "humanMailbox": human_email,
            "managedAgentNames": eligible
                .iter()
                .map(|agent| &agent.managed_agent_nip05)
                .collect::<Vec<_>>(),
        }),
        "Finite Identity bootstrap participant resolution",
    )
    .await?;
    let human_npub = UserId::new(resolved.human.npub)?;
    if resolved.human.relationship != "human" || &human_npub != expected_human {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "authenticated requester does not match the bootstrap human account",
        ));
    }
    let mut agents_by_nip05 = resolved
        .agents
        .into_iter()
        .map(|agent| Ok((canonical_email(&agent.nip05)?, agent)))
        .collect::<Result<BTreeMap<_, _>, ApiError>>()?;
    let mut participants = vec![StoredCohortParticipant {
        relationship: "human".to_owned(),
        name: resolved.human.name,
        nip05: human_email.clone(),
        npub: human_npub,
    }];
    let mut principals = BTreeSet::from([expected_human.clone()]);
    for roster_agent in eligible {
        let nip05 = canonical_email(&roster_agent.managed_agent_nip05)?;
        let resolved_agent = agents_by_nip05.remove(&nip05).ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "Finite Identity omitted an eligible bootstrap agent",
            )
        })?;
        let npub = UserId::new(resolved_agent.npub)?;
        if resolved_agent.relationship != "account_agent" || !principals.insert(npub.clone()) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "bootstrap participant principals are ambiguous",
            ));
        }
        participants.push(StoredCohortParticipant {
            relationship: "account_agent".to_owned(),
            name: roster_agent.display_name.clone(),
            nip05,
            npub,
        });
    }
    if !agents_by_nip05.is_empty() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Finite Identity returned agents outside the bootstrap roster",
        ));
    }
    Ok(BootstrapAccountCohort {
        account_id: roster.account_id,
        human_email,
        roster_revision: roster.roster_revision,
        participants,
    })
}

pub(crate) fn bootstrap_cohort_identity_aliases(
    cohort: &BootstrapAccountCohort,
    updated_at: &str,
) -> Result<Vec<IdentityAlias>, ApiError> {
    cohort
        .participants
        .iter()
        .map(|participant| {
            let key =
                NostrPublicKey::parse(participant.npub.as_str()).map_err(nostr_identity_error)?;
            Ok(IdentityAlias {
                npub: participant.npub.clone(),
                hex_public_key: key.to_hex(),
                preferred_nip05: Some(participant.nip05.clone()),
                nip05_verified_at: Some(updated_at.to_owned()),
                nip05_relays: Vec::new(),
                updated_at: updated_at.to_owned(),
            })
        })
        .collect()
}

pub(crate) async fn permanent_departures_for_cohort_invitation(
    state: &ServerState,
    invitation_id: &str,
) -> Result<BTreeMap<UserId, String>, ApiError> {
    let plan = {
        let store = state.store.lock().map_err(lock_error)?;
        store
            .load_cohort_invitation_plan(invitation_id)?
            .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "cohort plan is missing"))?
    };
    let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Brain account-agent authority is not configured",
        )
    })?;
    let roster: CoreAccountAgentRosterResponse = post_authority_json(
        &format!(
            "{}/api/core/v1/brain/account-agent-roster",
            authorities.core_base_url
        ),
        "Authorization",
        &format!("Bearer {}", authorities.core_token),
        &serde_json::json!({ "verifiedEmail": plan.human_email }),
        "Finite Core invitation acceptance roster",
    )
    .await?;
    if roster.account_id != plan.account_id || roster.roster_revision < plan.roster_revision {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "invitation acceptance roster does not match the committed account",
        ));
    }
    let current = roster
        .agents
        .iter()
        .map(|agent| Ok((canonical_email(&agent.managed_agent_nip05)?, agent)))
        .collect::<Result<BTreeMap<_, _>, ApiError>>()?;
    let mut departures = BTreeMap::new();
    for participant in plan
        .participants
        .iter()
        .filter(|participant| participant.relationship == "account_agent")
    {
        let nip05 = canonical_email(&participant.nip05)?;
        let permanently_departed = current.get(&nip05).is_none_or(|agent| {
            matches!(
                agent.lifecycle_state.trim().to_ascii_lowercase().as_str(),
                "departed" | "retired" | "deleted" | "unlinked"
            ) || matches!(
                agent
                    .exclusion_reason
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
                    .as_str(),
                "permanent_agent_departure"
                    | "agent_permanently_departed"
                    | "agent_unlinked"
                    | "agent_retired"
                    | "agent_deleted"
            )
        });
        if permanently_departed {
            departures.insert(
                participant.npub.clone(),
                "permanent_agent_departure".to_owned(),
            );
        }
    }
    Ok(departures)
}

pub(crate) fn validate_acceptance_narrowing(
    requested: &[String],
    departures: &BTreeMap<UserId, String>,
) -> Result<(), ApiError> {
    let requested = requested
        .iter()
        .map(|npub| {
            let npub = canonical_npub_from_public_key_input(npub)?;
            UserId::new(npub).map_err(ApiError::from)
        })
        .collect::<Result<BTreeSet<_>, ApiError>>()?;
    let expected = departures.keys().cloned().collect::<BTreeSet<_>>();
    if requested != expected {
        let proposal = departures
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            if proposal.is_empty() {
                "acceptance narrowing cannot add, substitute, or remove participants".to_owned()
            } else {
                format!("acceptance narrowing required; exact remove-only proposal: {proposal}")
            },
        ));
    }
    Ok(())
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
    let mounted_folders = {
        let store = state.store.lock().map_err(lock_error)?;
        store.mounted_folder_projection(&brain_id, &UserId::new(actor_npub.clone())?)?
    };

    let mut response = metadata_response_for_actor(stored, mounted_folders, &actor_npub);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_metadata_identities(&store, &mut response)?;
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
    let export = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_metadata_visible(&stored, &actor)?;
        store.encrypted_brain_export(&brain_id, &actor_id)?
    };
    Ok(Json(encrypted_brain_export_response(export)))
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
    reject_legacy_finite_vip_principal_write(&target_npub)?;
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

pub(crate) async fn remove_member_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    reject_legacy_finite_vip_principal_write(&target_npub)?;
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
    let removed_participants = {
        let store = state.store.lock().map_err(lock_error)?;
        store.member_removal_participants(&brain_id, &target)?
    };
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
        for removed in &removed_participants {
            notification_state.publish_access_update_for(&affected_brain_id, removed.as_str());
        }
    }
    Ok(Json(response))
}

pub(crate) async fn preview_member_removal_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
) -> Result<Json<PreviewMemberRemovalResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    reject_legacy_finite_vip_principal_write(&target_npub)?;
    let brain_id = BrainId::new(brain_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub)?;
    let (removed_participant_npubs, folder_access_removals) = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let access_plan = store.member_removal_access_plan(&brain_id, &target)?;
        (
            access_plan
                .removed_members
                .into_iter()
                .map(|participant| participant.to_string())
                .collect(),
            access_plan
                .folder_access_removals
                .into_iter()
                .map(
                    |(folder_id, participants)| PreviewMemberRemovalFolderAccessResponse {
                        folder_id: folder_id.to_string(),
                        removed_participant_npubs: participants
                            .into_iter()
                            .map(|participant| participant.to_string())
                            .collect(),
                    },
                )
                .collect(),
        )
    };
    Ok(Json(PreviewMemberRemovalResponse {
        brain_id: brain_id.to_string(),
        target_npub: target.to_string(),
        removed_participant_npubs,
        folder_access_removals,
    }))
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
    reject_legacy_finite_vip_principal_write(&target_npub)?;
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
    reject_legacy_finite_vip_principal_write(&target_npub)?;
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
            enrich_brain_invitation_identities(&store, response)?;
            attach_invitation_public_url(&state, response);
        }
        responses
    };
    Ok(Json(BrainInvitationListResponse { invitations }))
}

pub(crate) async fn preview_brain_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<PreviewBrainInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: PreviewBrainInvitationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let (preview, _) = build_invitation_preview(&state, &actor, &brain_id, request, false).await?;
    Ok(Json(preview))
}

pub(crate) async fn preview_pending_invitation_conversion_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, invitation_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<PreviewBrainInvitationResponse>, ApiError> {
    let actor = validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        (!body.is_empty()).then_some(&body),
    )?;
    let request = if body.is_empty() {
        PreviewPendingInvitationConversionRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?
    };
    let brain_id = BrainId::new(brain_id)?;
    let (preview, _, _) = pending_invitation_conversion_preview(
        &state,
        &actor,
        &brain_id,
        &invitation_id,
        request.approved_exclusions,
    )
    .await?;
    Ok(Json(preview))
}

async fn pending_invitation_conversion_preview(
    state: &ServerState,
    actor: &str,
    brain_id: &BrainId,
    invitation_id: &str,
    approved_exclusions: Vec<String>,
) -> Result<
    (
        PreviewBrainInvitationResponse,
        String,
        StoredBrainInvitation,
    ),
    ApiError,
> {
    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(brain_id)?;
        ensure_brain_admin(&stored, actor)?;
        let invitation = store.load_brain_invitation(invitation_id)?;
        if invitation.brain_id != *brain_id
            || invitation.status != LinkStatus::Pending
            || invitation.target_kind != BrainInvitationTargetKind::EmailBootstrap
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "invitation is not a pending legacy mailbox invitation",
            ));
        }
        invitation
    };
    let target_email = invitation.invited_email.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            "pending mailbox invitation has no mailbox",
        )
    })?;
    if !finite_vip_email(&target_email) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "only Finite VIP mailbox invitations require cohort conversion",
        ));
    }
    let (mut preview, account_id) = build_invitation_preview(
        state,
        actor,
        brain_id,
        PreviewBrainInvitationRequest {
            target_email,
            folder_only: invitation.folder_only,
            initial_folder_access: invitation
                .initial_folder_access
                .iter()
                .map(ToString::to_string)
                .collect(),
            expires_at: invitation.expires_at.clone(),
            approved_exclusions,
        },
        true,
    )
    .await?;
    preview.plan_id = format!(
        "cohort-conversion-{:x}",
        Sha256::digest(format!("{}:{}", invitation.id, preview.plan_id).as_bytes())
    );
    Ok((preview, account_id, invitation))
}

pub(crate) async fn convert_pending_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, invitation_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<BrainInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let actor_user_id = UserId::new(actor.clone())?;
    let brain_id = BrainId::new(brain_id)?;
    let request: ConvertPendingInvitationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    if request.backup_reference.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invitation conversion requires an explicit backupReference",
        ));
    }

    let already_converted = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let invitation = store.load_brain_invitation(&invitation_id)?;
        if invitation.brain_id == brain_id
            && invitation.target_kind == BrainInvitationTargetKind::AccountCohort
            && store
                .load_cohort_invitation_plan(&invitation_id)?
                .is_some_and(|plan| plan.plan_id == request.plan_id)
        {
            Some(invitation)
        } else {
            None
        }
    };
    if let Some(invitation) = already_converted {
        let mut response = brain_invitation_response(invitation);
        attach_invitation_public_url(&state, &mut response);
        let store = state.store.lock().map_err(lock_error)?;
        enrich_brain_invitation_identities(&store, &mut response)?;
        return Ok(Json(response));
    }

    let (preview, account_id, invitation) = pending_invitation_conversion_preview(
        &state,
        &actor,
        &brain_id,
        &invitation_id,
        request.approved_exclusions.clone(),
    )
    .await?;
    if preview.plan_id != request.plan_id {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "invitation conversion plan is stale; review preflight again",
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
    let included_participants = preview.participants.iter().collect::<Vec<_>>();
    let expected_grants = preview
        .key_versions
        .iter()
        .flat_map(|key| {
            included_participants.iter().map(move |participant| {
                (
                    key.folder_id.clone(),
                    key.key_version,
                    participant.npub.clone(),
                )
            })
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
    if expected_grants != provided_grants
        || request.participant_grants.len() != expected_grants.len()
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "participant grants must exactly match the conversion preflight",
        ));
    }
    let converted_at = server_timestamp(&state);
    let grants =
        bootstrap_grant_requests_to_metadata(&request.participant_grants, &actor, &converted_at)?;
    let control_records = grants
        .iter()
        .map(folder_key_grant_sync_record)
        .collect::<Result<Vec<_>, _>>()?;
    let participants = included_participants
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
    let stored_exclusions = preview.excluded.clone();
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.convert_pending_email_invitation_to_account_cohort(
            &brain_id,
            &invitation.id,
            &preview.plan_id,
            &account_id,
            &preview.target_email,
            preview.roster_revision,
            &participants,
            &serde_json::to_string(&stored_exclusions).map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "invitation exclusions did not serialize",
                )
            })?,
            &serde_json::to_string(&preview.key_versions).map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "invitation key versions did not serialize",
                )
            })?,
            &grants,
            &control_records,
            &actor_user_id,
            &request.backup_reference,
            &converted_at,
        )?
    };
    let mut response = brain_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    let store = state.store.lock().map_err(lock_error)?;
    enrich_brain_invitation_identities(&store, &mut response)?;
    Ok(Json(response))
}

pub(crate) async fn build_invitation_preview(
    state: &ServerState,
    actor: &str,
    brain_id: &BrainId,
    request: PreviewBrainInvitationRequest,
    replaces_pending_invitation: bool,
) -> Result<(PreviewBrainInvitationResponse, String), ApiError> {
    let target_email = canonical_email(&request.target_email)?;
    if !finite_vip_email(&target_email) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "account-cohort preview requires a Finite VIP mailbox",
        ));
    }
    OffsetDateTime::parse(&request.expires_at, &Rfc3339)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "expiresAt must be RFC 3339"))?;
    let selected_folders = selected_folder_ids(&request.initial_folder_access)?;
    let approved_exclusions = request
        .approved_exclusions
        .iter()
        .map(|nip05| canonical_email(nip05))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if approved_exclusions.len() != request.approved_exclusions.len() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "approvedExclusions must not contain duplicates",
        ));
    }
    if request.folder_only && selected_folders.len() != 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Folder invitation preview requires exactly one Folder",
        ));
    }
    let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Brain account-agent authority is not configured",
        )
    })?;
    let capacity_now = server_timestamp(state);
    let capacity_now_value = OffsetDateTime::parse(&capacity_now, &Rfc3339).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server clock produced an invalid timestamp",
        )
    })?;

    if !replaces_pending_invitation {
        let existing = {
            let store = state.store.lock().map_err(lock_error)?;
            let stored = store.load_brain(brain_id)?;
            ensure_brain_admin(&stored, actor)?;
            for folder_id in &selected_folders {
                if !stored
                    .brain
                    .folders
                    .iter()
                    .any(|folder| folder.id == *folder_id)
                {
                    return Err(ApiError::new(StatusCode::NOT_FOUND, "Folder not found"));
                }
            }
            let invitation =
                store
                    .list_brain_invitations(brain_id)?
                    .into_iter()
                    .find(|invitation| {
                        invitation.status == LinkStatus::Pending
                            && OffsetDateTime::parse(&invitation.expires_at, &Rfc3339)
                                .is_ok_and(|expires_at| expires_at > capacity_now_value)
                            && invitation.target_kind == BrainInvitationTargetKind::AccountCohort
                            && invitation.invited_email.as_deref() == Some(target_email.as_str())
                            && invitation.folder_only == request.folder_only
                            && invitation.initial_folder_access == selected_folders
                    });
            invitation
                .map(|invitation| {
                    let plan = store
                        .load_cohort_invitation_plan(&invitation.id)?
                        .ok_or_else(|| StoreError::BrokenInvariant {
                            reason: "pending account-cohort invitation has no immutable plan"
                                .to_owned(),
                        })?;
                    let usage =
                        store.brain_invitation_capacity_usage(brain_id, None, &capacity_now)?;
                    Ok::<_, StoreError>((invitation, plan, usage))
                })
                .transpose()?
        };
        if let Some((invitation, plan, usage)) = existing {
            let exclusions =
                serde_json::from_str::<Vec<InvitationPlanExclusionResponse>>(&plan.exclusions_json)
                    .map_err(|_| {
                        ApiError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "stored invitation exclusions are invalid",
                        )
                    })?;
            let stored_explicit_exclusions = exclusions
                .iter()
                .filter(|exclusion| exclusion.reason == "explicit_capacity_reduction")
                .map(|exclusion| canonical_email(&exclusion.nip05))
                .collect::<Result<BTreeSet<_>, _>>()?;
            let stored_required_exclusions = exclusions
                .iter()
                .filter(|exclusion| exclusion.reason != "explicit_capacity_reduction")
                .map(|exclusion| canonical_email(&exclusion.nip05))
                .collect::<Result<BTreeSet<_>, _>>()?;
            let requested_explicit_exclusions = approved_exclusions
                .difference(&stored_required_exclusions)
                .cloned()
                .collect::<BTreeSet<_>>();
            if stored_explicit_exclusions != requested_explicit_exclusions {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "a pending invitation already exists for this scope; revoke it before changing agent exclusions",
                ));
            }
            let key_versions = serde_json::from_str::<Vec<InvitationPlanKeyVersionResponse>>(
                &plan.key_versions_json,
            )
            .map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "stored invitation key versions are invalid",
                )
            })?;
            let maximum_sync_records =
                BRAIN_CAPACITY_ENVELOPE.sync_records - BRAIN_CAPACITY_ENVELOPE.folders;
            let mut blockers = Vec::new();
            if usage.members > BRAIN_CAPACITY_ENVELOPE.members {
                blockers.push("members".to_owned());
            }
            if usage.folder_access_entries > BRAIN_CAPACITY_ENVELOPE.folder_access_entries {
                blockers.push("folder_access_entries".to_owned());
            }
            if usage.folder_key_grants > BRAIN_CAPACITY_ENVELOPE.folder_key_grants {
                blockers.push("folder_key_grants".to_owned());
            }
            if usage.sync_records > maximum_sync_records {
                blockers.push("sync_records".to_owned());
            }
            if usage.pending_invitations > BRAIN_CAPACITY_ENVELOPE.invitations {
                blockers.push("pending_invitations".to_owned());
            }
            let capacity = InvitationPlanCapacityResponse {
                fits: blockers.is_empty(),
                resulting_members: usage.members,
                maximum_members: BRAIN_CAPACITY_ENVELOPE.members,
                resulting_folder_access_entries: usage.folder_access_entries,
                maximum_folder_access_entries: BRAIN_CAPACITY_ENVELOPE.folder_access_entries,
                resulting_folder_key_grants: usage.folder_key_grants,
                maximum_folder_key_grants: BRAIN_CAPACITY_ENVELOPE.folder_key_grants,
                resulting_sync_records: usage.sync_records,
                maximum_sync_records,
                resulting_pending_invitations: usage.pending_invitations,
                maximum_pending_invitations: BRAIN_CAPACITY_ENVELOPE.invitations,
                blockers,
            };
            return Ok((
                PreviewBrainInvitationResponse {
                    plan_id: plan.plan_id,
                    target_email,
                    scope: InvitationPlanScopeResponse {
                        kind: if request.folder_only {
                            "folder"
                        } else {
                            "brain"
                        }
                        .to_owned(),
                        brain_id: brain_id.to_string(),
                        folder_id: request.folder_only.then(|| selected_folders[0].to_string()),
                    },
                    roster_revision: plan.roster_revision,
                    participants: plan
                        .participants
                        .into_iter()
                        .map(|participant| InvitationPlanParticipantResponse {
                            relationship: participant.relationship,
                            name: participant.name,
                            nip05: participant.nip05,
                            npub: participant.npub.to_string(),
                            ready: true,
                        })
                        .collect(),
                    excluded: exclusions,
                    key_versions,
                    capacity,
                    expires_at: invitation.expires_at,
                },
                plan.account_id,
            ));
        }
    }

    let (
        key_versions,
        current_members,
        current_folder_access,
        current_grants,
        capacity_usage,
        capacity_reservations,
        matching_pending_invitation,
    ) = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(brain_id)?;
        ensure_brain_admin(&stored, actor)?;
        for folder_id in &selected_folders {
            if !stored
                .brain
                .folders
                .iter()
                .any(|folder| folder.id == *folder_id)
            {
                return Err(ApiError::new(StatusCode::NOT_FOUND, "Folder not found"));
            }
        }
        let selected = selected_folders.iter().collect::<BTreeSet<_>>();
        let key_versions = stored
            .brain
            .folders
            .iter()
            .filter(|folder| {
                if request.folder_only {
                    selected.contains(&folder.id)
                } else {
                    folder.access == FolderAccessMode::AllMembers || selected.contains(&folder.id)
                }
            })
            .map(|folder| InvitationPlanKeyVersionResponse {
                folder_id: folder.id.as_str().to_owned(),
                key_version: folder.current_key_version,
            })
            .collect::<Vec<_>>();
        let current_members = stored
            .brain
            .members
            .iter()
            .map(|member| member.user_id.to_string())
            .collect::<BTreeSet<_>>();
        let current_folder_access = stored
            .folder_access
            .iter()
            .map(|(folder_id, users)| {
                (
                    folder_id.to_string(),
                    users
                        .iter()
                        .map(ToString::to_string)
                        .collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let current_grants = stored
            .grants
            .iter()
            .map(|grant| {
                (
                    grant.folder_id.to_string(),
                    grant.key_version,
                    grant.recipient_npub.to_string(),
                )
            })
            .collect::<BTreeSet<_>>();
        let matching_pending_invitation = replaces_pending_invitation
            .then(|| {
                store
                    .list_brain_invitations(brain_id)?
                    .into_iter()
                    .find(|invitation| {
                        invitation.status == LinkStatus::Pending
                            && OffsetDateTime::parse(&invitation.expires_at, &Rfc3339)
                                .is_ok_and(|expires_at| expires_at > capacity_now_value)
                            && invitation.target_kind == BrainInvitationTargetKind::EmailBootstrap
                            && invitation.invited_email.as_deref() == Some(target_email.as_str())
                            && invitation.folder_only == request.folder_only
                            && invitation.initial_folder_access == selected_folders
                    })
                    .map(|invitation| invitation.id)
                    .ok_or_else(|| {
                        ApiError::new(
                            StatusCode::CONFLICT,
                            "pending mailbox invitation changed before conversion preflight",
                        )
                    })
            })
            .transpose()?;
        let capacity_usage = store.brain_invitation_capacity_usage(
            brain_id,
            matching_pending_invitation.as_deref(),
            &capacity_now,
        )?;
        let capacity_reservations = store.brain_invitation_capacity_reservations(
            brain_id,
            matching_pending_invitation.as_deref(),
            &capacity_now,
        )?;
        (
            key_versions,
            current_members,
            current_folder_access,
            current_grants,
            capacity_usage,
            capacity_reservations,
            matching_pending_invitation,
        )
    };

    let roster: CoreAccountAgentRosterResponse = post_authority_json(
        &format!(
            "{}/api/core/v1/brain/account-agent-roster",
            authorities.core_base_url
        ),
        "Authorization",
        &format!("Bearer {}", authorities.core_token),
        &serde_json::json!({ "verifiedEmail": target_email }),
        "Finite Core account-agent roster",
    )
    .await?;
    if canonical_email(&roster.human_mailbox)? != target_email
        || roster.account_id.trim().is_empty()
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Finite Core returned a mismatched account roster",
        ));
    }
    let eligible_agent_names = roster
        .agents
        .iter()
        .filter(|agent| agent.eligible)
        .map(|agent| canonical_email(&agent.managed_agent_nip05))
        .collect::<Result<Vec<_>, _>>()?;
    if roster.agents.iter().any(|agent| {
        agent.display_name.trim().is_empty()
            || agent.principal_binding_reference.trim().is_empty()
            || agent.lifecycle_state.trim().is_empty()
            || (!agent.eligible && agent.exclusion_reason.as_deref().unwrap_or("").is_empty())
    }) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Finite Core returned an incomplete account roster",
        ));
    }
    let resolved: IdentityCohortParticipantResolutionResponse = post_authority_json(
        &format!(
            "{}/api/v1/operator/brain/participant-resolution",
            authorities.identity_base_url
        ),
        "X-Finite-Operator-Token",
        &authorities.identity_token,
        &serde_json::json!({
            "workosUserId": roster.account_id,
            "humanMailbox": target_email,
            "managedAgentNames": eligible_agent_names,
        }),
        "Finite Identity cohort participant resolution",
    )
    .await?;
    if resolved.human.relationship != "human"
        || canonical_email(&resolved.human.nip05)? != target_email
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Finite Identity returned a mismatched human participant",
        ));
    }

    let human_npub = UserId::new(resolved.human.npub.clone())?;
    let mut principals = BTreeSet::from([human_npub.to_string()]);
    let mut resolved_agents = resolved
        .agents
        .into_iter()
        .map(|participant| {
            let nip05 = canonical_email(&participant.nip05)?;
            Ok((nip05, participant))
        })
        .collect::<Result<BTreeMap<_, _>, ApiError>>()?;
    let mut participants = vec![InvitationPlanParticipantResponse {
        relationship: "human".to_owned(),
        name: resolved.human.name,
        nip05: target_email.clone(),
        npub: human_npub.to_string(),
        ready: true,
    }];
    for agent in roster.agents.iter().filter(|agent| agent.eligible) {
        let nip05 = canonical_email(&agent.managed_agent_nip05)?;
        let participant = resolved_agents.remove(&nip05).ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "Finite Identity did not resolve every eligible account agent",
            )
        })?;
        if participant.relationship != "account_agent" {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "Finite Identity returned an invalid participant relationship",
            ));
        }
        let npub = UserId::new(participant.npub)?;
        if !principals.insert(npub.to_string()) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "Finite Identity returned duplicate cohort principals",
            ));
        }
        participants.push(InvitationPlanParticipantResponse {
            relationship: participant.relationship,
            name: participant.name,
            nip05,
            npub: npub.to_string(),
            ready: true,
        });
    }
    if !resolved_agents.is_empty() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Finite Identity returned agents outside the authoritative account roster",
        ));
    }

    let mut excluded = roster
        .agents
        .iter()
        .filter(|agent| !agent.eligible)
        .map(|agent| {
            Ok(InvitationPlanExclusionResponse {
                name: agent.display_name.clone(),
                nip05: canonical_email(&agent.managed_agent_nip05)?,
                reason: agent.exclusion_reason.clone().ok_or_else(|| {
                    ApiError::new(
                        StatusCode::CONFLICT,
                        "Finite Core omitted an exclusion reason",
                    )
                })?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    let excludable_agents = participants
        .iter()
        .filter(|participant| participant.relationship == "account_agent")
        .map(|participant| participant.nip05.clone())
        .collect::<BTreeSet<_>>();
    let already_excluded = excluded
        .iter()
        .map(|participant| participant.nip05.clone())
        .collect::<BTreeSet<_>>();
    if !approved_exclusions
        .iter()
        .all(|nip05| excludable_agents.contains(nip05) || already_excluded.contains(nip05))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "approvedExclusions may name only eligible account agents in this preflight",
        ));
    }
    let mut explicitly_excluded = participants
        .iter()
        .filter(|participant| approved_exclusions.contains(&participant.nip05))
        .map(|participant| InvitationPlanExclusionResponse {
            name: participant.name.clone(),
            nip05: participant.nip05.clone(),
            reason: "explicit_capacity_reduction".to_owned(),
        })
        .collect::<Vec<_>>();
    participants.retain(|participant| !approved_exclusions.contains(&participant.nip05));
    excluded.append(&mut explicitly_excluded);

    let added_members = if request.folder_only {
        0
    } else {
        participants
            .iter()
            .filter(|participant| {
                !current_members.contains(&participant.npub)
                    && !capacity_reservations.members.contains(&participant.npub)
            })
            .count()
    };
    let resulting_members = capacity_usage.members.saturating_add(added_members);
    let folder_access_additions = selected_folders
        .iter()
        .map(|folder_id| {
            participants
                .iter()
                .filter(|participant| {
                    !current_folder_access
                        .get(folder_id.as_str())
                        .is_some_and(|users| users.contains(&participant.npub))
                        && !capacity_reservations
                            .folder_access_entries
                            .contains(&(folder_id.to_string(), participant.npub.clone()))
                })
                .count()
        })
        .sum::<usize>();
    let resulting_folder_access_entries = capacity_usage
        .folder_access_entries
        .saturating_add(folder_access_additions);
    let missing_grants = key_versions
        .iter()
        .flat_map(|key| {
            participants.iter().map(move |participant| {
                (
                    key.folder_id.clone(),
                    key.key_version,
                    participant.npub.clone(),
                )
            })
        })
        .filter(|grant| {
            !current_grants.contains(grant)
                && !capacity_reservations.folder_key_grants.contains(grant)
        })
        .count();
    let resulting_folder_key_grants = capacity_usage
        .folder_key_grants
        .saturating_add(missing_grants);
    let resulting_sync_records = capacity_usage.sync_records.saturating_add(missing_grants);
    let resulting_pending_invitations =
        capacity_usage
            .pending_invitations
            .saturating_add(usize::from(
                !replaces_pending_invitation && matching_pending_invitation.is_none(),
            ));
    let mut blockers = Vec::new();
    if resulting_members > BRAIN_CAPACITY_ENVELOPE.members {
        blockers.push("members".to_owned());
    }
    if resulting_folder_access_entries > BRAIN_CAPACITY_ENVELOPE.folder_access_entries {
        blockers.push("folder_access_entries".to_owned());
    }
    if resulting_folder_key_grants > BRAIN_CAPACITY_ENVELOPE.folder_key_grants {
        blockers.push("folder_key_grants".to_owned());
    }
    let maximum_sync_records =
        BRAIN_CAPACITY_ENVELOPE.sync_records - BRAIN_CAPACITY_ENVELOPE.folders;
    if resulting_sync_records > maximum_sync_records {
        blockers.push("sync_records".to_owned());
    }
    if resulting_pending_invitations > BRAIN_CAPACITY_ENVELOPE.invitations {
        blockers.push("pending_invitations".to_owned());
    }
    let capacity = InvitationPlanCapacityResponse {
        fits: blockers.is_empty(),
        resulting_members,
        maximum_members: BRAIN_CAPACITY_ENVELOPE.members,
        resulting_folder_access_entries,
        maximum_folder_access_entries: BRAIN_CAPACITY_ENVELOPE.folder_access_entries,
        resulting_folder_key_grants,
        maximum_folder_key_grants: BRAIN_CAPACITY_ENVELOPE.folder_key_grants,
        resulting_sync_records,
        maximum_sync_records,
        resulting_pending_invitations,
        maximum_pending_invitations: BRAIN_CAPACITY_ENVELOPE.invitations,
        blockers,
    };
    let scope = InvitationPlanScopeResponse {
        kind: if request.folder_only {
            "folder"
        } else {
            "brain"
        }
        .to_owned(),
        brain_id: brain_id.as_str().to_owned(),
        folder_id: request
            .folder_only
            .then(|| selected_folders[0].as_str().to_owned()),
    };
    let plan_binding = serde_json::json!({
        "targetEmail": target_email,
        "scope": scope,
        "rosterRevision": roster.roster_revision,
        "participants": participants,
        "excluded": excluded,
        "keyVersions": key_versions,
        "capacity": capacity,
        "expiresAt": request.expires_at,
        "actor": actor,
        "roster": roster.agents.iter().map(|agent| serde_json::json!({
            "binding": agent.principal_binding_reference,
            "lifecycle": agent.lifecycle_state,
            "eligible": agent.eligible,
        })).collect::<Vec<_>>(),
    });
    let plan_bytes = serde_json::to_vec(&plan_binding).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "plan serialization failed",
        )
    })?;
    let plan_id = format!("cohort-plan-{:x}", Sha256::digest(plan_bytes));
    Ok((
        PreviewBrainInvitationResponse {
            plan_id,
            target_email,
            scope,
            roster_revision: roster.roster_revision,
            participants,
            excluded,
            key_versions,
            capacity,
            expires_at: request.expires_at,
        },
        roster.account_id,
    ))
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

    if finite_vip_email(&target_input) {
        let plan_id = request
            .plan_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::UPGRADE_REQUIRED,
                    "Finite VIP mailbox invitations now include the human and their ready account agents; update the client and run invitation preflight before retrying",
                )
            })?;
        let existing = {
            let store = state.store.lock().map_err(lock_error)?;
            let stored = store.load_brain(&brain_id)?;
            ensure_brain_admin(&stored, &actor)?;
            store.load_account_cohort_invitation_by_plan_id(&brain_id, plan_id)?
        };
        if let Some(invitation) = existing {
            let delivery_status = deliver_email_invitation(&state, &invitation)?;
            let mut response = brain_invitation_response(invitation);
            response.delivery_status = delivery_status;
            attach_invitation_public_url(&state, &mut response);
            let store = state.store.lock().map_err(lock_error)?;
            enrich_brain_invitation_identities(&store, &mut response)?;
            return Ok(Json(response));
        }
        let preview_request = PreviewBrainInvitationRequest {
            target_email: target_input.clone(),
            folder_only: request.folder_only,
            initial_folder_access: request.initial_folder_access.clone(),
            expires_at: request.expires_at.clone(),
            approved_exclusions: request.approved_exclusions.clone(),
        };
        let (preview, account_id) =
            build_invitation_preview(&state, &actor, &brain_id, preview_request, false).await?;
        if preview.plan_id != plan_id {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "invitation plan is stale; review the returned preflight again",
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
        let included_participants = preview.participants.iter().collect::<Vec<_>>();
        let expected_grants = preview
            .key_versions
            .iter()
            .flat_map(|key| {
                included_participants.iter().map(move |participant| {
                    (
                        key.folder_id.clone(),
                        key.key_version,
                        participant.npub.clone(),
                    )
                })
            })
            .collect::<BTreeSet<_>>();
        let provided_grants = request
            .participant_grants
            .iter()
            .map(|request| {
                canonical_npub_from_public_key_input(&request.grant.recipient_npub).map(
                    |recipient| {
                        (
                            request.folder_id.clone(),
                            request.grant.key_version,
                            recipient,
                        )
                    },
                )
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if expected_grants != provided_grants
            || request.participant_grants.len() != expected_grants.len()
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "participant grants must exactly match the preflight participant and Folder set",
            ));
        }
        let grants =
            bootstrap_grant_requests_to_metadata(&request.participant_grants, &actor, &created_at)?;
        let control_records = grants
            .iter()
            .map(folder_key_grant_sync_record)
            .collect::<Result<Vec<_>, _>>()?;
        let participants = included_participants
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
        let initial_folder_access = selected_folder_ids(&request.initial_folder_access)?;
        let invitation_scope_key = if request.folder_only {
            initial_folder_access
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        } else {
            "brain-membership".to_owned()
        };
        let id = generated_link_id(
            "invitation",
            &[brain_id.as_str(), plan_id, &invitation_scope_key],
            16,
        );
        let invite_code = generated_link_id(
            "invite",
            &[brain_id.as_str(), plan_id, &invitation_scope_key, "code"],
            16,
        );
        let accept_path = format!("/v1/brain-invitation-links/{invite_code}/accept");
        let stored_exclusions = preview.excluded.clone();
        let invitation = {
            let mut store = state.store.lock().map_err(lock_error)?;
            store.create_account_cohort_invitation(
                &brain_id,
                &id,
                plan_id,
                &account_id,
                &preview.target_email,
                preview.roster_revision,
                &participants,
                &serde_json::to_string(&stored_exclusions).map_err(|_| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "invitation exclusions did not serialize",
                    )
                })?,
                &serde_json::to_string(&preview.key_versions).map_err(|_| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "invitation key versions did not serialize",
                    )
                })?,
                request.folder_only,
                &initial_folder_access,
                &grants,
                &control_records,
                &invite_code,
                &accept_path,
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
        return Ok(Json(response));
    }

    let npub_target = if let Ok(public_key) = NostrPublicKey::parse(&target_input) {
        Some(public_key.to_npub().map_err(nostr_identity_error)?)
    } else {
        None
    };

    let invitation = if let Some(target_npub) = npub_target {
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
    } else {
        if !email_like(&target_input) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invitation target must be npub, hex, active finite.vip NIP-05, or email",
            ));
        }
        let invited_email = canonical_email(&target_input)?;
        let invite_unwrap_npub = UserId::new(canonical_npub_from_public_key_input(
            request.invite_unwrap_npub.as_deref().ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "inviteUnwrapNpub is required for email bootstrap invitations",
                )
            })?,
        )?)?;
        let bootstrap_payload_hash = request
            .bootstrap_payload_hash
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "bootstrapPayloadHash is required for email bootstrap invitations",
                )
            })?;
        let bootstrap_wrapped_event_json = request
            .bootstrap_wrapped_event_json
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "bootstrapWrappedEventJson is required for email bootstrap invitations",
                )
            })?;
        let bootstrap_authorization_event_json = request
            .bootstrap_authorization_event_json
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "bootstrapAuthorizationEventJson is required for email bootstrap invitations",
                )
            })?;
        validate_folder_key_grant_wrapper(bootstrap_wrapped_event_json, &invite_unwrap_npub)?;
        let selected_restricted_folder_access =
            selected_folder_ids(&request.initial_folder_access)?;
        let invitation_scope_key = if request.folder_only {
            selected_restricted_folder_access
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        } else {
            "brain-membership".to_owned()
        };
        let id = generated_link_id(
            "invitation",
            &[
                brain_id.as_str(),
                invited_email.as_str(),
                &invitation_scope_key,
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
                invited_email.as_str(),
                &invitation_scope_key,
                actor_user_id.as_str(),
                request.expires_at.as_str(),
                created_at.as_str(),
                "code",
            ],
            16,
        );
        let accept_path = format!("/v1/brain-invitation-links/{invite_code}/claim");
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let scope = email_bootstrap_scope_for_brain(
            &stored,
            &selected_restricted_folder_access,
            request.folder_only,
        )?;
        validate_email_bootstrap_authorization(
            bootstrap_authorization_event_json,
            &actor,
            &brain_id,
            &invited_email,
            &invite_unwrap_npub,
            bootstrap_payload_hash,
            &request.expires_at,
            &scope,
        )?;
        store.create_email_brain_invitation(
            &brain_id,
            &id,
            &invited_email,
            &invite_unwrap_npub,
            bootstrap_payload_hash,
            bootstrap_wrapped_event_json,
            bootstrap_authorization_event_json,
            &invite_code,
            &accept_path,
            &selected_restricted_folder_access,
            request.folder_only,
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
    {
        let store = state.store.lock().map_err(lock_error)?;
        let invitation = store.load_brain_invitation_by_code(&invite_code)?;
        if invitation.target_kind != BrainInvitationTargetKind::EmailBootstrap {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            }
            .into());
        }
    }
    Ok(text_response(public_invite_instructions_text()))
}

pub(crate) async fn post_proof_brain_invitation_instructions_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invite_code): AxumPath<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request: PostProofInviteInstructionsRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let invitation = load_post_proof_email_invitation(
        &state,
        &headers,
        &method,
        &uri,
        &invite_code,
        &body,
        &request,
    )
    .await?;
    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain(&invitation.brain_id)?
    };
    Ok(text_response(post_proof_invite_instructions_text(
        &state,
        &invitation,
        &stored,
    )))
}

pub(crate) async fn post_proof_brain_invitation_bootstrap_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invite_code): AxumPath<String>,
    body: Bytes,
) -> Result<Json<BrainInvitationResponse>, ApiError> {
    let request: PostProofInviteInstructionsRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let invitation = load_post_proof_email_invitation(
        &state,
        &headers,
        &method,
        &uri,
        &invite_code,
        &body,
        &request,
    )
    .await?;
    if invitation.status == LinkStatus::Pending && invitation.bootstrap_wrapped_event_json.is_none()
    {
        return Err(StoreError::UnavailableLink {
            kind: "brain invitation",
        }
        .into());
    }
    let mut response = brain_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_brain_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

async fn load_post_proof_email_invitation(
    state: &ServerState,
    headers: &HeaderMap,
    method: &Method,
    uri: &axum::http::Uri,
    invite_code: &str,
    body: &Bytes,
    request: &PostProofInviteInstructionsRequest,
) -> Result<StoredBrainInvitation, ApiError> {
    let actor = validate_request_auth(state, headers, method, uri, Some(body))?;
    let actor_user_id = UserId::new(actor)?;
    let invited_email = canonical_email(&request.email)?;
    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain_invitation_by_code(invite_code)?
    };
    if invitation.target_kind != BrainInvitationTargetKind::EmailBootstrap {
        return Err(StoreError::UnavailableLink {
            kind: "brain invitation",
        }
        .into());
    }
    if invitation.invited_email.as_deref() != Some(invited_email.as_str()) {
        return Err(StoreError::UnavailableLink {
            kind: "brain invitation",
        }
        .into());
    }
    if invitation.status == LinkStatus::Accepted
        && invitation.claimed_by_npub.as_ref() != Some(&actor_user_id)
    {
        return Err(StoreError::UnavailableLink {
            kind: "brain invitation",
        }
        .into());
    }
    validate_email_proof_window(
        &invitation,
        &request.email_proof_created_at,
        &server_timestamp(state),
    )?;
    verify_identity_authority_email_proof(state, invited_email.as_str(), &actor_user_id).await?;
    Ok(invitation)
}

pub(crate) async fn accept_brain_invitation_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invite_code): AxumPath<String>,
    body: Bytes,
) -> Result<Json<BrainInvitationResponse>, ApiError> {
    let actor = validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        (!body.is_empty()).then_some(&body),
    )?;
    let actor = UserId::new(actor)?;
    let narrowing: AcceptAccountCohortInvitationRequest = if body.is_empty() {
        AcceptAccountCohortInvitationRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?
    };
    let now = server_timestamp(&state);
    let pending = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain_invitation_by_code(&invite_code)?
    };
    if pending.target_kind == BrainInvitationTargetKind::EmailBootstrap
        && pending
            .invited_email
            .as_deref()
            .is_some_and(finite_vip_email)
    {
        return Err(ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            "this Finite VIP invitation must be converted to account-cohort access before acceptance; ask the Brain admin to update and retry",
        ));
    }
    let departures = if pending.target_kind == BrainInvitationTargetKind::AccountCohort {
        permanent_departures_for_cohort_invitation(&state, &pending.id).await?
    } else {
        BTreeMap::new()
    };
    validate_acceptance_narrowing(&narrowing.removed_participants, &departures)?;
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        if pending.target_kind == BrainInvitationTargetKind::AccountCohort {
            store.accept_account_cohort_invitation_by_code(
                &invite_code,
                &actor,
                &departures,
                &now,
            )?
        } else {
            store.accept_brain_invitation_by_code(&invite_code, &actor, &now)?
        }
    };
    let mut response = brain_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_brain_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn claim_email_brain_invitation_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invite_code): AxumPath<String>,
    body: Bytes,
) -> Result<Json<BrainInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let actor_user_id = UserId::new(actor.clone())?;
    let request: ClaimEmailBrainInvitationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let now = server_timestamp(&state);
    let invited_email = canonical_email(&request.email)?;

    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain_invitation_by_code(&invite_code)?
    };
    if invitation.target_kind == BrainInvitationTargetKind::EmailBootstrap
        && invitation
            .invited_email
            .as_deref()
            .is_some_and(finite_vip_email)
    {
        return Err(ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            "this Finite VIP invitation must be converted to account-cohort access before acceptance; ask the Brain admin to update and retry",
        ));
    }
    if invitation.target_kind != BrainInvitationTargetKind::EmailBootstrap {
        return Err(StoreError::UnavailableLink {
            kind: "brain invitation",
        }
        .into());
    }
    if invitation.invited_email.as_deref() != Some(invited_email.as_str()) {
        return Err(StoreError::UnavailableLink {
            kind: "brain invitation",
        }
        .into());
    }

    let invitation = if invitation.status == LinkStatus::Accepted {
        if invitation.claimed_by_npub.as_ref() == Some(&actor_user_id) {
            let mut invitation = invitation;
            invitation.duplicate_accept = true;
            invitation
        } else {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            }
            .into());
        }
    } else {
        validate_email_proof_window(&invitation, &request.email_proof_created_at, &now)?;
        verify_identity_authority_email_proof(&state, invited_email.as_str(), &actor_user_id)
            .await?;
        if let (Some(authorization), Some(invite_unwrap_npub), Some(payload_hash)) = (
            invitation.bootstrap_authorization_event_json.as_deref(),
            invitation.invite_unwrap_npub.as_ref(),
            invitation.bootstrap_payload_hash.as_deref(),
        ) {
            validate_email_bootstrap_authorization(
                authorization,
                invitation.created_by_npub.as_str(),
                &invitation.brain_id,
                invited_email.as_str(),
                invite_unwrap_npub,
                payload_hash,
                &invitation.expires_at,
                &invitation.bootstrap_scope,
            )?;
            let proof_event_json = request
                .invite_unwrap_proof_event_json
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "inviteUnwrapProofEventJson is required for email bootstrap claims",
                    )
                })?;
            validate_email_bootstrap_claim_proof(
                proof_event_json,
                invite_unwrap_npub,
                &invitation.brain_id,
                &invite_code,
                invited_email.as_str(),
                &actor_user_id,
                payload_hash,
                &request.email_proof_created_at,
            )?;
        } else {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "email bootstrap invitation is missing authorization metadata",
            ));
        }
        let grants = bootstrap_grant_requests_to_metadata(&request.grants, &actor, &now)?;
        let control_records = grants
            .iter()
            .map(folder_key_grant_sync_record)
            .collect::<Result<Vec<_>, _>>()?;
        let mut store = state.store.lock().map_err(lock_error)?;
        store.claim_email_brain_invitation_by_code_with_control_records(
            &invite_code,
            invited_email.as_str(),
            &actor_user_id,
            &grants,
            &control_records,
            &now,
        )?
    };

    let mut response = brain_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_brain_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}
