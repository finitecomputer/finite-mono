use super::*;

impl CoreStore {
    pub async fn issue_finite_private_api_key(
        &self,
        input: IssueFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let key = postgres_issue_finite_private_api_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(key)
    }

    pub async fn provision_finite_private_runtime_key(
        &self,
        input: ProvisionFinitePrivateRuntimeKeyInput,
    ) -> CoreResult<ProvisionFinitePrivateRuntimeKeyResult> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_provision_finite_private_runtime_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn revoke_finite_private_api_key(
        &self,
        input: RevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let key = postgres_revoke_finite_private_api_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(key)
    }

    pub async fn rotate_finite_private_api_key(
        &self,
        input: RotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let key = postgres_rotate_finite_private_api_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(key)
    }

    pub async fn issue_finite_private_friend_key(
        &self,
        input: IssueFinitePrivateFriendKeyInput,
    ) -> CoreResult<IssuedFinitePrivateFriendKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_issue_finite_private_friend_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }
}

async fn postgres_provision_finite_private_runtime_key<C>(
    client: &C,
    input: ProvisionFinitePrivateRuntimeKeyInput,
) -> CoreResult<ProvisionFinitePrivateRuntimeKeyResult>
where
    C: GenericClient + Sync,
{
    let now = input.now.clone().unwrap_or(current_time_iso()?);
    let request = locked_agent_creation_request(client, &input.request_id).await?;
    verify_agent_creation_lease(&request, &input.runner_id, &input.lease_token)?;
    let project = select_project(client, &request.project_id)
        .await?
        .ok_or_else(|| missing_request_project_error(&request))?;
    let user = select_user_by_id(client, &request.owner_user_id)
        .await?
        .ok_or_else(|| {
            CoreError::Store(format!(
                "agent creation request {} references missing owner user {}",
                request.id, request.owner_user_id
            ))
        })?;
    let source_host_id = input
        .source_host_id
        .as_deref()
        .and_then(|value| trim_to_option(Some(value)))
        .map(|value| normalize_source_host_id(&value))
        .transpose()?;
    let source_machine_id = input
        .source_machine_id
        .as_deref()
        .and_then(|value| trim_to_option(Some(value)))
        .map(|value| {
            let normalized = normalize_id_part(&value);
            if normalized.is_empty() {
                Err(CoreError::MissingSourceMachineId)
            } else {
                Ok(normalized)
            }
        })
        .transpose()?;
    // Resolve the runtime to bind the key to by natural key (source_import_key)
    // rather than rederiving its id from the source identifiers.
    let agent_runtime_id = match (source_host_id.as_deref(), source_machine_id.as_deref()) {
        (Some(source_host_id), Some(source_machine_id)) => {
            let key = source_import_key(source_host_id, source_machine_id);
            let by_source = select_agent_runtime_by_source_import_key(client, &key)
                .await?
                .map(|runtime| runtime.id);
            if by_source.is_some() {
                by_source
            } else if request.relocation.is_some() {
                request.agent_runtime_id.clone()
            } else {
                None
            }
        }
        _ => match request.agent_runtime_id.clone() {
            Some(runtime_id) if select_agent_runtime(client, &runtime_id).await?.is_some() => {
                Some(runtime_id)
            }
            _ => None,
        },
    };
    let grant = approve_finite_private_grant_row(
        client,
        &user,
        crate::DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE,
        &now,
    )
    .await?;
    let raw_api_key = generate_finite_private_api_key()?;
    let api_key = issue_finite_private_api_key_row(
        client,
        &grant,
        &raw_api_key,
        Some(project.id),
        agent_runtime_id,
        &now,
    )
    .await?;
    Ok(ProvisionFinitePrivateRuntimeKeyResult {
        grant,
        api_key,
        raw_api_key,
    })
}

pub(super) async fn postgres_revoke_finite_private_api_key<C>(
    client: &C,
    input: RevokeFinitePrivateApiKeyInput,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let key_id =
        trim_to_option(Some(&input.key_id)).ok_or(CoreError::InvalidFinitePrivateApiKey)?;
    let row = client
        .query_opt(
            "UPDATE finite_private_api_keys
             SET status = 'revoked', updated_at = $2::text::timestamptz
             WHERE id = $1
             RETURNING id, grant_id, project_id, agent_runtime_id, key_hash, status,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[&key_id, &now],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::InvalidFinitePrivateApiKey)?;
    let key = finite_private_api_key_from_row(&row)?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.api_key.revoke",
            target_type: "api_key",
            target_id: &key.id,
            grant_id: Some(&key.grant_id),
            api_key_id: Some(&key.id),
            actor: None,
            metadata: json!({}),
            now: &now,
        },
    )
    .await?;
    Ok(key)
}

async fn issue_finite_private_api_key_row<C>(
    client: &C,
    grant: &FinitePrivateGrant,
    raw_key: &str,
    project_id: Option<String>,
    agent_runtime_id: Option<String>,
    now: &str,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    if grant.status != FinitePrivateGrantStatus::Active {
        return Err(CoreError::FinitePrivateGrantNotActive);
    }
    let key_hash = hash_finite_private_api_key(raw_key)?;
    let key_id = finite_private_api_key_id_for(&grant.id, &key_hash);
    let row = client
        .query_one(
            "INSERT INTO finite_private_api_keys (
               id, grant_id, project_id, agent_runtime_id, key_hash, status, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, 'active', $6::text::timestamptz, $6::text::timestamptz)
             ON CONFLICT (key_hash) DO UPDATE SET
               status = 'active',
               project_id = EXCLUDED.project_id,
               agent_runtime_id = EXCLUDED.agent_runtime_id,
               updated_at = EXCLUDED.updated_at
             RETURNING id, grant_id, project_id, agent_runtime_id, key_hash, status,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[
                &key_id,
                &grant.id,
                &project_id,
                &agent_runtime_id,
                &key_hash,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    let key = finite_private_api_key_from_row(&row)?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.api_key.issue",
            target_type: "api_key",
            target_id: &key.id,
            grant_id: Some(&key.grant_id),
            api_key_id: Some(&key.id),
            actor: None,
            metadata: json!({
            "projectId": key.project_id.clone(),
            "agentRuntimeId": key.agent_runtime_id.clone()
            }),
            now,
        },
    )
    .await?;
    Ok(key)
}

pub(super) async fn postgres_issue_finite_private_api_key<C>(
    client: &C,
    input: IssueFinitePrivateApiKeyInput,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let grant_id =
        trim_to_option(Some(&input.grant_id)).ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let grant = select_finite_private_grant(client, &grant_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    issue_finite_private_api_key_row(
        client,
        &grant,
        &input.raw_key,
        trim_to_option(input.project_id.as_deref()),
        trim_to_option(input.agent_runtime_id.as_deref()),
        &now,
    )
    .await
}

pub(super) async fn postgres_rotate_finite_private_api_key<C>(
    client: &C,
    input: RotateFinitePrivateApiKeyInput,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let key_id =
        trim_to_option(Some(&input.key_id)).ok_or(CoreError::InvalidFinitePrivateApiKey)?;
    let old_row = client
        .query_opt(
            "SELECT id, grant_id, project_id, agent_runtime_id, key_hash, status,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
             FROM finite_private_api_keys WHERE id = $1 FOR UPDATE",
            &[&key_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::InvalidFinitePrivateApiKey)?;
    let old_key = finite_private_api_key_from_row(&old_row)?;
    let new_key_hash = hash_finite_private_api_key(&input.raw_key)?;
    if new_key_hash == old_key.key_hash {
        return Err(CoreError::InvalidFinitePrivateApiKey);
    }
    let grant = select_finite_private_grant(client, &old_key.grant_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let new_key = issue_finite_private_api_key_row(
        client,
        &grant,
        &input.raw_key,
        old_key.project_id.clone(),
        old_key.agent_runtime_id.clone(),
        &now,
    )
    .await?;
    postgres_revoke_finite_private_api_key(
        client,
        RevokeFinitePrivateApiKeyInput {
            key_id: old_key.id.clone(),
            now: Some(now.clone()),
        },
    )
    .await?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.api_key.rotate",
            target_type: "api_key",
            target_id: &new_key.id,
            grant_id: Some(&new_key.grant_id),
            api_key_id: Some(&new_key.id),
            actor: None,
            metadata: json!({ "oldApiKeyId": old_key.id }),
            now: &now,
        },
    )
    .await?;
    Ok(new_key)
}

/// Approve a grant and issue its first key against one client.
///
/// Callers pass a transaction, so the pair is atomic: no orphaned grant on a
/// failed key issue, and a dry run can preview both steps.
async fn postgres_issue_finite_private_friend_key<C>(
    client: &C,
    input: IssueFinitePrivateFriendKeyInput,
) -> CoreResult<IssuedFinitePrivateFriendKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let grant = postgres_approve_finite_private_grant(
        client,
        ApproveFinitePrivateGrantInput {
            verified_email: input.verified_email,
            workos_user_id: input.workos_user_id,
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
            project_id: input.project_id,
            agent_runtime_id: input.agent_runtime_id,
            now: Some(now),
        },
    )
    .await?;
    Ok(IssuedFinitePrivateFriendKey { grant, api_key })
}
