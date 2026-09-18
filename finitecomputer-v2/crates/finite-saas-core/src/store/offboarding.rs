use super::*;

/// Read the runtime's recorded offboarding phase. The callers that mutate
/// phases already hold the runtime row locked inside their transaction.
pub(super) async fn postgres_offboarding_phase<C>(
    client: &C,
    agent_runtime_id: &str,
) -> CoreResult<Option<OffboardingPhase>>
where
    C: GenericClient + Sync,
{
    let phase: Option<String> = client
        .query_opt(
            "SELECT offboarding_phase FROM agent_runtimes WHERE id = $1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::ProjectRuntimeNotFound)?
        .get("offboarding_phase");
    phase
        .as_deref()
        .map(|value| {
            parse_offboarding_phase(value)
                .ok_or_else(|| CoreError::Store(format!("invalid offboarding phase {value}")))
        })
        .transpose()
}

/// Advance the runtime's offboarding phase strictly forward, in the same
/// transaction as the side effect the phase records. Restating the current
/// phase is an idempotent no-op (replayed completions); any backward move
/// fails closed and names both phases.
pub(super) async fn set_offboarding_phase<C>(
    client: &C,
    agent_runtime_id: &str,
    phase: OffboardingPhase,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let updated = client
        .execute(
            "UPDATE agent_runtimes
             SET offboarding_phase = $2, updated_at = $3::text::timestamptz
             WHERE id = $1
               AND (
                 offboarding_phase IS NULL
                 OR offboarding_phase = $2
                 OR array_position(
                      ARRAY['retirement_requested', 'receipt_verified', 'compute_removed',
                            'link_deactivated', 'archived']::text[],
                      offboarding_phase
                    ) < array_position(
                      ARRAY['retirement_requested', 'receipt_verified', 'compute_removed',
                            'link_deactivated', 'archived']::text[],
                      $2
                    )
               )",
            &[&agent_runtime_id, &phase.as_str(), &now],
        )
        .await
        .map_err(store_error)?;
    if updated == 1 {
        return Ok(());
    }
    let current = postgres_offboarding_phase(client, agent_runtime_id)
        .await?
        .ok_or_else(|| {
            CoreError::Store(format!(
                "runtime {agent_runtime_id} rejected the forward-only offboarding phase update with no recorded phase"
            ))
        })?;
    Err(CoreError::OffboardingPhaseRegression {
        current,
        attempted: phase,
    })
}

/// Row-scoped `offboard_destroyed_runtime`: hide the normal project from its
/// room members, deactivate the runtime's links, drop its relay credential,
/// revoke every active Finite Private key bound to the runtime or its project,
/// and audit the revocation. Project, membership, runtime, and link rows remain
/// retained for recovery and audit.
pub(super) async fn postgres_offboard_destroyed_runtime<C>(
    client: &C,
    request: &RuntimeControlRequest,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    postgres_offboard_runtime(
        client,
        &request.project_id,
        &request.agent_runtime_id,
        now,
        "finite_private.runtime.destroy_revoke_keys",
        None,
    )
    .await?;
    set_offboarding_phase(
        client,
        &request.agent_runtime_id,
        OffboardingPhase::Archived,
        now,
    )
    .await?;
    Ok(())
}

pub(super) async fn postgres_offboard_runtime<C>(
    client: &C,
    project_id: &str,
    agent_runtime_id: &str,
    now: &str,
    revocation_action: &'static str,
    actor: Option<&str>,
) -> CoreResult<Vec<String>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id FROM agent_runtimes WHERE id=$1 FOR UPDATE",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?;
    client
        .execute(
            "UPDATE runtime_core_credentials SET revoked=TRUE WHERE agent_runtime_id=$1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?;
    client
        .execute(
            "UPDATE project_room_memberships AS membership
             SET archived_at = $2::text::timestamptz
             WHERE membership.project_id = $1
               AND membership.archived_at IS NULL
               AND EXISTS (
                 SELECT 1
                 FROM projects AS project
                 WHERE project.id = $1
                   AND project.import_candidate_id IS NULL
               )",
            &[&project_id, &now],
        )
        .await
        .map_err(store_error)?;
    client
        .execute(
            "UPDATE project_runtime_links SET active = FALSE WHERE agent_runtime_id = $1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?;
    // The link deactivation above is the offboarding boundary; record it in
    // the same transaction.
    set_offboarding_phase(
        client,
        agent_runtime_id,
        OffboardingPhase::LinkDeactivated,
        now,
    )
    .await?;
    client
        .execute(
            "DELETE FROM runtime_relay_credentials WHERE agent_runtime_id = $1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?;
    let revoked_rows = client
        .query(
            "UPDATE finite_private_api_keys
             SET status = 'revoked', updated_at = $3::text::timestamptz
             WHERE status = 'active'
               AND (agent_runtime_id = $1 OR project_id = $2)
             RETURNING id",
            &[&agent_runtime_id, &project_id, &now],
        )
        .await
        .map_err(store_error)?;
    let revoked_api_key_ids: Vec<String> = revoked_rows.iter().map(|row| row.get("id")).collect();
    if !revoked_api_key_ids.is_empty() {
        insert_finite_private_admin_audit_event(
            client,
            FinitePrivateAdminAuditInsert {
                action: revocation_action,
                target_type: "agent_runtime",
                target_id: agent_runtime_id,
                grant_id: None,
                api_key_id: None,
                actor,
                metadata: json!({
                    "projectId": project_id,
                    "revokedApiKeyIds": revoked_api_key_ids,
                }),
                now,
            },
        )
        .await?;
    }
    Ok(revoked_api_key_ids)
}
