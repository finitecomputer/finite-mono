//! Fail-closed checks and the owner rows of an operator creation.

use super::*;

/// The admin must already be one linked Core account with exactly this
/// email and WorkOS user. Unlike the older admin paths this never creates
/// or relinks the admin's own account.
pub(super) async fn linked_admin_user_id<C>(
    client: &C,
    admin: &OperatorIdentity,
) -> CoreResult<String>
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
pub(super) async fn refuse_ambiguous_owner_email<C>(client: &C, email: &str) -> CoreResult<()>
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

/// Adding an Agent to an account its person already uses needs the
/// operator's explicit confirmation.
pub(super) fn refuse_unconfirmed_existing_account(
    input: &OperatorCreateAgentInput,
) -> CoreResult<()> {
    if input.existing_account_confirmed {
        Ok(())
    } else {
        Err(CoreError::OperatorAgentCreationRefused(
            "owner account has already signed in; confirm adding an agent to an existing account",
        ))
    }
}

/// The provisioned WorkOS user must be the one this owner signs in as: the
/// linked one when they have signed in, otherwise one no other Core account
/// holds and the same one earlier operator creations for them used.
pub(super) async fn refuse_mismatched_owner_identity<C>(
    client: &C,
    owner: &CoreUser,
    identity: &OwnerSignInIdentity,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    if owner.status == UserLinkStatus::Linked {
        if owner.workos_user_id.as_deref() != Some(identity.workos_user_id.as_str()) {
            return Err(CoreError::OperatorAgentCreationRefused(
                "owner signs in as a different WorkOS user than the one provisioned",
            ));
        }
        return Ok(());
    }
    let row = client
        .query_one(
            "SELECT
               EXISTS (SELECT 1 FROM users WHERE workos_user_id = $1 AND id <> $2) AS taken,
               EXISTS (SELECT 1 FROM operator_agent_creations
                       WHERE owner_user_id = $2 AND owner_workos_user_id <> $1) AS differs",
            &[&identity.workos_user_id, &owner.id],
        )
        .await
        .map_err(store_error)?;
    if row.get::<_, bool>("taken") {
        return Err(CoreError::OperatorAgentCreationRefused(
            "the provisioned WorkOS user already belongs to another Core account",
        ));
    }
    if row.get::<_, bool>("differs") {
        return Err(CoreError::OperatorAgentCreationRefused(
            "an earlier operator creation bound this owner to a different WorkOS user",
        ));
    }
    Ok(())
}

/// An event trial stops every Agent in its org when it ends unpaid. An org
/// that has started one never gets an operator Agent, and an org with an
/// operator Agent can never start one (`reserve_trial`).
pub(super) async fn refuse_trial_org<C>(client: &C, org: &CustomerOrganization) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
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

/// The Agent's Finite Private key comes from the owner's grant, which the
/// operator path never changes. A grant an admin revoked stays revoked, so
/// the creation is refused up front instead of failing at launch.
pub(super) async fn refuse_inactive_private_grant<C>(client: &C, owner: &CoreUser) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let status: Option<String> = client
        .query_opt(
            "SELECT status FROM finite_private_grants WHERE user_id = $1",
            &[&owner.id],
        )
        .await
        .map_err(store_error)?
        .map(|row| row.get(0));
    if status.is_some_and(|status| status != "active") {
        return Err(CoreError::OperatorAgentCreationRefused(
            "owner's Finite Private grant is not active; restore it before creating an agent",
        ));
    }
    Ok(())
}

/// Find the owner by email, or mint them as a pending account. `true` only
/// when this transaction's own INSERT created the row: a concurrent creator
/// for the same email resolves to the conflict branch and reports `false`.
pub(super) async fn upsert_pending_owner<C>(
    client: &C,
    email: &str,
    now: &str,
) -> CoreResult<(CoreUser, bool)>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_one(
            "INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
             VALUES ($1, $2, 'pending', NULL, $3::text::timestamptz, $3::text::timestamptz)
             ON CONFLICT (normalized_email) DO UPDATE SET updated_at = users.updated_at
             RETURNING id, normalized_email, link_status, workos_user_id,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at,
                       (xmax = 0) AS inserted",
            &[&new_user_id()?, &email, &now],
        )
        .await
        .map_err(store_error)?;
    Ok((core_user_from_row(&row)?, row.get("inserted")))
}

/// The owner's personal org, minted sponsored when missing. An existing org
/// keeps its billing class. The upsert holds the org row lock until commit.
pub(super) async fn upsert_sponsored_org<C>(
    client: &C,
    owner: &CoreUser,
    now: &str,
) -> CoreResult<(CustomerOrganization, bool)>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_one(
            "INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
             VALUES ($1, $2, $3, 'sponsored', $4::text::timestamptz, $4::text::timestamptz)
             ON CONFLICT (owner_user_id) DO UPDATE SET updated_at = customer_orgs.updated_at
             RETURNING id, owner_user_id, name, billing_class,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at,
                       (xmax = 0) AS inserted",
            &[&new_customer_org_id()?, &owner.id, &owner.email, &now],
        )
        .await
        .map_err(store_error)?;
    Ok((customer_org_from_row(&row)?, row.get("inserted")))
}
