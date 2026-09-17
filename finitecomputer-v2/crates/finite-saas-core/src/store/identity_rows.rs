use super::*;

impl CoreStore {
    pub async fn link_verified_user(&self, input: LinkVerifiedUserInput) -> CoreResult<CoreUser> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified_email = normalize_owner_email(Some(&input.verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let workos_user_id = input.workos_user_id.trim().to_string();
        if workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let user = ensure_linked_user_row(
            &*tx,
            &verified_email,
            &workos_user_id,
            BillingClass::Standard,
            &now,
        )
        .await?;
        self.finish(tx).await?;
        Ok(user)
    }
}

pub(super) async fn select_user_by_id<C>(client: &C, user_id: &str) -> CoreResult<Option<CoreUser>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, normalized_email, link_status, workos_user_id,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
             FROM users WHERE id = $1",
            &[&user_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| core_user_from_row(&row))
        .transpose()
}

/// Resolve a user by their natural key (`users.normalized_email UNIQUE`). This
/// replaces the old `user_id = f(email)` derivation: identity is looked up, not
/// reconstructed, so a re-signup after a wipe finds nothing and mints a fresh id.
pub(super) async fn select_user_by_email<C>(client: &C, email: &str) -> CoreResult<Option<CoreUser>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, normalized_email, link_status, workos_user_id,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
             FROM users WHERE normalized_email = $1",
            &[&email],
        )
        .await
        .map_err(store_error)?
        .map(|row| core_user_from_row(&row))
        .transpose()
}

/// Resolve the one personal org for an owner via the
/// `customer_orgs_one_personal_org_per_owner` unique index.
pub(super) async fn select_personal_org_by_owner<C>(
    client: &C,
    owner_user_id: &str,
) -> CoreResult<Option<CustomerOrganization>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, owner_user_id, name, billing_class, core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
             FROM customer_orgs WHERE owner_user_id = $1",
            &[&owner_user_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| customer_org_from_row(&row))
        .transpose()
}

/// Find-or-create the linked user by their natural key. The conflict target is
/// `normalized_email` (UNIQUE), so an existing row keeps its already-minted
/// surrogate id and we only relink workos/status; a brand-new email gets a
/// fresh `new_user_id()`. The primary key is NEVER derived from the email.
pub(crate) async fn upsert_linked_user<C>(
    client: &C,
    email: &str,
    workos_user_id: &str,
    now: &str,
) -> CoreResult<CoreUser>
where
    C: GenericClient + Sync,
{
    let user_id = new_user_id()?;
    let row = client
        .query_one(
            "INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
             VALUES ($1, $2, 'linked', $3, $4::text::timestamptz, $4::text::timestamptz)
             ON CONFLICT (normalized_email) DO UPDATE SET
               link_status = 'linked',
               workos_user_id = EXCLUDED.workos_user_id,
               updated_at = EXCLUDED.updated_at
             RETURNING id, normalized_email, link_status, workos_user_id,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[&user_id, &email, &workos_user_id, &now],
        )
        .await
        .map_err(store_error)?;
    core_user_from_row(&row)
}

pub(crate) async fn ensure_personal_org_row<C>(
    client: &C,
    user: &CoreUser,
    billing_class: BillingClass,
    now: &str,
) -> CoreResult<CustomerOrganization>
where
    C: GenericClient + Sync,
{
    // Fresh surrogate id on insert; ON CONFLICT (owner_user_id) keeps the
    // existing org's id so the one-personal-org-per-owner invariant holds.
    let org_id = new_customer_org_id()?;
    let row = client
        .query_one(
            "INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5::text::timestamptz, $5::text::timestamptz)
             ON CONFLICT (owner_user_id) DO UPDATE SET updated_at = customer_orgs.updated_at
             RETURNING id, owner_user_id, name, billing_class, core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[&org_id, &user.id, &user.email, &billing_class.as_str(), &now],
        )
        .await
        .map_err(store_error)?;
    customer_org_from_row(&row)
}

/// Find-or-create a linked user by natural key (email), then ensure their
/// personal org exists — the Postgres equivalent of
/// `ensure_linked_user_with_billing_class`. Enforces the WorkOS-id-uniqueness
/// guard. The billing class only takes effect when the org is first created.
async fn ensure_linked_user_row<C>(
    client: &C,
    email: &str,
    workos_user_id: &str,
    billing_class: BillingClass,
    now: &str,
) -> CoreResult<CoreUser>
where
    C: GenericClient + Sync,
{
    if client
        .query_opt(
            "SELECT id FROM users WHERE workos_user_id = $1 AND normalized_email <> $2",
            &[&workos_user_id, &email],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::WorkosUserConflict);
    }
    let user = upsert_linked_user(client, email, workos_user_id, now).await?;
    ensure_personal_org_row(client, &user, billing_class, now).await?;
    Ok(user)
}

/// `ensure_linked_user` (Grandfathered default) for the import/runtime-control
/// paths that do not carry billing intent.
pub(super) async fn ensure_grandfathered_linked_user<C>(
    client: &C,
    email: &str,
    workos_user_id: &str,
    now: &str,
) -> CoreResult<CoreUser>
where
    C: GenericClient + Sync,
{
    ensure_linked_user_row(
        client,
        email,
        workos_user_id,
        BillingClass::Grandfathered,
        now,
    )
    .await
}

/// Find-or-create a PENDING user by natural key (email). Mirrors
/// `ensure_pending_user`: an existing row (pending or linked) keeps its
/// surrogate id; a brand-new email gets a fresh one. Never derives id from PII.
pub(super) async fn ensure_pending_user_row<C>(
    client: &C,
    email: &str,
    now: &str,
) -> CoreResult<CoreUser>
where
    C: GenericClient + Sync,
{
    if let Some(existing) = select_user_by_email(client, email).await? {
        return Ok(existing);
    }
    let user_id = new_user_id()?;
    let row = client
        .query_one(
            "INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
             VALUES ($1, $2, 'pending', NULL, $3::text::timestamptz, $3::text::timestamptz)
             ON CONFLICT (normalized_email) DO UPDATE SET updated_at = users.updated_at
             RETURNING id, normalized_email, link_status, workos_user_id,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[&user_id, &email, &now],
        )
        .await
        .map_err(store_error)?;
    core_user_from_row(&row)
}
