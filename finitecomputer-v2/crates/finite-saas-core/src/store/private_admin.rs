use super::*;

impl CoreStore {
    pub async fn admin_issue_finite_private_friend_key(
        &self,
        input: AdminIssueFinitePrivateFriendKeyInput,
    ) -> CoreResult<AdminIssuedFinitePrivateKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_issue_finite_private_friend_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_rotate_finite_private_api_key(
        &self,
        input: AdminRotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_rotate_finite_private_api_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_revoke_finite_private_api_key(
        &self,
        input: AdminRevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_revoke_finite_private_api_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_reset_finite_private_usage_window(
        &self,
        input: AdminResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_reset_finite_private_usage_window(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_assign_finite_private_limit_profile(
        &self,
        input: AdminAssignFinitePrivateLimitProfileInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_assign_finite_private_limit_profile(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn finite_private_admin_audit_events(
        &self,
    ) -> CoreResult<Vec<FinitePrivateAdminAuditEvent>> {
        let client = self.connection().await?;
        postgres_finite_private_admin_audit_events(&**client).await
    }

    pub async fn finite_private_admin_state(&self) -> CoreResult<FinitePrivateAdminState> {
        let client = self.connection().await?;
        postgres_finite_private_admin_state(&**client).await
    }
}

pub(super) async fn insert_finite_private_admin_audit_event<C>(
    client: &C,
    event: FinitePrivateAdminAuditInsert<'_>,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let actor = event.actor.unwrap_or("finite-saas-core");
    let id = crate::id_from_parts(
        "fp_audit",
        &[event.action, event.target_id, actor, event.now],
    );
    client
        .execute(
            "INSERT INTO finite_private_admin_audit_events (
               id, action, target_type, target_id, grant_id, api_key_id, actor, metadata, created_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9::text::timestamptz)
            ON CONFLICT (id) DO NOTHING",
            &[
                &id,
                &event.action,
                &event.target_type,
                &event.target_id,
                &event.grant_id,
                &event.api_key_id,
                &actor,
                &event.metadata,
                &event.now,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn postgres_admin_issue_finite_private_friend_key<C>(
    client: &C,
    input: AdminIssueFinitePrivateFriendKeyInput,
) -> CoreResult<AdminIssuedFinitePrivateKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let grant = postgres_approve_finite_private_grant(
        client,
        ApproveFinitePrivateGrantInput {
            verified_email: input.friend_email,
            workos_user_id: None,
            limit_profile_id: input.limit_profile_id,
            now: Some(now.clone()),
        },
    )
    .await?;
    let api_key = postgres_issue_finite_private_api_key(
        client,
        IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(),
            raw_key: input.raw_key,
            project_id: None,
            agent_runtime_id: None,
            now: Some(now.clone()),
        },
    )
    .await?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.friend_key.admin_issue",
            target_type: "api_key",
            target_id: &api_key.id,
            grant_id: Some(&grant.id),
            api_key_id: Some(&api_key.id),
            actor: Some(&admin_email),
            metadata: json!({ "limitProfileId": grant.limit_profile_id.clone() }),
            now: &now,
        },
    )
    .await?;
    Ok(AdminIssuedFinitePrivateKey { grant, api_key })
}

async fn postgres_admin_rotate_finite_private_api_key<C>(
    client: &C,
    input: AdminRotateFinitePrivateApiKeyInput,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let old_key_id = input.key_id.trim().to_string();
    let key = postgres_rotate_finite_private_api_key(
        client,
        RotateFinitePrivateApiKeyInput {
            key_id: input.key_id,
            raw_key: input.raw_key,
            now: Some(now.clone()),
        },
    )
    .await?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.api_key.admin_rotate",
            target_type: "api_key",
            target_id: &key.id,
            grant_id: Some(&key.grant_id),
            api_key_id: Some(&key.id),
            actor: Some(&admin_email),
            metadata: json!({ "oldApiKeyId": old_key_id }),
            now: &now,
        },
    )
    .await?;
    Ok(key)
}

async fn postgres_admin_revoke_finite_private_api_key<C>(
    client: &C,
    input: AdminRevokeFinitePrivateApiKeyInput,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let key = postgres_revoke_finite_private_api_key(
        client,
        RevokeFinitePrivateApiKeyInput {
            key_id: input.key_id,
            now: Some(now.clone()),
        },
    )
    .await?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.api_key.admin_revoke",
            target_type: "api_key",
            target_id: &key.id,
            grant_id: Some(&key.grant_id),
            api_key_id: Some(&key.id),
            actor: Some(&admin_email),
            metadata: json!({}),
            now: &now,
        },
    )
    .await?;
    Ok(key)
}

async fn postgres_admin_reset_finite_private_usage_window<C>(
    client: &C,
    input: AdminResetFinitePrivateUsageWindowInput,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let grant = postgres_reset_finite_private_usage_window(
        client,
        ResetFinitePrivateUsageWindowInput {
            grant_id: input.grant_id,
            now: Some(now.clone()),
        },
    )
    .await?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.grant.admin_window_reset",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({}),
            now: &now,
        },
    )
    .await?;
    Ok(grant)
}

async fn postgres_admin_assign_finite_private_limit_profile<C>(
    client: &C,
    input: AdminAssignFinitePrivateLimitProfileInput,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let grant_id =
        trim_to_option(Some(&input.grant_id)).ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let limit_profile_id = trim_to_option(Some(&input.limit_profile_id))
        .ok_or(CoreError::FinitePrivateLimitProfileNotFound)?;
    ensure_finite_private_limit_profile_row(client, &limit_profile_id, &now).await?;
    let previous = select_finite_private_grant(client, &grant_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    client
        .execute(
            "UPDATE finite_private_grants
             SET limit_profile_id = $2, updated_at = $3::text::timestamptz
             WHERE id = $1",
            &[&grant_id, &limit_profile_id, &now],
        )
        .await
        .map_err(store_error)?;
    let grant = select_finite_private_grant(client, &grant_id, false)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.grant.admin_assign_limit_profile",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({
                "previousLimitProfileId": previous.limit_profile_id,
                "limitProfileId": grant.limit_profile_id.clone(),
            }),
            now: &now,
        },
    )
    .await?;
    Ok(grant)
}

async fn postgres_finite_private_admin_audit_events<C>(
    client: &C,
) -> CoreResult<Vec<FinitePrivateAdminAuditEvent>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT id, action, target_type, target_id, grant_id, api_key_id, actor, metadata,
                {created} AS created_at
         FROM finite_private_admin_audit_events
         ORDER BY created_at, id",
        created = rfc3339_col("created_at"),
    );
    client
        .query(&sql, &[])
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            Ok(FinitePrivateAdminAuditEvent {
                id: row.get("id"),
                action: row.get("action"),
                target_type: row.get("target_type"),
                target_id: row.get("target_id"),
                grant_id: row.get("grant_id"),
                api_key_id: row.get("api_key_id"),
                actor: row.get("actor"),
                metadata: json_column(row, "metadata")?,
                created_at: row.get("created_at"),
            })
        })
        .collect()
}

async fn postgres_finite_private_admin_state<C>(client: &C) -> CoreResult<FinitePrivateAdminState>
where
    C: GenericClient + Sync,
{
    let grant_sql = format!(
        "SELECT fp_grant.id, fp_grant.user_id, fp_grant.limit_profile_id, fp_grant.status,
                CASE WHEN fp_grant.current_window_started_at IS NULL THEN NULL
                     ELSE {started} END AS current_window_started_at,
                fp_grant.current_window_used_units, fp_grant.burst_window_epoch,
                {created} AS created_at, {updated} AS updated_at,
                account.normalized_email
         FROM finite_private_grants AS fp_grant
         JOIN users AS account ON account.id = fp_grant.user_id
         ORDER BY fp_grant.created_at, fp_grant.id",
        started = rfc3339_col("fp_grant.current_window_started_at"),
        created = rfc3339_col("fp_grant.created_at"),
        updated = rfc3339_col("fp_grant.updated_at"),
    );
    let grant_rows = client.query(&grant_sql, &[]).await.map_err(store_error)?;
    let grants = grant_rows
        .iter()
        .map(finite_private_grant_from_row)
        .collect::<CoreResult<Vec<_>>>()?;
    let key_sql = format!(
        "SELECT id, grant_id, project_id, agent_runtime_id, key_hash, status,
                {created} AS created_at, {updated} AS updated_at
         FROM finite_private_api_keys
         ORDER BY created_at, id",
        created = rfc3339_col("created_at"),
        updated = rfc3339_col("updated_at"),
    );
    let api_keys = client
        .query(&key_sql, &[])
        .await
        .map_err(store_error)?
        .iter()
        .map(finite_private_api_key_from_row)
        .collect::<CoreResult<Vec<_>>>()?;
    let profile_sql = format!(
        "SELECT id, burst_window_seconds, burst_limit_units, weekly_limit_units,
                {created} AS created_at, {updated} AS updated_at
         FROM finite_private_limit_profiles
         ORDER BY id",
        created = rfc3339_col("created_at"),
        updated = rfc3339_col("updated_at"),
    );
    let profiles = client
        .query(&profile_sql, &[])
        .await
        .map_err(store_error)?
        .iter()
        .map(finite_private_limit_profile_from_row)
        .collect::<Vec<_>>();
    let project_rows = client
        .query(
            "SELECT project.id, project.owner_user_id, project.display_name,
                    link.agent_runtime_id
             FROM projects AS project
             JOIN finite_private_grants AS fp_grant
               ON fp_grant.user_id = project.owner_user_id
             LEFT JOIN project_runtime_links AS link
               ON link.project_id = project.id AND link.active = TRUE
             ORDER BY project.display_name, project.id",
            &[],
        )
        .await
        .map_err(store_error)?;
    let mut projects_by_user = BTreeMap::<String, Vec<FinitePrivateAdminProject>>::new();
    for row in project_rows {
        projects_by_user
            .entry(row.get("owner_user_id"))
            .or_default()
            .push(FinitePrivateAdminProject {
                id: row.get("id"),
                display_name: row.get("display_name"),
                agent_runtime_id: row.get("agent_runtime_id"),
            });
    }
    let mut accounts = grant_rows
        .iter()
        .zip(grants.iter())
        .map(|(row, grant)| FinitePrivateAdminAccount {
            user_id: grant.user_id.clone(),
            email: row.get("normalized_email"),
            grant: grant.clone(),
            api_keys: api_keys
                .iter()
                .filter(|key| key.grant_id == grant.id)
                .cloned()
                .collect(),
            projects: projects_by_user.remove(&grant.user_id).unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    accounts.sort_by(|left, right| {
        left.email
            .cmp(&right.email)
            .then_with(|| left.user_id.cmp(&right.user_id))
    });
    let admin_audit_events = postgres_finite_private_admin_audit_events(client).await?;
    Ok(FinitePrivateAdminState {
        accounts,
        profiles,
        grants,
        api_keys,
        admin_audit_events,
    })
}
