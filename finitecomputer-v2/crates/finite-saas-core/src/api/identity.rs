use super::*;

pub(super) async fn me(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<MeResponse>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    link_verified_identity(&state, &identity).await?;
    Ok(Json(me_response_for_identity(&state, &identity).await?))
}

pub(super) async fn dashboard_summary(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<DashboardSummaryResponse>, ApiError> {
    // This route deliberately performs the same fresh WorkOS verification as
    // every other owner route. It only removes duplicate verification caused
    // by the dashboard fanning one page render out across three Core requests.
    let identity = require_verified_identity(&state, &headers).await?;
    link_verified_identity(&state, &identity).await?;

    let billing_input = LinkVerifiedUserInput {
        verified_email: identity.email.clone(),
        workos_user_id: identity.workos_user_id.clone(),
        now: None,
    };
    let (me, billing, finite_private_usage) = tokio::try_join!(
        me_response_for_identity(&state, &identity),
        async {
            state
                .store
                .billing_overview(billing_input)
                .await
                .map_err(ApiError::from)
        },
        async {
            state
                .store
                .finite_private_usage_status_for_workos_user(&identity.workos_user_id, None)
                .await
                .map_err(ApiError::from)
        },
    )?;

    Ok(Json(DashboardSummaryResponse {
        me,
        billing,
        finite_private_usage,
    }))
}

pub(super) async fn link_verified_identity(
    state: &CoreApiState,
    identity: &VerifiedIdentity,
) -> Result<(), ApiError> {
    state
        .store
        .link_verified_user(LinkVerifiedUserInput {
            verified_email: identity.email.clone(),
            workos_user_id: identity.workos_user_id.clone(),
            now: None,
        })
        .await?;
    Ok(())
}

async fn me_response_for_identity(
    state: &CoreApiState,
    identity: &VerifiedIdentity,
) -> Result<MeResponse, ApiError> {
    let projects = state
        .store
        .visible_projects_for_workos_user(&identity.workos_user_id)
        .await?;
    let agent_creation_requests = state
        .store
        .agent_creation_requests_for_workos_user(&identity.workos_user_id)
        .await?;
    Ok(MeResponse {
        email: identity.email.clone(),
        workos_user_id: identity.workos_user_id.clone(),
        projects: public_visible_projects(projects),
        agent_creation_requests: agent_creation_requests
            .into_iter()
            .map(AgentCreationRequestSummary::from)
            .collect(),
    })
}

pub(super) async fn projects(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PublicVisibleProject>>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    Ok(Json(public_visible_projects(
        state
            .store
            .visible_projects_for_workos_user(&identity.workos_user_id)
            .await?,
    )))
}

#[derive(Debug)]
pub(super) struct VerifiedIdentity {
    pub(super) email: String,
    pub(super) workos_user_id: String,
    workos_organization_id: Option<String>,
}

pub(super) async fn require_verified_identity(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<VerifiedIdentity, ApiError> {
    if [
        WORKOS_USER_ID_HEADER,
        WORKOS_EMAIL_HEADER,
        WORKOS_EMAIL_VERIFIED_HEADER,
    ]
    .into_iter()
    .any(|name| headers.contains_key(name))
    {
        return Err(ApiError::unauthorized(
            "caller-supplied identity headers are not accepted",
        ));
    }

    let access_token =
        bearer_token(headers).ok_or_else(|| ApiError::unauthorized("sign in is required"))?;
    let session = state
        .auth
        .workos()
        .verify_access_token(&access_token)
        .await
        .map_err(|error| workos_api_error_at("access_token", error))?;
    let user = state
        .auth
        .workos()
        .verified_user(&session.subject)
        .await
        .map_err(|error| workos_api_error_at("user_lookup", error))?;
    let email = normalize_owner_email(Some(&user.email))
        .ok_or_else(|| ApiError::unauthorized("invalid account"))?;
    Ok(VerifiedIdentity {
        email,
        workos_user_id: session.subject,
        workos_organization_id: session.organization_id,
    })
}

/// Core-side operator authorization. The WorkOS organization claim is an
/// identity-provider predicate only and is never used as a Core Customer
/// Organization id.
pub(super) async fn require_admin_identity(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<VerifiedIdentity, ApiError> {
    let identity = require_verified_identity(state, headers).await?;
    if identity.workos_organization_id.as_deref() != Some(state.auth.workos().operator_org_id()) {
        return Err(ApiError::forbidden(
            "admin access is required for this endpoint",
        ));
    }
    Ok(identity)
}
