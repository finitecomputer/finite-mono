use crate::*;
use finite_brain_store::{
    ProvenanceOriginKind, StoredInvitationPlan, StoredPlanAgent, StoredPlanExclusion,
};

const PLAN_COMMIT_WINDOW_SECONDS: u64 = 15 * 60;
const COMMIT_INVITATION_EXPIRY_SECONDS: u64 = 14 * 24 * 60 * 60;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreAccountAgentRosterResponse {
    workos_user_id: String,
    human_mailbox: String,
    roster_revision: i64,
    #[serde(default)]
    agents: Vec<CoreRosterAgentEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreRosterAgentEntry {
    managed_agent_email: String,
    #[serde(default)]
    agent_npub: Option<String>,
    status: String,
}

/// Fully resolved invite set for one invited email: human Principal, grant-ready
/// agent Principals, and explicit exclusions for everything not grant-ready.
#[derive(Clone)]
pub(crate) struct PlanResolution {
    workos_user_id: Option<String>,
    human_email: String,
    human_npub: Option<String>,
    agents: Vec<InvitationPlanAgent>,
    exclusions: Vec<InvitationPlanExclusion>,
    roster_revision: Option<i64>,
}

/// Resolve the invited email through Finite Identity and, when it binds to a
/// Finite account, the Core account agent roster. Roster agents that are not
/// grant-ready become explicit exclusions, never silent drops.
pub(crate) async fn resolve_invitation_plan(
    state: &ServerState,
    email: &str,
) -> Result<PlanResolution, ApiError> {
    let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Brain account-agent authority is not configured",
        )
    })?;
    let roster: Option<CoreAccountAgentRosterResponse> = post_authority_json_optional(
        &format!(
            "{}/api/core/v1/brain/account-agent-roster",
            authorities.core_base_url
        ),
        "Authorization",
        &format!("Bearer {}", authorities.core_token),
        &serde_json::json!({ "email": email }),
        "Finite Core account agent roster",
    )
    .await?;

    let mut human_npub = None;
    let mut agents = Vec::new();
    let mut exclusions = Vec::new();
    let (workos_user_id, roster_revision) = if let Some(roster) = roster {
        if canonical_email(&roster.human_mailbox)? != email {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "Finite Core returned a mismatched account mailbox",
            ));
        }
        if roster.workos_user_id.trim().is_empty() {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "Finite Core returned an empty account id",
            ));
        }
        let owner: IdentityUserResolutionResponse = post_authority_json(
            &format!(
                "{}/api/v1/operator/brain/user-resolution",
                authorities.identity_base_url
            ),
            "X-Finite-Operator-Token",
            &authorities.identity_token,
            &serde_json::json!({ "workosUserId": roster.workos_user_id }),
            "Finite Identity User Nostr Identity resolution",
        )
        .await?;
        if owner.workos_user_id != roster.workos_user_id {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "Finite Identity returned a mismatched WorkOS account",
            ));
        }
        human_npub = Some(owner.user_npub);
        for agent in &roster.agents {
            match resolve_roster_agent(state, agent).await? {
                Ok(resolved) => agents.push(resolved),
                Err(exclusion) => exclusions.push(exclusion),
            }
        }
        (Some(roster.workos_user_id), Some(roster.roster_revision))
    } else {
        if finite_vip_email(email) {
            match resolve_and_record_identity(state, email).await {
                Ok(identity) => human_npub = Some(identity.npub),
                Err(error) if error.status == StatusCode::NOT_FOUND => {
                    exclusions.push(InvitationPlanExclusion {
                        ref_: email.to_owned(),
                        reason: "email does not resolve to a Finite account or identity".to_owned(),
                    });
                }
                Err(error) => return Err(error),
            }
        } else {
            exclusions.push(InvitationPlanExclusion {
                ref_: email.to_owned(),
                reason: "email does not bind to a Finite account".to_owned(),
            });
        }
        (None, None)
    };

    Ok(PlanResolution {
        workos_user_id,
        human_email: email.to_owned(),
        human_npub,
        agents,
        exclusions,
        roster_revision,
    })
}

/// Resolve one roster agent to its Agent Principal npub and verify the binding
/// through Finite Identity. Any failure is an explicit exclusion with a reason.
async fn resolve_roster_agent(
    state: &ServerState,
    agent: &CoreRosterAgentEntry,
) -> Result<Result<InvitationPlanAgent, InvitationPlanExclusion>, ApiError> {
    let exclusion = |reason: &str| {
        Ok(Err(InvitationPlanExclusion {
            ref_: agent.managed_agent_email.clone(),
            reason: reason.to_owned(),
        }))
    };
    let Ok(managed_agent_email) = canonical_email(&agent.managed_agent_email) else {
        return exclusion("account roster returned an invalid Managed Agent email");
    };
    if agent.status != "active" {
        return exclusion("agent is not active in the account roster");
    }
    let agent_npub = if let Some(agent_npub) = agent
        .agent_npub
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(agent_npub.to_owned())
    } else {
        match resolve_identity_input(state, &managed_agent_email).await {
            Ok(identity) => Some(identity.npub),
            Err(_) => None,
        }
    };
    let Some(agent_npub) = agent_npub else {
        return exclusion("agent npub is not resolvable");
    };
    let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Brain account-agent authority is not configured",
        )
    })?;
    let binding: Option<IdentityAgentResolutionResponse> = post_authority_json_optional(
        &format!(
            "{}/api/v1/operator/brain/agent-resolution",
            authorities.identity_base_url
        ),
        "X-Finite-Operator-Token",
        &authorities.identity_token,
        &serde_json::json!({ "agentNpub": agent_npub }),
        "Finite Identity Managed Agent resolution",
    )
    .await?;
    let Some(binding) = binding else {
        return exclusion("Finite Identity has no Managed Agent binding for the resolved npub");
    };
    if binding.agent_npub != agent_npub
        || canonical_email(&binding.managed_agent_email).ok() != Some(managed_agent_email.clone())
    {
        return exclusion("Finite Identity Managed Agent binding does not match the roster agent");
    }
    Ok(Ok(InvitationPlanAgent {
        managed_agent_email,
        agent_npub: Some(agent_npub),
        status: agent.status.clone(),
    }))
}

/// Hash the full resolved set plus roster revision so commit cannot drift
/// from the previewed plan.
fn invitation_plan_hash(
    brain_id: &BrainId,
    inviter_npub: &UserId,
    human_email: &str,
    human_npub: Option<&str>,
    agents: &[(String, Option<String>, String)],
    exclusions: &[(String, String)],
    roster_revision: Option<i64>,
) -> String {
    let mut sorted_agents = agents.to_vec();
    sorted_agents.sort_by_key(|(email, _, _)| email.clone());
    let agent_parts = sorted_agents
        .iter()
        .map(|(email, npub, status)| {
            serde_json::json!([email, npub.as_deref().unwrap_or(""), status])
        })
        .collect::<Vec<_>>();
    let mut sorted_exclusions = exclusions.to_vec();
    sorted_exclusions.sort_by_key(|(ref_, _)| ref_.clone());
    let exclusion_parts = sorted_exclusions
        .iter()
        .map(|(ref_, reason)| serde_json::json!([ref_, reason]))
        .collect::<Vec<_>>();
    let input = serde_json::json!({
        "brainId": brain_id.as_str(),
        "inviterNpub": inviter_npub.as_str(),
        "human": { "email": human_email, "npub": human_npub },
        "agents": agent_parts,
        "exclusions": exclusion_parts,
        "rosterRevision": roster_revision,
    });
    let mut hasher = Sha256::new();
    hasher.update(input.to_string().as_bytes());
    format!("sha256:{}", hex_prefix(&hasher.finalize(), 32))
}

fn resolution_plan_hash(
    brain_id: &BrainId,
    inviter_npub: &UserId,
    resolution: &PlanResolution,
) -> String {
    let agents = resolution
        .agents
        .iter()
        .map(|agent| {
            (
                agent.managed_agent_email.clone(),
                agent.agent_npub.clone(),
                agent.status.clone(),
            )
        })
        .collect::<Vec<_>>();
    let exclusions = resolution
        .exclusions
        .iter()
        .map(|exclusion| (exclusion.ref_.clone(), exclusion.reason.clone()))
        .collect::<Vec<_>>();
    invitation_plan_hash(
        brain_id,
        inviter_npub,
        &resolution.human_email,
        resolution.human_npub.as_deref(),
        &agents,
        &exclusions,
        resolution.roster_revision,
    )
}

fn stored_plan_hash(plan: &StoredInvitationPlan) -> String {
    let agents = plan
        .agents
        .iter()
        .map(|agent| {
            (
                agent.managed_agent_email.clone(),
                agent.agent_npub.clone(),
                agent.status.clone(),
            )
        })
        .collect::<Vec<_>>();
    let exclusions = plan
        .exclusions
        .iter()
        .map(|exclusion| (exclusion.ref_.clone(), exclusion.reason.clone()))
        .collect::<Vec<_>>();
    invitation_plan_hash(
        &plan.brain_id,
        &plan.inviter_npub,
        &plan.human_email,
        plan.human_npub.as_ref().map(UserId::as_str),
        &agents,
        &exclusions,
        plan.roster_revision,
    )
}

fn timestamp_plus_seconds(state: &ServerState, seconds: u64) -> String {
    format_unix_timestamp(state.auth_now_unix_seconds() + seconds)
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned())
}

pub(crate) fn persist_invitation_plan(
    state: &ServerState,
    brain_id: &BrainId,
    inviter_npub: &UserId,
    resolution: PlanResolution,
) -> Result<StoredInvitationPlan, ApiError> {
    // Plan ids are deterministic over (brain, inviter, mailbox, hash,
    // folder, second): two preflights inside one second collide on the
    // unique constraint. Retry with a salt instead of surfacing a 409.
    let mut last_error = None;
    for attempt in 0..3_u8 {
        let salt = (attempt > 0).then(|| format!("preflight-{attempt}"));
        match persist_invitation_plan_with_salt(
            state,
            brain_id,
            inviter_npub,
            resolution.clone(),
            None,
            salt.as_deref(),
        ) {
            Ok(plan) => return Ok(plan),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
    }))
}

/// `unique_salt` separates otherwise identical plans created within the same
/// second (deterministic ids collide there); approval-request filing retries
/// with an attempt suffix, mirroring the approval-request id retry.
pub(crate) fn persist_invitation_plan_with_salt(
    state: &ServerState,
    brain_id: &BrainId,
    inviter_npub: &UserId,
    resolution: PlanResolution,
    folder_scope: Option<&FolderId>,
    unique_salt: Option<&str>,
) -> Result<StoredInvitationPlan, ApiError> {
    let created_at = server_timestamp(state);
    let expires_at = timestamp_plus_seconds(state, PLAN_COMMIT_WINDOW_SECONDS);
    let mut plan_hash = resolution_plan_hash(brain_id, inviter_npub, &resolution);
    let mut id_parts = [
        brain_id.as_str(),
        inviter_npub.as_str(),
        resolution.human_email.as_str(),
        created_at.as_str(),
    ]
    .to_vec();
    if let Some(folder_id) = folder_scope {
        // Folder scoping must change both the hash (a Folder plan and a
        // membership plan for one mailbox never share a commit identity) and
        // the derived id.
        plan_hash = generated_link_id(
            "plan-folder",
            &[brain_id.as_str(), folder_id.as_str(), plan_hash.as_str()],
            32,
        );
        id_parts.insert(3, folder_id.as_str());
    }
    id_parts.insert(3, plan_hash.as_str());
    if let Some(salt) = unique_salt {
        id_parts.push(salt);
    }
    let id = generated_link_id("plan", &id_parts, 16);
    let human_npub = resolution
        .human_npub
        .as_deref()
        .map(UserId::new)
        .transpose()?;
    let plan = StoredInvitationPlan {
        id,
        brain_id: brain_id.clone(),
        plan_hash,
        inviter_npub: inviter_npub.clone(),
        workos_user_id: resolution.workos_user_id,
        human_email: resolution.human_email,
        human_npub,
        agents: resolution
            .agents
            .into_iter()
            .map(|agent| StoredPlanAgent {
                managed_agent_email: agent.managed_agent_email,
                agent_npub: agent.agent_npub,
                status: agent.status,
            })
            .collect(),
        exclusions: resolution
            .exclusions
            .into_iter()
            .map(|exclusion| StoredPlanExclusion {
                ref_: exclusion.ref_,
                reason: exclusion.reason,
            })
            .collect(),
        roster_revision: resolution.roster_revision,
        folder_id: folder_scope.cloned(),
        committed: false,
        expires_at,
        created_at: created_at.clone(),
        updated_at: created_at,
    };
    let mut store = state.store.lock().map_err(lock_error)?;
    Ok(store.create_brain_invitation_plan(&plan)?)
}

pub(crate) fn preflight_response(
    plan: StoredInvitationPlan,
    supersedes_plan_id: Option<String>,
) -> InvitationPreflightResponse {
    InvitationPreflightResponse {
        plan_id: plan.id,
        plan_hash: plan.plan_hash,
        human: InvitationPlanHuman {
            email: plan.human_email,
            npub: plan.human_npub.map(|npub| npub.to_string()),
        },
        agents: plan
            .agents
            .into_iter()
            .map(|agent| InvitationPlanAgent {
                managed_agent_email: agent.managed_agent_email,
                agent_npub: agent.agent_npub,
                status: agent.status,
            })
            .collect(),
        roster_revision: plan.roster_revision,
        exclusions: plan
            .exclusions
            .into_iter()
            .map(|exclusion| InvitationPlanExclusion {
                ref_: exclusion.ref_,
                reason: exclusion.reason,
            })
            .collect(),
        expires_at: plan.expires_at,
        supersedes_plan_id,
    }
}

pub(crate) async fn preflight_brain_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<InvitationPreflightResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: InvitationPreflightRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let actor_user_id = UserId::new(actor)?;
    let email = canonical_email(&request.target)?;
    {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, actor_user_id.as_str())?;
    }
    let resolution = resolve_invitation_plan(&state, &email).await?;
    let plan = persist_invitation_plan(&state, &brain_id, &actor_user_id, resolution)?;
    Ok(Json(preflight_response(plan, None)))
}

pub(crate) async fn commit_brain_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: InvitationCommitRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let actor_user_id = UserId::new(actor)?;
    let plan = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, actor_user_id.as_str())?;
        store
            .load_brain_invitation_plan(&request.plan_id)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "invitation plan not found"))?
    };
    if plan.brain_id != brain_id {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "invitation plan not found",
        ));
    }
    if plan.inviter_npub != actor_user_id {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "only the inviting admin can commit this plan",
        ));
    }
    if plan.committed {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "invitation plan is already committed",
        ));
    }
    if request.plan_hash != plan.plan_hash || stored_plan_hash(&plan) != plan.plan_hash {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "plan hash does not match the resolved set",
        ));
    }
    if plan.expires_at <= server_timestamp(&state) {
        return Err(ApiError::new(
            StatusCode::GONE,
            "invitation plan has expired; run preflight again",
        ));
    }

    match execute_invitation_plan_commit(
        &state,
        &brain_id,
        &actor_user_id,
        &plan,
        request.reduced_set,
        PlanCommitOrigin::Direct,
        None,
    )
    .await?
    {
        PlanCommitResult::Committed(response) => Ok(Json(response).into_response()),
        PlanCommitResult::Drifted(preflight) => {
            Ok((StatusCode::CONFLICT, Json(preflight)).into_response())
        }
    }
}

/// How one plan commit was authorized: directly by the inviting admin, or by
/// a signed Approval artifact (ADR-0046).
pub(crate) enum PlanCommitOrigin {
    Direct,
    Approval { approval_event_id: String },
}

/// Approval artifact bookkeeping applied inside the commit critical section.
pub(crate) struct ApprovalExecutionContext {
    pub nonce: String,
    pub approval_event_id: String,
    pub signer_npub: UserId,
    pub request_id: Option<String>,
}

/// Plan commit outcome shared by the direct route and the Approval route.
pub(crate) enum PlanCommitResult {
    Committed(InvitationCommitResponse),
    /// The roster drifted; carry the fresh preflight persisted for retry.
    Drifted(InvitationPreflightResponse),
}

/// Shared plan commit execution: re-verify the roster, write one npub-bound
/// invitation per included principal, and mark the plan committed. The caller
/// performs plan existence and shape checks first. When an Approval execution
/// context is present, the nonce replay guard, nonce record, and request
/// resolution run inside the same store critical section as the commit.
pub(crate) async fn execute_invitation_plan_commit(
    state: &ServerState,
    brain_id: &BrainId,
    committer_npub: &UserId,
    plan: &StoredInvitationPlan,
    reduced_set: Option<Vec<String>>,
    origin: PlanCommitOrigin,
    approval: Option<ApprovalExecutionContext>,
) -> Result<PlanCommitResult, ApiError> {
    // Re-verify the roster: on revision drift, refuse to commit the stale plan
    // and answer with a fresh preflight instead.
    if let Some(workos_user_id) = plan.workos_user_id.as_deref() {
        let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Brain account-agent authority is not configured",
            )
        })?;
        let roster: Option<CoreAccountAgentRosterResponse> = post_authority_json_optional(
            &format!(
                "{}/api/core/v1/brain/account-agent-roster",
                authorities.core_base_url
            ),
            "Authorization",
            &format!("Bearer {}", authorities.core_token),
            &serde_json::json!({ "workosUserId": workos_user_id }),
            "Finite Core account agent roster",
        )
        .await?;
        let drifted = match roster {
            None => true,
            Some(roster) => Some(roster.roster_revision) != plan.roster_revision,
        };
        if drifted {
            let resolution = resolve_invitation_plan(state, &plan.human_email).await?;
            let fresh = persist_invitation_plan(state, brain_id, &plan.inviter_npub, resolution)?;
            return Ok(PlanCommitResult::Drifted(preflight_response(
                fresh,
                Some(plan.id.clone()),
            )));
        }
    }

    let mut reduced = BTreeSet::new();
    for email in reduced_set.unwrap_or_default() {
        let email = canonical_email(&email)?;
        if !plan
            .agents
            .iter()
            .any(|agent| agent.managed_agent_email == email)
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "reducedSet contains a participant outside the plan",
            ));
        }
        reduced.insert(email);
    }

    let mut included: Vec<(String, String)> = Vec::new();
    if let Some(human_npub) = plan.human_npub.as_ref() {
        included.push((plan.human_email.clone(), human_npub.as_str().to_owned()));
    }
    for agent in &plan.agents {
        if reduced.contains(&agent.managed_agent_email) {
            continue;
        }
        if let Some(agent_npub) = agent.agent_npub.as_ref() {
            included.push((agent.managed_agent_email.clone(), agent_npub.clone()));
        }
    }
    if included.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "plan has no invitable principals",
        ));
    }

    let (origin_kind, origin_ref) = match &origin {
        PlanCommitOrigin::Direct => (ProvenanceOriginKind::Invitation, plan.id.clone()),
        PlanCommitOrigin::Approval { approval_event_id } => {
            (ProvenanceOriginKind::Approval, approval_event_id.clone())
        }
    };
    let created_at = server_timestamp(state);
    let invitation_expires_at = timestamp_plus_seconds(state, COMMIT_INVITATION_EXPIRY_SECONDS);
    let mut invitations = Vec::new();
    let mut skipped = Vec::new();
    let mut superseded_invitation_ids = Vec::new();
    {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(brain_id)?;
        ensure_brain_admin(&stored, committer_npub.as_str())?;
        if let Some(approval) = approval.as_ref()
            && store.approval_nonce_seen(brain_id, &approval.nonce)?
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "approval nonce was already applied",
            ));
        }
        for (index, (ref_, npub)) in included.iter().enumerate() {
            let target = UserId::new(npub.clone())?;
            if store.member_exists(brain_id, &target)? {
                skipped.push(CommitSkippedPrincipal {
                    ref_: ref_.clone(),
                    reason: "already a brain member".to_owned(),
                });
                continue;
            }
            // Re-invite after expiry supersedes the stale delivery handle
            // instead of colliding on the pending (Brain, target) singleton.
            superseded_invitation_ids.extend(store.revoke_expired_pending_brain_invitations(
                brain_id,
                &target,
                committer_npub,
                &created_at,
            )?);
            let index = index.to_string();
            let id = generated_link_id(
                "invitation",
                &[
                    brain_id.as_str(),
                    plan.id.as_str(),
                    npub,
                    index.as_str(),
                    created_at.as_str(),
                ],
                16,
            );
            let invite_code = generated_link_id(
                "invite",
                &[
                    brain_id.as_str(),
                    plan.id.as_str(),
                    npub,
                    index.as_str(),
                    created_at.as_str(),
                    "code",
                ],
                16,
            );
            let accept_path = format!("/v1/brain-invitation-links/{invite_code}/accept");
            let invitation = store.create_brain_invitation_with_provenance(
                brain_id,
                &id,
                &target,
                &invite_code,
                &accept_path,
                &[],
                committer_npub,
                &invitation_expires_at,
                &created_at,
                Some(origin_ref.as_str()),
                plan.roster_revision,
                origin_kind,
            )?;
            let mut response = brain_invitation_response(invitation);
            attach_invitation_public_url(state, &mut response);
            invitations.push(CommittedPrincipalInvitation {
                ref_: ref_.clone(),
                npub: npub.clone(),
                invitation: response,
            });
        }
        store.mark_brain_invitation_plan_committed(&plan.id, &created_at)?;
        if let Some(approval) = approval.as_ref() {
            store.record_brain_approval_nonce(
                brain_id,
                &approval.nonce,
                &approval.approval_event_id,
                &approval.signer_npub,
                finite_brain_core::BRAIN_APPROVAL_ACTION_INVITE_COMMIT,
                &created_at,
            )?;
            if let Some(request_id) = approval.request_id.as_deref() {
                store.resolve_brain_approval_request(
                    request_id,
                    finite_brain_store::ApprovalRequestStatus::Approved,
                    Some(approval.approval_event_id.as_str()),
                    &approval.signer_npub,
                    &created_at,
                )?;
            }
        }
    }

    Ok(PlanCommitResult::Committed(InvitationCommitResponse {
        status: "committed".to_owned(),
        plan_id: plan.id.clone(),
        roster_revision: plan.roster_revision,
        invitations,
        skipped,
        superseded_invitation_ids,
    }))
}

/// Re-check the account roster when a plan-linked invitation is accepted or
/// claimed. Permanently departed participants are excluded with an explicit
/// narrowed result; acceptance never adds and never substitutes participants.
pub(crate) async fn check_invitation_acceptance_narrowing(
    state: &ServerState,
    invitation: &StoredBrainInvitation,
    actor: &UserId,
) -> Result<Option<NarrowedAcceptanceResponse>, ApiError> {
    // Approval-committed invitations carry the approval event id as their
    // origin ref; the stored Approval request links that event back to the
    // plan it authorized.
    let plan = {
        let store = state.store.lock().map_err(lock_error)?;
        match invitation.origin_kind {
            ProvenanceOriginKind::Approval => {
                let Some(event_id) = invitation.origin_ref.as_deref() else {
                    return Ok(None);
                };
                let request = store
                    .load_brain_approval_request_by_event_id(&invitation.brain_id, event_id)?;
                let Some(request) = request else {
                    return Ok(None);
                };
                let payload: UnsignedBrainApprovalPayload =
                    serde_json::from_str(&request.payload_json).map_err(|_| {
                        ApiError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "stored approval request payload is corrupt",
                        )
                    })?;
                let Some(plan_id) = payload.plan_id.as_deref() else {
                    return Ok(None);
                };
                store.load_brain_invitation_plan(plan_id)?
            }
            _ => {
                let Some(plan_id) = invitation.origin_ref.as_deref() else {
                    return Ok(None);
                };
                store.load_brain_invitation_plan(plan_id)?
            }
        }
    };
    let Some(plan) = plan else {
        return Ok(None);
    };
    let is_human = plan.human_npub.as_ref() == Some(actor);
    let actor_agent = plan
        .agents
        .iter()
        .find(|agent| agent.agent_npub.as_deref() == Some(actor.as_str()));
    if !is_human && actor_agent.is_none() {
        return Ok(None);
    }
    let Some(workos_user_id) = plan.workos_user_id.as_deref() else {
        return Ok(None);
    };
    let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Brain account-agent authority is not configured",
        )
    })?;
    let roster: Option<CoreAccountAgentRosterResponse> = post_authority_json_optional(
        &format!(
            "{}/api/core/v1/brain/account-agent-roster",
            authorities.core_base_url
        ),
        "Authorization",
        &format!("Bearer {}", authorities.core_token),
        &serde_json::json!({ "workosUserId": workos_user_id }),
        "Finite Core account agent roster",
    )
    .await?;
    let active_agent_emails = roster
        .as_ref()
        .map(|roster| {
            roster
                .agents
                .iter()
                .filter(|agent| agent.status == "active")
                .filter_map(|agent| canonical_email(&agent.managed_agent_email).ok())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let departed = plan
        .agents
        .iter()
        .filter(|agent| !active_agent_emails.contains(&agent.managed_agent_email))
        .map(|agent| InvitationPlanExclusion {
            ref_: agent.managed_agent_email.clone(),
            reason: "permanently departed the account roster".to_owned(),
        })
        .collect::<Vec<_>>();
    if is_human && roster.is_none() {
        return Err(ApiError::new(
            StatusCode::GONE,
            "invited account has permanently departed the account roster",
        ));
    }
    if let Some(agent) = actor_agent
        && !active_agent_emails.contains(&agent.managed_agent_email)
    {
        return Err(ApiError::new(
            StatusCode::GONE,
            "invited agent has permanently departed the account roster",
        ));
    }
    let current_revision = roster.as_ref().map(|roster| roster.roster_revision);
    if current_revision == plan.roster_revision && departed.is_empty() {
        return Ok(None);
    }
    Ok(Some(NarrowedAcceptanceResponse {
        roster_revision: current_revision.or(plan.roster_revision),
        exclusions: departed,
    }))
}
