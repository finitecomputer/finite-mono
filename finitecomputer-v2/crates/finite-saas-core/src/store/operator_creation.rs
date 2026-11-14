//! Operator-created Agents (migration 0047). One transaction resolves or
//! mints the owner's pending account and personal org, checks the owner's
//! provisioned sign-in identity, records an explicit operator admission, and
//! writes the same Project, creation request and owner membership rows
//! self-serve creation writes. The lease, launch, Agent Principal and chat
//! identity then follow Core's normal path.

use super::*;
use crate::owner_identity::OwnerIdentityPlan;
use crate::{
    MAX_OPERATOR_CREATION_REASON_CHARS, MAX_OPERATOR_IDEMPOTENCY_KEY_CHARS,
    OPERATOR_REQUEST_KEY_PREFIX, OperatorAgentCreation, OperatorAgentCreationListing,
    OperatorCreateAgentInput, OperatorCreateAgentResult, OperatorCreationActor,
    OperatorCreationAdmission, OperatorCreationOwner, OperatorIdentity, OwnerSignInIdentity,
    UserLinkStatus, generate_surrogate_id,
};

mod guards;
mod records;
use guards::*;
use records::*;

/// What a rollback-only preview records in place of an identity it did not
/// provision. Never committed: only a dry-run store accepts it.
const PREVIEW_WORKOS_USER_ID: &str = "preview-not-provisioned";
const PREVIEW_CHAT_ACCOUNT_ID: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

impl CoreStore {
    pub fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    /// Read-only: how the operator path gets the owner's sign-in identity.
    /// Runs every refusal the transaction would make from rows that exist
    /// now, in the same order, so nothing is created at WorkOS or the hosted
    /// device for a request Core refuses anyway. The transaction repeats
    /// each check under its locks for races.
    pub async fn operator_owner_identity_plan(
        &self,
        input: &OperatorCreateAgentInput,
    ) -> CoreResult<OwnerIdentityPlan> {
        let wanted = normalize(input)?;
        let client = self.connection().await?;
        linked_admin_user_id(&**client, &input.admin).await?;
        if let Some(existing) = select_record_by_key(&**client, &wanted.idempotency_key).await? {
            return Ok(OwnerIdentityPlan::Reuse(OwnerSignInIdentity {
                workos_user_id: existing.owner.workos_user_id,
                chat_account_id: existing.owner.chat_account_id,
            }));
        }
        refuse_ambiguous_owner_email(&**client, &wanted.owner_email).await?;
        let Some(owner) = select_user_by_email(&**client, &wanted.owner_email).await? else {
            return Ok(OwnerIdentityPlan::Unlinked);
        };
        if owner.status == UserLinkStatus::Linked {
            refuse_unconfirmed_existing_account(input)?;
        }
        if let Some(org) = select_personal_org_by_owner(&**client, &owner.id).await? {
            refuse_trial_org(&**client, &org).await?;
        }
        refuse_inactive_private_grant(&**client, &owner).await?;
        if owner.status != UserLinkStatus::Linked {
            return Ok(OwnerIdentityPlan::Unlinked);
        }
        Ok(OwnerIdentityPlan::Linked {
            workos_user_id: owner
                .workos_user_id
                .ok_or_else(|| CoreError::Store("linked user has no WorkOS user".into()))?,
        })
    }

    /// Create an ordinary Agent owned by the account named by
    /// `input.owner_email`, minting a pending account when none exists.
    /// Retrying the same idempotency key with the same input returns the
    /// same creation; any difference, or an ambiguous owner, is refused.
    /// Callers resolve `input.owner_identity` first
    /// (`owner_identity::operator_create_agent`).
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
        let result = create_for_owner(&*tx, input, self.dry_run)
            .instrument(span)
            .await;
        if let Err(error) = &result {
            tracing::error!(operation = "admin_create_agent_for_owner", error = %error,
                "operator agent creation refused or failed");
        }
        self.finish(tx, result).await
    }

    /// Every operator creation, newest first, with the owner as they are now.
    pub async fn operator_agent_creations(&self) -> CoreResult<Vec<OperatorAgentCreationListing>> {
        let client = self.connection().await?;
        client
            .query(
                &format!(
                    "SELECT {RECORD_COLUMNS}, owner.normalized_email AS owner_current_email,
                            owner.link_status AS owner_link_status,
                            owner.workos_user_id AS owner_current_workos_user_id
                     FROM operator_agent_creations o
                     JOIN users owner ON owner.id = o.owner_user_id
                     ORDER BY o.created_at DESC, o.id DESC"
                ),
                &[],
            )
            .await
            .map_err(store_error)?
            .iter()
            .map(listing_from_row)
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
    // Refuse rather than cut: two long keys sharing a prefix are two keys.
    if input.idempotency_key.trim().chars().count() > MAX_OPERATOR_IDEMPOTENCY_KEY_CHARS {
        return Err(CoreError::InvalidOperatorIdempotencyKey);
    }
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
        profile_picture_url: normalize_profile_picture_url(input.profile_picture_url.as_deref())?,
    })
}

/// The owner identity this creation binds the Agent to. Open admission is
/// never allowed: without one, only a rollback-only preview proceeds.
fn owner_identity(
    input: &OperatorCreateAgentInput,
    dry_run: bool,
) -> CoreResult<Option<OwnerSignInIdentity>> {
    let Some(identity) = &input.owner_identity else {
        return if dry_run {
            Ok(None)
        } else {
            Err(CoreError::OperatorOwnerIdentityRequired)
        };
    };
    let workos_user_id = identity.workos_user_id.trim();
    let chat_account_id = normalize_owner_chat_account_id(Some(&identity.chat_account_id))?;
    match chat_account_id {
        Some(chat_account_id) if !workos_user_id.is_empty() => Ok(Some(OwnerSignInIdentity {
            workos_user_id: workos_user_id.to_string(),
            chat_account_id,
        })),
        _ => Err(CoreError::OperatorOwnerIdentityRequired),
    }
}

async fn create_for_owner<C>(
    client: &C,
    input: OperatorCreateAgentInput,
    dry_run: bool,
) -> CoreResult<OperatorCreateAgentResult>
where
    C: GenericClient + Sync,
{
    let wanted = normalize(&input)?;
    let identity = owner_identity(&input, dry_run)?;
    let now = match input.now.clone() {
        Some(now) => now,
        None => current_time_iso()?,
    };
    let admin_user_id = linked_admin_user_id(client, &input.admin).await?;
    if let Some(existing) = select_record_by_key(client, &wanted.idempotency_key).await? {
        return reuse_existing(client, existing, &wanted, identity.as_ref(), &now).await;
    }

    refuse_ambiguous_owner_email(client, &wanted.owner_email).await?;
    let (owner, account_created) = upsert_pending_owner(client, &wanted.owner_email, &now).await?;
    if let Some(identity) = &identity {
        refuse_mismatched_owner_identity(client, &owner, identity).await?;
    }
    if owner.status == UserLinkStatus::Linked {
        refuse_unconfirmed_existing_account(&input)?;
    }
    // The upsert row-locks the org, serializing this with trial reservation
    // (which locks it too), so a trial and an operator Agent never meet.
    let (org, customer_org_created) = upsert_sponsored_org(client, &owner, &now).await?;
    refuse_trial_org(client, &org).await?;
    refuse_inactive_private_grant(client, &owner).await?;
    let request_key = format!("{OPERATOR_REQUEST_KEY_PREFIX}{}", wanted.idempotency_key);

    let provisioned = identity.is_some();
    let identity = identity.unwrap_or_else(|| OwnerSignInIdentity {
        workos_user_id: PREVIEW_WORKOS_USER_ID.to_string(),
        chat_account_id: PREVIEW_CHAT_ACCOUNT_ID.to_string(),
    });
    let project_id = new_self_service_project_id()?;
    let request_id = new_agent_creation_request_id()?;
    let record = OperatorAgentCreation {
        id: generate_surrogate_id("operator_creation")?,
        idempotency_key: wanted.idempotency_key.clone(),
        owner: OperatorCreationOwner {
            email: wanted.owner_email.clone(),
            user_id: owner.id.clone(),
            account_created,
            customer_org_id: org.id.clone(),
            customer_org_created,
            workos_user_id: identity.workos_user_id.clone(),
            chat_account_id: identity.chat_account_id.clone(),
            existing_account_confirmed: input.existing_account_confirmed,
        },
        project_id: project_id.clone(),
        agent_creation_request_id: request_id.clone(),
        display_name: wanted.display_name.clone(),
        hosting_tier: wanted.hosting_tier,
        placement: wanted.placement,
        target_source_host_id: wanted.target_source_host_id.clone(),
        admission: OperatorCreationAdmission::OperatorSponsored,
        reason: wanted.reason.clone(),
        actor: OperatorCreationActor {
            admin_user_id,
            admin: OperatorIdentity {
                email: normalize_owner_email(Some(&input.admin.email)).unwrap_or_default(),
                workos_user_id: input.admin.workos_user_id.trim().to_string(),
            },
            surface: input.surface,
            cli_os_user: input
                .cli_os_user
                .as_deref()
                .and_then(|user| trim_to_option(Some(user))),
        },
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
        let identity = provisioned.then_some(identity);
        return reuse_existing(client, existing, &wanted, identity.as_ref(), &now).await;
    }
    // This transaction owns the key now. A request already holding the
    // operator-prefixed key was written by self-serve before the prefix was
    // reserved: a clear conflict, never a unique-index error.
    if select_agent_creation_request_by_idempotency(client, &owner.id, &request_key)
        .await?
        .is_some()
    {
        return Err(CoreError::OperatorAgentCreationRefused(
            "the owner already has a creation request with this operator key",
        ));
    }

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
        idempotency_key: request_key,
        display_name: wanted.display_name,
        runner_class: wanted.placement.runner_class,
        hosting_tier: Some(wanted.hosting_tier),
        placement: Some(wanted.placement),
        desired_runtime_artifact_id: None,
        runtime_spec: None,
        target_source_host_id: wanted.target_source_host_id,
        relocation: None,
        profile_picture_url: wanted.profile_picture_url,
        // Becomes FINITECHAT_OWNER_NPUBS at lease: the Agent admits only it.
        owner_chat_account_id: Some(identity.chat_account_id),
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
        owner_user_id = %record.owner.user_id,
        account_created,
        "operator created an agent for an owner"
    );
    Ok(OperatorCreateAgentResult {
        record,
        project,
        request,
        reused: false,
        owner_linked: owner.status == UserLinkStatus::Linked,
        owner_identity_provisioned: provisioned,
    })
}

async fn reuse_existing<C>(
    client: &C,
    record: OperatorAgentCreation,
    wanted: &Normalized,
    identity: Option<&OwnerSignInIdentity>,
    now: &str,
) -> CoreResult<OperatorCreateAgentResult>
where
    C: GenericClient + Sync,
{
    let request = select_agent_creation_request_by_idempotency(
        client,
        &record.owner.user_id,
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
        owner_email: record.owner.email.clone(),
        display_name: record.display_name.clone(),
        reason: record.reason.clone(),
        idempotency_key: record.idempotency_key.clone(),
        hosting_tier: record.hosting_tier,
        placement: record.placement,
        target_source_host_id: record.target_source_host_id.clone(),
        profile_picture_url: request.profile_picture_url.clone(),
    };
    let same_identity = identity.is_none_or(|identity| {
        identity.workos_user_id == record.owner.workos_user_id
            && identity.chat_account_id == record.owner.chat_account_id
    });
    if &stored != wanted || !same_identity {
        return Err(CoreError::OperatorAgentCreationRefused(
            "idempotency key already names a different operator creation",
        ));
    }
    let owner = select_user_by_id(client, &record.owner.user_id)
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
        owner_identity_provisioned: true,
    })
}
