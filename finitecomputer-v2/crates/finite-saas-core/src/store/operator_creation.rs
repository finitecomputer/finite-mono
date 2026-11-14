//! Operator-created Agents (migration 0047). One transaction resolves or
//! mints the owner's pending account and personal org, records an explicit
//! operator admission, and writes the same Project, creation request and
//! owner membership rows self-serve creation writes. The lease, launch,
//! Agent Principal and chat identity then follow Core's normal path.

use super::*;
use crate::{
    MAX_OPERATOR_CREATION_REASON_CHARS, OperatorAgentCreation, OperatorCreateAgentInput,
    OperatorCreateAgentResult, OperatorCreationAdmission, OperatorIdentity, UserLinkStatus,
    generate_surrogate_id, parse_operator_creation_admission, parse_operator_creation_surface,
};

/// Request idempotency keys of operator creations carry this prefix, so a
/// later self-serve submission by the owner can never reuse one.
pub(crate) const OPERATOR_REQUEST_KEY_PREFIX: &str = "operator:";

const RECORD_COLUMNS: &str = "id, idempotency_key, owner_email, owner_user_id,
    owner_account_created, customer_org_id, customer_org_created, project_id,
    agent_creation_request_id, display_name, hosting_tier, placement_runner_class,
    runtime_resource_class, target_source_host_id, owner_chat_account_id, admission,
    reason, admin_user_id, admin_email, admin_workos_user_id, surface,
    core_rfc3339(created_at) AS created_at";

impl CoreStore {
    /// Create an ordinary Agent owned by the account named by
    /// `input.owner_email`, minting a pending account when none exists.
    /// Retrying the same idempotency key with the same input returns the
    /// same creation; any difference, or an ambiguous owner, is refused.
    pub async fn admin_create_agent_for_owner(
        &self,
        input: OperatorCreateAgentInput,
    ) -> CoreResult<OperatorCreateAgentResult> {
        let span = tracing::info_span!(
            "admin_create_agent_for_owner",
            operation = "admin_create_agent_for_owner",
            admin_workos_user_id = %input.admin.workos_user_id,
            surface = input.surface.as_str(),
        );
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = create_for_owner(&*tx, input).instrument(span).await;
        if let Err(error) = &result {
            tracing::error!(operation = "admin_create_agent_for_owner", error = %error,
                "operator agent creation refused or failed");
        }
        self.finish(tx, result).await
    }

    /// Every operator creation, newest first, for operators to read back.
    pub async fn operator_agent_creations(&self) -> CoreResult<Vec<OperatorAgentCreation>> {
        let client = self.connection().await?;
        client
            .query(
                &format!(
                    "SELECT {RECORD_COLUMNS} FROM operator_agent_creations
                     ORDER BY created_at DESC, id DESC"
                ),
                &[],
            )
            .await
            .map_err(store_error)?
            .iter()
            .map(record_from_row)
            .collect()
    }
}

/// The validated, normalized request. Retries compare exactly these values.
#[derive(Debug, PartialEq, Eq)]
struct Normalized {
    owner_email: String,
    display_name: String,
    reason: String,
    idempotency_key: String,
    hosting_tier: HostingTier,
    placement: RuntimePlacement,
    target_source_host_id: Option<String>,
    owner_chat_account_id: Option<String>,
    profile_picture_url: Option<String>,
}

fn normalize(input: &OperatorCreateAgentInput) -> CoreResult<Normalized> {
    let owner_email = normalize_owner_email(Some(&input.owner_email))
        .filter(|email| {
            email.len() <= 320
                && !email.chars().any(char::is_whitespace)
                && email.split('@').count() == 2
                && email.split('@').all(|part| !part.is_empty())
        })
        .ok_or(CoreError::InvalidOperatorOwnerEmail)?;
    let display_name =
        trim_to_option(Some(&input.display_name)).ok_or(CoreError::MissingAgentDisplayName)?;
    let reason = trim_to_option(Some(&input.reason))
        .filter(|reason| reason.chars().count() <= MAX_OPERATOR_CREATION_REASON_CHARS)
        .ok_or(CoreError::InvalidOperatorReason)?;
    let idempotency_key = normalize_idempotency_key(&input.idempotency_key)
        .ok_or(CoreError::MissingAgentCreationIdempotencyKey)?;
    let hosting_tier = input.hosting_tier.unwrap_or(HostingTier::Standard);
    let mut placement = input
        .placement_override
        .unwrap_or_else(|| RuntimePlacement::for_hosting_tier(hosting_tier));
    if let Some(resource_class) = input.runtime_resource_class {
        placement.runtime_resource_class = resource_class;
    }
    let target_source_host_id = input
        .target_source_host_id
        .as_deref()
        .map(normalize_source_host_id)
        .transpose()?;
    if target_source_host_id.is_some() && placement.runner_class != crate::RunnerClass::Kata {
        return Err(CoreError::OperatorAgentCreationRefused(
            "a target host is only valid for a Kata placement",
        ));
    }
    Ok(Normalized {
        owner_email,
        display_name,
        reason,
        idempotency_key,
        hosting_tier,
        placement,
        target_source_host_id,
        owner_chat_account_id: normalize_owner_chat_account_id(
            input.owner_chat_account_id.as_deref(),
        )?,
        profile_picture_url: normalize_profile_picture_url(input.profile_picture_url.as_deref())?,
    })
}

async fn create_for_owner<C>(
    client: &C,
    input: OperatorCreateAgentInput,
) -> CoreResult<OperatorCreateAgentResult>
where
    C: GenericClient + Sync,
{
    let wanted = normalize(&input)?;
    let now = match input.now.clone() {
        Some(now) => now,
        None => current_time_iso()?,
    };
    let admin_user_id = linked_admin_user_id(client, &input.admin).await?;
    if let Some(existing) = select_record_by_key(client, &wanted.idempotency_key).await? {
        return reuse_existing(client, existing, &wanted, &now).await;
    }

    refuse_ambiguous_owner_email(client, &wanted.owner_email).await?;
    let existing_user = select_user_by_email(client, &wanted.owner_email).await?;
    let owner_account_created = existing_user.is_none();
    let owner = match existing_user {
        Some(user) => user,
        None => ensure_pending_user_row(client, &wanted.owner_email, &now).await?,
    };
    let existing_org = select_personal_org_by_owner(client, &owner.id).await?;
    let customer_org_created = existing_org.is_none();
    let org = match existing_org {
        Some(org) => org,
        None => ensure_personal_org_row(client, &owner, BillingClass::Sponsored, &now).await?,
    };
    refuse_trial_org(client, &org).await?;

    let project_id = new_self_service_project_id()?;
    let request_id = new_agent_creation_request_id()?;
    let record = OperatorAgentCreation {
        id: generate_surrogate_id("operator_creation")?,
        idempotency_key: wanted.idempotency_key.clone(),
        owner_email: wanted.owner_email.clone(),
        owner_user_id: owner.id.clone(),
        owner_account_created,
        customer_org_id: org.id.clone(),
        customer_org_created,
        project_id: project_id.clone(),
        agent_creation_request_id: request_id.clone(),
        display_name: wanted.display_name.clone(),
        hosting_tier: wanted.hosting_tier,
        placement: wanted.placement,
        target_source_host_id: wanted.target_source_host_id.clone(),
        owner_chat_account_id: wanted.owner_chat_account_id.clone(),
        admission: OperatorCreationAdmission::OperatorSponsored,
        reason: wanted.reason.clone(),
        admin_user_id,
        admin: OperatorIdentity {
            email: normalize_owner_email(Some(&input.admin.email)).unwrap_or_default(),
            workos_user_id: input.admin.workos_user_id.trim().to_string(),
        },
        surface: input.surface,
        created_at: now.clone(),
    };
    // Claim the key before any non-idempotent write. A concurrent retry of
    // the same key waits here on the unique index until this transaction
    // ends, then reuses what it committed. Its project and request foreign
    // keys are checked at commit (migration 0047 defers them).
    if !claim_record(client, &record).await? {
        let existing = select_record_by_key(client, &wanted.idempotency_key)
            .await?
            .ok_or_else(|| CoreError::Store("operator creation claim lost its row".into()))?;
        return reuse_existing(client, existing, &wanted, &now).await;
    }
    grant_operator_entitlement(client, &org, wanted.hosting_tier, &now).await?;

    let project = Project {
        id: project_id.clone(),
        customer_org_id: org.id.clone(),
        owner_user_id: owner.id.clone(),
        display_name: wanted.display_name.clone(),
        agent_email: Some(canonical_agent_email(&wanted.display_name, &project_id)),
        import_candidate_id: None,
        hosting_tier: Some(wanted.hosting_tier),
        placement: Some(wanted.placement),
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    upsert_project_row(client, &project).await?;
    let request = AgentCreationRequest {
        id: request_id,
        customer_org_id: org.id,
        owner_user_id: owner.id.clone(),
        project_id: project_id.clone(),
        idempotency_key: format!("{OPERATOR_REQUEST_KEY_PREFIX}{}", wanted.idempotency_key),
        display_name: wanted.display_name,
        runner_class: wanted.placement.runner_class,
        hosting_tier: Some(wanted.hosting_tier),
        placement: Some(wanted.placement),
        desired_runtime_artifact_id: None,
        runtime_spec: None,
        target_source_host_id: wanted.target_source_host_id,
        relocation: None,
        profile_picture_url: wanted.profile_picture_url,
        owner_chat_account_id: wanted.owner_chat_account_id,
        status: AgentCreationRequestStatus::Requested,
        requested_launch_code: None,
        agent_runtime_id: None,
        runner_id: None,
        lease_token: None,
        lease_expires_at: None,
        failure_message: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    upsert_agent_creation_request_row(client, &request).await?;
    ensure_hosted_web_membership_row(client, &owner, &project_id, &now).await?;
    tracing::info!(
        operation = "admin_create_agent_for_owner",
        record_id = %record.id,
        project_id = %record.project_id,
        request_id = %record.agent_creation_request_id,
        owner_user_id = %record.owner_user_id,
        owner_account_created,
        "operator created an agent for an owner"
    );
    Ok(OperatorCreateAgentResult {
        record,
        project,
        request,
        reused: false,
        owner_linked: owner.status == UserLinkStatus::Linked,
    })
}

/// The admin must already be one linked Core account with exactly this
/// email and WorkOS user. Unlike the older admin paths this never creates
/// or relinks the admin's own account.
async fn linked_admin_user_id<C>(client: &C, admin: &OperatorIdentity) -> CoreResult<String>
where
    C: GenericClient + Sync,
{
    let email = normalize_owner_email(Some(&admin.email)).ok_or(CoreError::MissingVerifiedEmail)?;
    let workos_user_id = admin.workos_user_id.trim();
    if workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    client
        .query_opt(
            "SELECT id FROM users
             WHERE normalized_email = $1 AND workos_user_id = $2 AND link_status = 'linked'",
            &[&email, &workos_user_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| row.get("id"))
        .ok_or(CoreError::OperatorIdentityMismatch)
}

/// An email that an account email change moved away from, or one an
/// in-flight change involves, could name two people. An Agent's own Finite
/// identity is never a person. Refuse rather than pick one.
async fn refuse_ambiguous_owner_email<C>(client: &C, email: &str) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_one(
            "SELECT
               EXISTS (SELECT 1 FROM account_email_changes
                       WHERE (status = 'prepared' AND (old_email = $1 OR new_email = $1))
                          OR (status = 'completed' AND old_email = $1)) AS email_change,
               EXISTS (SELECT 1 FROM projects WHERE lower(agent_email) = $1) AS agent_identity",
            &[&email],
        )
        .await
        .map_err(store_error)?;
    if row.get::<_, bool>("email_change") {
        return Err(CoreError::OperatorAgentCreationRefused(
            "owner email is part of an account email change; resolve it before creating an agent",
        ));
    }
    if row.get::<_, bool>("agent_identity") {
        return Err(CoreError::OperatorAgentCreationRefused(
            "owner email is an Agent's own address, not a person's account",
        ));
    }
    Ok(())
}

/// An event-trial org's Agents stop when the trial ends unpaid. An operator
/// creation must not leave the person that surprise, so it is refused.
async fn refuse_trial_org<C>(client: &C, org: &CustomerOrganization) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    if org.billing_class != BillingClass::Standard {
        return Ok(());
    }
    let on_trial: bool = client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM trial_redemptions
                            WHERE customer_org_id = $1 AND state <> 'expired')",
            &[&org.id],
        )
        .await
        .map_err(store_error)?
        .get(0);
    if on_trial {
        return Err(CoreError::OperatorAgentCreationRefused(
            "owner's account is on an event trial whose end would stop the agent",
        ));
    }
    Ok(())
}

/// The explicit admission: one more Agent for this org, so the operator
/// Agent never uses up the person's own self-serve allowance. The Launch
/// Code and tier the org already holds are left as they are. Self-serve
/// creation treats a standard org with no entitlement row as allowed one
/// Agent once billing is active, so that first row starts at two.
async fn grant_operator_entitlement<C>(
    client: &C,
    org: &CustomerOrganization,
    hosting_tier: HostingTier,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let first_allowance: i32 = if org.billing_class == BillingClass::Standard {
        2
    } else {
        1
    };
    client
        .execute(
            "INSERT INTO agent_creation_entitlements
               (id, customer_org_id, hosting_tier, allowed_new_agent_runtimes, launch_code, created_at, updated_at)
             VALUES ($1, $2, $3, $4, NULL, $5::text::timestamptz, $5::text::timestamptz)
             ON CONFLICT (customer_org_id) DO UPDATE SET
               allowed_new_agent_runtimes = agent_creation_entitlements.allowed_new_agent_runtimes + 1,
               updated_at = EXCLUDED.updated_at",
            &[
                &agent_creation_entitlement_id_for(&org.id),
                &org.id,
                &hosting_tier.as_str(),
                &first_allowance,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn reuse_existing<C>(
    client: &C,
    record: OperatorAgentCreation,
    wanted: &Normalized,
    now: &str,
) -> CoreResult<OperatorCreateAgentResult>
where
    C: GenericClient + Sync,
{
    let request = select_agent_creation_request_by_idempotency(
        client,
        &record.owner_user_id,
        &format!("{OPERATOR_REQUEST_KEY_PREFIX}{}", record.idempotency_key),
    )
    .await?
    .filter(|request| request.id == record.agent_creation_request_id)
    .ok_or_else(|| {
        CoreError::Store(format!(
            "operator creation {} lost its creation request",
            record.id
        ))
    })?;
    let stored = Normalized {
        owner_email: record.owner_email.clone(),
        display_name: record.display_name.clone(),
        reason: record.reason.clone(),
        idempotency_key: record.idempotency_key.clone(),
        hosting_tier: record.hosting_tier,
        placement: record.placement,
        target_source_host_id: record.target_source_host_id.clone(),
        owner_chat_account_id: record.owner_chat_account_id.clone(),
        profile_picture_url: request.profile_picture_url.clone(),
    };
    if &stored != wanted {
        return Err(CoreError::OperatorAgentCreationRefused(
            "idempotency key already names a different operator creation",
        ));
    }
    let owner = select_user_by_id(client, &record.owner_user_id)
        .await?
        .ok_or_else(|| {
            CoreError::Store(format!("operator creation {} lost its owner", record.id))
        })?;
    let project = select_project(client, &record.project_id)
        .await?
        .ok_or_else(|| missing_request_project_error(&request))?;
    ensure_hosted_web_membership_row(client, &owner, &project.id, now).await?;
    Ok(OperatorCreateAgentResult {
        record,
        project,
        request,
        reused: true,
        owner_linked: owner.status == UserLinkStatus::Linked,
    })
}

async fn select_record_by_key<C>(client: &C, key: &str) -> CoreResult<Option<OperatorAgentCreation>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            &format!(
                "SELECT {RECORD_COLUMNS} FROM operator_agent_creations WHERE idempotency_key = $1"
            ),
            &[&key],
        )
        .await
        .map_err(store_error)?
        .map(|row| record_from_row(&row))
        .transpose()
}

/// Insert the record unless its idempotency key exists; true when inserted.
async fn claim_record<C>(client: &C, record: &OperatorAgentCreation) -> CoreResult<bool>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "INSERT INTO operator_agent_creations
               (id, idempotency_key, owner_email, owner_user_id, owner_account_created,
                customer_org_id, customer_org_created, project_id, agent_creation_request_id,
                display_name, hosting_tier, placement_runner_class, runtime_resource_class,
                target_source_host_id, owner_chat_account_id, admission, reason, admin_user_id,
                admin_email, admin_workos_user_id, surface, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                     $17, $18, $19, $20, $21, $22::text::timestamptz)
             ON CONFLICT (idempotency_key) DO NOTHING",
            &[
                &record.id,
                &record.idempotency_key,
                &record.owner_email,
                &record.owner_user_id,
                &record.owner_account_created,
                &record.customer_org_id,
                &record.customer_org_created,
                &record.project_id,
                &record.agent_creation_request_id,
                &record.display_name,
                &record.hosting_tier.as_str(),
                &record.placement.runner_class.as_str(),
                &record.placement.runtime_resource_class.as_str(),
                &record.target_source_host_id,
                &record.owner_chat_account_id,
                &record.admission.as_str(),
                &record.reason,
                &record.admin_user_id,
                &record.admin.email,
                &record.admin.workos_user_id,
                &record.surface.as_str(),
                &record.created_at,
            ],
        )
        .await
        .map_err(store_error)
        .map(|inserted| inserted == 1)
}

fn record_from_row(row: &Row) -> CoreResult<OperatorAgentCreation> {
    let invalid = |column: &str, value: &str| {
        CoreError::Store(format!("invalid operator creation {column} {value}"))
    };
    let text = |column: &str| -> String { row.get(column) };
    let tier = text("hosting_tier");
    let runner_class = text("placement_runner_class");
    let resource_class = text("runtime_resource_class");
    let admission = text("admission");
    let surface = text("surface");
    Ok(OperatorAgentCreation {
        id: text("id"),
        idempotency_key: text("idempotency_key"),
        owner_email: text("owner_email"),
        owner_user_id: text("owner_user_id"),
        owner_account_created: row.get("owner_account_created"),
        customer_org_id: text("customer_org_id"),
        customer_org_created: row.get("customer_org_created"),
        project_id: text("project_id"),
        agent_creation_request_id: text("agent_creation_request_id"),
        display_name: text("display_name"),
        hosting_tier: parse_hosting_tier(&tier).ok_or_else(|| invalid("tier", &tier))?,
        placement: RuntimePlacement {
            runner_class: parse_runner_class(&runner_class)
                .ok_or_else(|| invalid("runner class", &runner_class))?,
            runtime_resource_class: parse_runtime_resource_class(&resource_class)
                .ok_or_else(|| invalid("resource class", &resource_class))?,
        },
        target_source_host_id: row.get("target_source_host_id"),
        owner_chat_account_id: row.get("owner_chat_account_id"),
        admission: parse_operator_creation_admission(&admission)
            .ok_or_else(|| invalid("admission", &admission))?,
        reason: text("reason"),
        admin_user_id: text("admin_user_id"),
        admin: OperatorIdentity {
            email: text("admin_email"),
            workos_user_id: text("admin_workos_user_id"),
        },
        surface: parse_operator_creation_surface(&surface)
            .ok_or_else(|| invalid("surface", &surface))?,
        created_at: text("created_at"),
    })
}
