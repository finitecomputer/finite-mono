use super::*;

impl CoreStore {
    pub async fn request_agent_creation(
        &self,
        input: RequestAgentCreationInput,
    ) -> CoreResult<RequestAgentCreationResult> {
        self.request_agent_creation_configured(input, AgentCreationConfiguration::default())
            .await
    }

    pub async fn request_agent_creation_configured(
        &self,
        input: RequestAgentCreationInput,
        configuration: AgentCreationConfiguration,
    ) -> CoreResult<RequestAgentCreationResult> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_request_agent_creation(&*tx, input, configuration).await?;
        self.finish(tx).await?;
        Ok(result)
    }
}

/// Observability wrapper around the agent-creation mutation. This is the single
/// most incident-prone write path (it is the one that shipped broken for
/// standard billing while the server logged nothing), so it always runs inside
/// a span carrying `org_id`/`user_id`/`operation` and emits a structured error
/// log on failure. Full DB detail is logged in the `ApiError` conversion behind
/// a correlation id; here we anchor the failure to the org and user.
async fn postgres_request_agent_creation<C>(
    client: &C,
    input: RequestAgentCreationInput,
    configuration: AgentCreationConfiguration,
) -> CoreResult<RequestAgentCreationResult>
where
    C: GenericClient + Sync,
{
    // Best-effort identity for the span/log. Surrogate ids are no longer
    // derivable from the email, so we resolve the real ids by natural-key
    // lookup; failures here must not fail the request, so they just log "-".
    let user = match normalize_owner_email(Some(&input.verified_email)) {
        Some(email) => select_user_by_email(client, &email).await.ok().flatten(),
        None => None,
    };
    let user_id = user.as_ref().map(|user| user.id.clone());
    let org_id = match user_id.as_deref() {
        Some(user_id) => select_personal_org_by_owner(client, user_id)
            .await
            .ok()
            .flatten()
            .map(|org| org.id),
        None => None,
    };
    let span = tracing::info_span!(
        "request_agent_creation",
        operation = "request_agent_creation",
        user_id = user_id.as_deref().unwrap_or("-"),
        org_id = org_id.as_deref().unwrap_or("-"),
    );
    let result = postgres_request_agent_creation_inner(client, input, configuration)
        .instrument(span)
        .await;
    if let Err(error) = &result {
        tracing::error!(
            operation = "request_agent_creation",
            user_id = user_id.as_deref().unwrap_or("-"),
            org_id = org_id.as_deref().unwrap_or("-"),
            error = %error,
            "agent creation request failed"
        );
    }
    result
}

async fn postgres_request_agent_creation_inner<C>(
    client: &C,
    input: RequestAgentCreationInput,
    configuration: AgentCreationConfiguration,
) -> CoreResult<RequestAgentCreationResult>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let verified_email = normalize_owner_email(Some(&input.verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let workos_user_id = input.workos_user_id.trim().to_string();
    if workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let display_name =
        trim_to_option(Some(&input.display_name)).ok_or(CoreError::MissingAgentDisplayName)?;
    let idempotency_key = normalize_idempotency_key(&input.idempotency_key)
        .ok_or(CoreError::MissingAgentCreationIdempotencyKey)?;
    let profile_picture_url =
        normalize_profile_picture_url(configuration.profile_picture_url.as_deref())?;
    let owner_chat_account_id =
        normalize_owner_chat_account_id(configuration.owner_chat_account_id.as_deref())?;
    let launch_code = trim_to_option(Some(&input.launch_code));
    let billing_class = if launch_code.is_some() {
        BillingClass::Sponsored
    } else {
        BillingClass::Standard
    };
    // Gate on billing/launch against the EXISTING org (resolved by natural key),
    // before minting any rows. On the standard-billing path the org already
    // exists from checkout; a brand-new email has no org and thus no billing.
    let existing_org_id = match select_user_by_email(client, &verified_email).await? {
        Some(user) => select_personal_org_by_owner(client, &user.id)
            .await?
            .map(|org| org.id),
        None => None,
    };
    let locked_launch_code = if let Some(code) = launch_code.as_deref() {
        let locked = lock_postgres_launch_code(client, code, &now).await?;
        if let (Some(redeemed_org_id), Some(redeemed_key)) = (
            locked.record.redeemed_customer_org_id.as_deref(),
            locked.record.redemption_idempotency_key.as_deref(),
        ) {
            // A concurrent identical retry may have resolved the org while
            // this transaction waited on the code row lock. Re-read the
            // natural-key mapping after the lock before deciding whether the
            // already-bound redemption is the same account/request.
            let current_org_id = match select_user_by_email(client, &verified_email).await? {
                Some(user) => select_personal_org_by_owner(client, &user.id)
                    .await?
                    .map(|org| org.id),
                None => None,
            };
            if current_org_id.as_deref() != Some(redeemed_org_id) || idempotency_key != redeemed_key
            {
                return Err(CoreError::InvalidLaunchCode);
            }
        } else if locked.record.redeemed_customer_org_id.is_some()
            || locked.record.redemption_idempotency_key.is_some()
        {
            return Err(CoreError::InvalidLaunchCode);
        }
        Some(locked)
    } else if !match existing_org_id.as_deref() {
        Some(org_id) => billing::customer_org_has_active_billing(client, org_id).await?,
        None => false,
    } {
        return Err(CoreError::BillingRequired);
    } else {
        None
    };
    let hosting_tier = if let Some(locked) = locked_launch_code.as_ref() {
        locked.hosting_tier.unwrap_or(HostingTier::Standard)
    } else {
        let org_id = existing_org_id
            .as_deref()
            .ok_or(CoreError::MissingHostingTier)?;
        billing::select_customer_billing_account(client, org_id, false)
            .await?
            .and_then(|account| account.hosting_tier)
            .ok_or(CoreError::MissingHostingTier)?
    };
    if configuration
        .requested_hosting_tier
        .is_some_and(|requested| requested != hosting_tier)
    {
        return Err(CoreError::HostingTierNotAuthorized);
    }
    let placement = configuration
        .placement
        .unwrap_or_else(|| RuntimePlacement::for_hosting_tier(hosting_tier));
    if locked_launch_code
        .as_ref()
        .is_some_and(|code| code.target_source_host_id.is_some())
        && placement.runner_class != crate::RunnerClass::Kata
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    if client
        .query_opt(
            "SELECT id FROM users WHERE workos_user_id = $1 AND normalized_email <> $2",
            &[&workos_user_id, &verified_email],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::WorkosUserConflict);
    }

    let user = upsert_linked_user(client, &verified_email, &workos_user_id, &now).await?;
    let org = ensure_personal_org_row(client, &user, billing_class, &now).await?;

    // Dedupe via the UNIQUE(owner_user_id, idempotency_key): look up an existing
    // request, return it as reused; only mint fresh surrogate ids on a new one.
    if let Some(mut existing_request) =
        select_agent_creation_request_by_idempotency(client, &user.id, &idempotency_key).await?
    {
        if let Some(locked) = locked_launch_code.as_ref()
            && (locked.record.redeemed_customer_org_id.as_deref() != Some(org.id.as_str())
                || locked.record.redemption_idempotency_key.as_deref()
                    != Some(idempotency_key.as_str())
                || existing_request.requested_launch_code.as_deref()
                    != Some(locked.record.id.as_str()))
        {
            return Err(CoreError::InvalidLaunchCode);
        }
        // The reuse path must not silently drop a pre-mint it now carries: the
        // first attempt may have failed open (hosted device briefly
        // unavailable), and a retried attempt with the same idempotency key
        // brings the owner's hosted-chat account id. Backfill ONLY the empty
        // column — an existing value stays authoritative — and only while the
        // row is still `requested`: the predicate in the UPDATE itself is the
        // check, and it is the exact complement of the runner lease's
        // transition (`status = 'requested'` candidate ->
        // `SET status = 'launching'`, runner_id/lease_token/lease_expires_at),
        // which injects FINITECHAT_OWNER_NPUBS from this column at spec-build
        // time after taking the row lock. Row-level locking serializes the
        // two writers, so either the backfill commits first and the lease
        // reads the id, or the lease commits first, this UPDATE matches zero
        // rows, and the row is NOT mutated — the runtime spec was built
        // without the owner identity and the database must not claim
        // otherwise. On that loss the row is re-read so the returned request
        // reports what is actually persisted.
        if existing_request.owner_chat_account_id.is_none() && owner_chat_account_id.is_some() {
            let landed = client
                .query_opt(
                    "UPDATE agent_creation_requests
                     SET owner_chat_account_id = $2, updated_at = $3::text::timestamptz
                     WHERE id = $1
                       AND owner_chat_account_id IS NULL
                       AND status = 'requested'
                     RETURNING core_rfc3339(updated_at) AS updated_at",
                    &[&existing_request.id, &owner_chat_account_id, &now],
                )
                .await
                .map_err(store_error)?;
            match landed {
                Some(row) => {
                    existing_request.owner_chat_account_id = owner_chat_account_id;
                    existing_request.updated_at = row.get("updated_at");
                }
                None => {
                    existing_request = select_agent_creation_request_by_idempotency(
                        client,
                        &user.id,
                        &idempotency_key,
                    )
                    .await?
                    .unwrap_or(existing_request);
                }
            }
        }
        let project = select_project(client, &existing_request.project_id)
            .await?
            .ok_or_else(|| missing_request_project_error(&existing_request))?;
        ensure_hosted_web_membership_row(client, &user, &project.id, &now).await?;
        return Ok(RequestAgentCreationResult {
            project,
            request: existing_request,
            reused: true,
        });
    }

    let allowed_new_agent_runtimes = if let Some(locked) = locked_launch_code.as_ref() {
        if locked.record.redeemed_customer_org_id.is_none() {
            grant_launch_code_agent_creation_entitlement_row(
                client,
                &org.id,
                &locked.record.id,
                hosting_tier,
                &now,
            )
            .await?
            .allowed_new_agent_runtimes
        } else {
            select_agent_creation_entitlement_by_org(client, &org.id)
                .await?
                .map(|entitlement| entitlement.allowed_new_agent_runtimes)
                .unwrap_or(0)
        }
    } else {
        select_agent_creation_entitlement_by_org(client, &org.id)
            .await?
            .map(|entitlement| entitlement.allowed_new_agent_runtimes)
            .unwrap_or(1)
    };
    let active_request_count =
        postgres_active_agent_creation_entitlement_count(client, &org.id).await?;
    if active_request_count >= i64::from(allowed_new_agent_runtimes) {
        return Err(CoreError::AgentCreationEntitlementExhausted);
    }
    if let Some(locked) = locked_launch_code.as_ref() {
        if locked.record.redeemed_customer_org_id.is_none() {
            redeem_postgres_launch_code(client, &locked.record.id, &org.id, &idempotency_key, &now)
                .await?;
        }
    } else {
        ensure_standard_agent_creation_entitlement_row(client, &org.id, &now).await?;
    }

    let request_id = new_agent_creation_request_id()?;
    let project_id = new_self_service_project_id()?;
    let project = Project {
        id: project_id.clone(),
        customer_org_id: org.id.clone(),
        owner_user_id: user.id.clone(),
        display_name: display_name.clone(),
        agent_email: Some(canonical_agent_email(&display_name, &project_id)),
        import_candidate_id: None,
        hosting_tier: Some(hosting_tier),
        placement: Some(placement),
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    upsert_project_row(client, &project).await?;

    let request = AgentCreationRequest {
        id: request_id,
        customer_org_id: org.id,
        owner_user_id: user.id.clone(),
        project_id: project_id.clone(),
        idempotency_key,
        display_name,
        runner_class: placement.runner_class,
        hosting_tier: Some(hosting_tier),
        placement: Some(placement),
        desired_runtime_artifact_id: None,
        runtime_spec: None,
        target_source_host_id: locked_launch_code
            .as_ref()
            .and_then(|code| code.target_source_host_id.clone()),
        relocation: None,
        profile_picture_url,
        owner_chat_account_id,
        status: AgentCreationRequestStatus::Requested,
        requested_launch_code: locked_launch_code.map(|locked| locked.record.id),
        agent_runtime_id: None,
        runner_id: None,
        lease_token: None,
        lease_expires_at: None,
        failure_message: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    upsert_agent_creation_request_row(client, &request).await?;
    ensure_hosted_web_membership_row(client, &user, &project_id, &request.created_at).await?;

    Ok(RequestAgentCreationResult {
        project,
        request,
        reused: false,
    })
}

pub(super) fn missing_request_project_error(request: &AgentCreationRequest) -> CoreError {
    CoreError::Store(format!(
        "agent creation request {} references missing project {}",
        request.id, request.project_id
    ))
}
