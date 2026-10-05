use super::*;

pub(super) async fn provision_trial_restore_key<C: GenericClient + Sync>(
    client: &C,
    request: &AgentCreationRequest,
    user_id: &str,
    proposed: Option<String>,
    now: &str,
) -> CoreResult<ProvisionFinitePrivateRuntimeKeyResult> {
    let raw_api_key = proposed.ok_or(CoreError::InvalidFinitePrivateApiKey)?;
    if !raw_api_key
        .strip_prefix("fpk_live_")
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(CoreError::InvalidFinitePrivateApiKey);
    }
    let key_hash = hash_finite_private_api_key(&raw_api_key)?;
    // Restore must not silently reactivate an administratively revoked grant.
    let grant =
        select_finite_private_grant(client, &finite_private_grant_id_for_user(user_id), true)
            .await?
            .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    if grant.status != FinitePrivateGrantStatus::Active {
        return Err(CoreError::FinitePrivateGrantNotActive);
    }
    let stored: Option<String> = client.query_one(
        "SELECT restore_private_key_id FROM trial_runtime_archives WHERE restore_request_id=$1 FOR UPDATE",
        &[&request.id],
    ).await.map_err(store_error)?.get(0);
    let api_key = if let Some(key_id) = stored {
        let row = client
            .query_one(
                "SELECT id, grant_id, project_id, agent_runtime_id, key_hash, status,
             core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
             FROM finite_private_api_keys WHERE id=$1 FOR UPDATE",
                &[&key_id],
            )
            .await
            .map_err(store_error)?;
        let key = finite_private_api_key_from_row(&row)?;
        if key.key_hash != key_hash
            || key.status != FinitePrivateApiKeyStatus::Active
            || key.grant_id != grant.id
            || key.project_id.as_deref() != Some(request.project_id.as_str())
            || key.agent_runtime_id != request.agent_runtime_id
        {
            return Err(CoreError::InvalidFinitePrivateApiKey);
        }
        key
    } else {
        // Insert-only: an existing hash must never be rebound or reactivated,
        // including a concurrent proposal from a different restore operation.
        let key_id = finite_private_api_key_id_for(&grant.id, &key_hash);
        let row = client
            .query_opt(
                "INSERT INTO finite_private_api_keys
             (id,grant_id,project_id,agent_runtime_id,key_hash,status,created_at,updated_at)
             VALUES($1,$2,$3,$4,$5,'active',$6::text::timestamptz,$6::text::timestamptz)
             ON CONFLICT DO NOTHING
             RETURNING id,grant_id,project_id,agent_runtime_id,key_hash,status,
             core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
                &[
                    &key_id,
                    &grant.id,
                    &request.project_id,
                    &request.agent_runtime_id,
                    &key_hash,
                    &now,
                ],
            )
            .await
            .map_err(store_error)?
            .ok_or(CoreError::InvalidFinitePrivateApiKey)?;
        let key = finite_private_api_key_from_row(&row)?;
        client.execute("UPDATE trial_runtime_archives SET restore_private_key_id=$2 WHERE restore_request_id=$1", &[&request.id,&key.id]).await.map_err(store_error)?;
        insert_finite_private_admin_audit_event(client, FinitePrivateAdminAuditInsert {
            action: "finite_private.api_key.issue", target_type:"api_key", target_id:&key.id,
            grant_id:Some(&grant.id), api_key_id:Some(&key.id), actor:None,
            metadata:json!({"projectId":request.project_id,"agentRuntimeId":request.agent_runtime_id,"restoreRequestId":request.id}), now,
        }).await?;
        key
    };
    Ok(ProvisionFinitePrivateRuntimeKeyResult {
        grant,
        api_key,
        raw_api_key,
    })
}
