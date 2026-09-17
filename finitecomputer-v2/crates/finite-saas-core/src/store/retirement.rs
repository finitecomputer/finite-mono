use super::*;

impl CoreStore {
    pub async fn admin_archive_unrecoverable_runtime(
        &self,
        input: AdminArchiveUnrecoverableRuntimeInput,
    ) -> CoreResult<UnrecoverableRuntimeArchiveReceipt> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let receipt = postgres_admin_archive_unrecoverable_runtime(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(receipt)
    }

    pub async fn admin_offboard_retired_runtime(
        &self,
        input: AdminOffboardRetiredRuntimeInput,
    ) -> CoreResult<RetiredRuntimeOffboardReceipt> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let receipt = postgres_admin_offboard_retired_runtime(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(receipt)
    }
}

async fn postgres_admin_archive_unrecoverable_runtime<C>(
    client: &C,
    input: AdminArchiveUnrecoverableRuntimeInput,
) -> CoreResult<UnrecoverableRuntimeArchiveReceipt>
where
    C: GenericClient + Sync,
{
    if !input.operator_observed_compute_absent
        || !input.operator_observed_durable_state_absent
        || !input.owner_acknowledged_unrecoverable
    {
        return Err(CoreError::UnrecoverableRuntimeArchiveAcknowledgementRequired);
    }
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let admin_workos_user_id = input.admin_workos_user_id.trim().to_string();
    if admin_workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let expected_owner_email = normalize_owner_email(Some(&input.expected_owner_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let row = client
        .query_opt(
            "SELECT runtime.id AS agent_runtime_id, runtime.source_host_id,
                    runtime.source_machine_id, owner.normalized_email AS owner_email,
                    (
                      runtime.provider_runtime_handle IS NOT NULL
                      OR COALESCE(jsonb_array_length(runtime.provider_runtime_handle_history), 0) > 0
                      OR runtime.contact_endpoint IS NOT NULL
                    ) AS has_provider_metadata
             FROM projects AS project
             JOIN users AS owner ON owner.id = project.owner_user_id
             JOIN project_runtime_links AS link
               ON link.project_id = project.id AND link.active = TRUE
             JOIN agent_runtimes AS runtime ON runtime.id = link.agent_runtime_id
             WHERE project.id = $1
             FOR UPDATE OF project, link, runtime",
            &[&input.project_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::ProjectRuntimeNotFound)?;
    let agent_runtime_id: String = row.get("agent_runtime_id");
    let source_host_id: String = row.get("source_host_id");
    let source_machine_id: String = row.get("source_machine_id");
    let owner_email: String = row.get("owner_email");
    if owner_email != expected_owner_email {
        return Err(CoreError::UnrecoverableRuntimeArchiveOwnerMismatch);
    }
    if agent_runtime_id != input.expected_agent_runtime_id
        || source_host_id != input.expected_source_host_id
        || source_machine_id != input.expected_source_machine_id
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    if row.get::<_, bool>("has_provider_metadata") {
        return Err(CoreError::UnrecoverableRuntimeArchiveProviderMetadataPresent);
    }
    if client
        .query_opt(
            "SELECT 1
             FROM runtime_control_requests
             WHERE agent_runtime_id = $1
               AND status IN ('requested', 'launching', 'compute_up', 'ready')
             LIMIT 1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::RuntimeControlOperationConflict);
    }
    if client
        .query_opt(
            "SELECT 1 FROM runtime_retirement_snapshots WHERE agent_runtime_id = $1 LIMIT 1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::RuntimeRetirementSnapshotConflict);
    }

    ensure_grandfathered_linked_user(client, &admin_email, &admin_workos_user_id, &now).await?;
    let revoked_api_key_ids = postgres_offboard_runtime(
        client,
        &input.project_id,
        &agent_runtime_id,
        &now,
        "finite_private.runtime.archive_unrecoverable_revoke_keys",
        Some(&admin_email),
    )
    .await?;
    set_offboarding_phase(client, &agent_runtime_id, OffboardingPhase::Archived, &now).await?;
    let revoked_finite_private_key_count = revoked_api_key_ids.len();
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "runtime.admin_archive_unrecoverable",
            target_type: "agent_runtime",
            target_id: &agent_runtime_id,
            grant_id: None,
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({
                "projectId": input.project_id,
                "ownerEmail": owner_email,
                "sourceHostId": source_host_id,
                "sourceMachineId": source_machine_id,
                "operatorObservedComputeAbsent": true,
                "operatorObservedDurableStateAbsent": true,
                "ownerAcknowledgedUnrecoverable": true,
                "revokedApiKeyIds": revoked_api_key_ids,
            }),
            now: &now,
        },
    )
    .await?;
    Ok(UnrecoverableRuntimeArchiveReceipt {
        project_id: input.project_id,
        agent_runtime_id,
        source_host_id,
        source_machine_id,
        owner_email,
        archived_at: now,
        revoked_finite_private_key_count,
    })
}

/// Repair boundary for a Runtime whose destroy control stored a VERIFIED
/// retirement receipt but whose offboarding transaction never ran (the
/// `project_runtime_links.active` link, room membership, relay credential, and
/// Finite Private keys survive with no compute behind them). This path never
/// creates, modifies, or deletes the retirement snapshot, and never touches
/// provider metadata columns.
///
/// The command is safe to run twice: a second run finds no active link and
/// fails closed with `ProjectRuntimeNotFound`, leaving every committed effect
/// of the first run untouched.
async fn postgres_admin_offboard_retired_runtime<C>(
    client: &C,
    input: AdminOffboardRetiredRuntimeInput,
) -> CoreResult<RetiredRuntimeOffboardReceipt>
where
    C: GenericClient + Sync,
{
    if !input.operator_observed_compute_absent {
        return Err(CoreError::RetiredRuntimeOffboardAcknowledgementRequired);
    }
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let admin_workos_user_id = input.admin_workos_user_id.trim().to_string();
    if admin_workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let expected_owner_email = normalize_owner_email(Some(&input.expected_owner_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let row = client
        .query_opt(
            "SELECT runtime.id AS agent_runtime_id, runtime.source_host_id,
                    runtime.source_machine_id, owner.normalized_email AS owner_email
             FROM projects AS project
             JOIN users AS owner ON owner.id = project.owner_user_id
             JOIN project_runtime_links AS link
               ON link.project_id = project.id AND link.active = TRUE
             JOIN agent_runtimes AS runtime ON runtime.id = link.agent_runtime_id
             WHERE project.id = $1
             FOR UPDATE OF project, link, runtime",
            &[&input.project_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::ProjectRuntimeNotFound)?;
    let agent_runtime_id: String = row.get("agent_runtime_id");
    let source_host_id: String = row.get("source_host_id");
    let source_machine_id: String = row.get("source_machine_id");
    let owner_email: String = row.get("owner_email");
    if owner_email != expected_owner_email {
        return Err(CoreError::RetiredRuntimeOffboardOwnerMismatch);
    }
    if agent_runtime_id != input.expected_agent_runtime_id
        || source_host_id != input.expected_source_host_id
        || source_machine_id != input.expected_source_machine_id
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    if client
        .query_opt(
            "SELECT 1
             FROM runtime_control_requests
             WHERE agent_runtime_id = $1
               AND status IN ('requested', 'launching', 'compute_up', 'ready')
             LIMIT 1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::RuntimeControlOperationConflict);
    }
    let snapshot_row = client
        .query_opt(
            "SELECT request_id
             FROM runtime_retirement_snapshots
             WHERE agent_runtime_id = $1 AND verified_at IS NOT NULL
             LIMIT 1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::RetiredRuntimeOffboardReceiptMissing)?;
    let retirement_request_id: String = snapshot_row.get("request_id");
    let snapshot = postgres_runtime_retirement_snapshot(client, &retirement_request_id)
        .await?
        .ok_or(CoreError::RetiredRuntimeOffboardReceiptMissing)?;
    let request = locked_runtime_control_request(client, &retirement_request_id).await?;
    let runtime = select_agent_runtime(client, &agent_runtime_id)
        .await?
        .ok_or(CoreError::ProjectRuntimeNotFound)?;
    let spec_row = client
        .query_opt(
            "SELECT runtime_spec
             FROM agent_creation_requests
             WHERE agent_runtime_id = $1 AND runtime_spec IS NOT NULL
             ORDER BY created_at DESC, id DESC
             LIMIT 1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::RuntimeRetirementSnapshotMismatch)?;
    let value: Value = spec_row.get("runtime_spec");
    let runtime_spec: RuntimeSpecEnvelope = serde_json::from_value(value).map_err(json_error)?;
    // The stored receipt must re-verify against its own destroy request,
    // Runtime binding, and RuntimeSpec exactly as at destroy completion.
    validate_runtime_retirement_snapshot_receipt(
        &snapshot.receipt,
        &request,
        &runtime,
        &runtime_spec,
        &now,
    )?;

    ensure_grandfathered_linked_user(client, &admin_email, &admin_workos_user_id, &now).await?;
    // The operator's compute-absent attestation (required above) plus the
    // re-verified receipt resume the recorded phase forward.
    set_offboarding_phase(
        client,
        &agent_runtime_id,
        OffboardingPhase::ComputeRemoved,
        &now,
    )
    .await?;
    let revoked_api_key_ids = postgres_offboard_runtime(
        client,
        &input.project_id,
        &agent_runtime_id,
        &now,
        "finite_private.runtime.offboard_retired_revoke_keys",
        Some(&admin_email),
    )
    .await?;
    set_offboarding_phase(client, &agent_runtime_id, OffboardingPhase::Archived, &now).await?;
    let revoked_finite_private_key_count = revoked_api_key_ids.len();
    let retirement_locator = snapshot.receipt.locator.clone();
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "runtime.admin_offboard_retired",
            target_type: "agent_runtime",
            target_id: &agent_runtime_id,
            grant_id: None,
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({
                "projectId": input.project_id,
                "ownerEmail": owner_email,
                "sourceHostId": source_host_id,
                "sourceMachineId": source_machine_id,
                "operatorObservedComputeAbsent": true,
                "retirementRequestId": retirement_request_id,
                "retirementLocator": retirement_locator,
                "revokedApiKeyIds": revoked_api_key_ids,
            }),
            now: &now,
        },
    )
    .await?;
    Ok(RetiredRuntimeOffboardReceipt {
        project_id: input.project_id,
        agent_runtime_id,
        retirement_request_id,
        retirement_locator,
        offboarded_at: now,
        revoked_finite_private_key_count,
    })
}
