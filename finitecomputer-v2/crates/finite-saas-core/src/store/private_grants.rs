use super::*;

impl CoreStore {
    pub async fn approve_finite_private_grant(
        &self,
        input: ApproveFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let grant = postgres_approve_finite_private_grant(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(grant)
    }

    pub async fn revoke_finite_private_grant(
        &self,
        input: RevokeFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let grant = postgres_revoke_finite_private_grant(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(grant)
    }

    pub async fn reset_finite_private_usage_window(
        &self,
        input: ResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let grant = postgres_reset_finite_private_usage_window(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(grant)
    }
}

/// Return an existing profile, create one of Core's built-in profiles on
/// demand, and reject unknown profile identifiers.
pub(super) async fn ensure_finite_private_limit_profile_row<C>(
    client: &C,
    id: &str,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    if client
        .query_opt(
            "SELECT id FROM finite_private_limit_profiles WHERE id = $1",
            &[&id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Ok(());
    }
    let burst_limit_units = match id {
        crate::DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE => {
            crate::DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS
        }
        crate::FINITE_PRIVATE_5X_LIMIT_PROFILE => crate::FINITE_PRIVATE_5X_BURST_LIMIT_UNITS,
        _ => return Err(CoreError::FinitePrivateLimitProfileNotFound),
    };
    client
        .execute(
            "INSERT INTO finite_private_limit_profiles (
               id, burst_window_seconds, burst_limit_units, weekly_limit_units, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5::text::timestamptz, $5::text::timestamptz)
             ON CONFLICT (id) DO NOTHING",
            &[
                &id,
                &crate::DEFAULT_FINITE_PRIVATE_BURST_WINDOW_SECONDS,
                &burst_limit_units,
                &crate::DEFAULT_FINITE_PRIVATE_WEEKLY_LIMIT_UNITS,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

pub(super) async fn approve_finite_private_grant_row<C>(
    client: &C,
    user: &CoreUser,
    limit_profile_id: &str,
    now: &str,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    ensure_finite_private_limit_profile_row(client, limit_profile_id, now).await?;
    let grant_id = finite_private_grant_id_for_user(&user.id);
    let row = client
        .query_one(
            "INSERT INTO finite_private_grants (
               id, user_id, limit_profile_id, status, current_window_started_at,
               current_window_used_units, burst_window_epoch, created_at, updated_at
             )
             VALUES ($1, $2, $3, 'active', NULL, 0, 0, $4::text::timestamptz, $4::text::timestamptz)
             ON CONFLICT (user_id) DO UPDATE SET
               limit_profile_id = EXCLUDED.limit_profile_id,
               status = 'active',
               current_window_started_at = NULL,
               current_window_used_units = 0,
               burst_window_epoch = finite_private_grants.burst_window_epoch + 1,
               updated_at = EXCLUDED.updated_at
             RETURNING id, user_id, limit_profile_id, status,
                       core_rfc3339(current_window_started_at) AS current_window_started_at,
                       current_window_used_units, burst_window_epoch,
                       core_rfc3339(created_at) AS created_at,
                       core_rfc3339(updated_at) AS updated_at",
            &[&grant_id, &user.id, &limit_profile_id, &now],
        )
        .await
        .map_err(store_error)?;
    let grant = finite_private_grant_from_row(&row)?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.grant.approve",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: None,
            metadata: json!({
            "userId": grant.user_id.clone(),
            "limitProfileId": grant.limit_profile_id.clone(),
            "verifiedEmail": user.email.clone()
            }),
            now,
        },
    )
    .await?;
    Ok(grant)
}

pub(super) async fn postgres_approve_finite_private_grant<C>(
    client: &C,
    input: ApproveFinitePrivateGrantInput,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let verified_email = normalize_owner_email(Some(&input.verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let limit_profile_id = trim_to_option(input.limit_profile_id.as_deref())
        .unwrap_or_else(|| crate::DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE.to_string());
    let user = match trim_to_option(input.workos_user_id.as_deref()) {
        Some(workos_user_id) => {
            ensure_grandfathered_linked_user(client, &verified_email, &workos_user_id, &now).await?
        }
        None => ensure_pending_user_row(client, &verified_email, &now).await?,
    };
    approve_finite_private_grant_row(client, &user, &limit_profile_id, &now).await
}

async fn postgres_revoke_finite_private_grant<C>(
    client: &C,
    input: RevokeFinitePrivateGrantInput,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let grant_id =
        trim_to_option(Some(&input.grant_id)).ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let row = client
        .query_opt(
            "UPDATE finite_private_grants
             SET status = 'revoked', updated_at = $2::text::timestamptz
             WHERE id = $1
             RETURNING id, user_id, limit_profile_id, status,
                       core_rfc3339(current_window_started_at) AS current_window_started_at,
                       current_window_used_units, burst_window_epoch,
                       core_rfc3339(created_at) AS created_at,
                       core_rfc3339(updated_at) AS updated_at",
            &[&grant_id, &now],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let grant = finite_private_grant_from_row(&row)?;
    // Revoke every key under the grant (the in-memory model bumps them all).
    let revoked = client
        .query(
            "UPDATE finite_private_api_keys
             SET status = 'revoked', updated_at = $2::text::timestamptz
             WHERE grant_id = $1
             RETURNING id",
            &[&grant_id, &now],
        )
        .await
        .map_err(store_error)?;
    let revoked_api_key_ids: Vec<String> = revoked.iter().map(|row| row.get("id")).collect();
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.grant.revoke",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: None,
            metadata: json!({ "revokedApiKeyIds": revoked_api_key_ids }),
            now: &now,
        },
    )
    .await?;
    Ok(grant)
}

pub(super) async fn postgres_reset_finite_private_usage_window<C>(
    client: &C,
    input: ResetFinitePrivateUsageWindowInput,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let grant_id =
        trim_to_option(Some(&input.grant_id)).ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let row = client
        .query_opt(
            "UPDATE finite_private_grants
             SET current_window_started_at = $2::text::timestamptz,
                 current_window_used_units = 0,
                 burst_window_epoch = burst_window_epoch + 1,
                 updated_at = $2::text::timestamptz
             WHERE id = $1
             RETURNING id, user_id, limit_profile_id, status,
                       core_rfc3339(current_window_started_at) AS current_window_started_at,
                       current_window_used_units, burst_window_epoch,
                       core_rfc3339(created_at) AS created_at,
                       core_rfc3339(updated_at) AS updated_at",
            &[&grant_id, &now],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let grant = finite_private_grant_from_row(&row)?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.grant.reset_window",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: None,
            metadata: json!({}),
            now: &now,
        },
    )
    .await?;
    Ok(grant)
}
