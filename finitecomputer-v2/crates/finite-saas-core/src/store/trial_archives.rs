//! Trial storage residency. No archive completion offboards the logical Agent.
use super::*;
use crate::TrialArchiveSnapshot;

pub(super) async fn snapshot<C: GenericClient + Sync>(
    tx: &C,
    runtime: &str,
) -> CoreResult<Option<TrialArchiveSnapshot>> {
    tx.query_opt("SELECT snapshot FROM trial_runtime_archives WHERE agent_runtime_id=$1 AND restored_at IS NULL", &[&runtime])
        .await.map_err(store_error)?.map(|r| serde_json::from_value(r.get(0)).map_err(json_error)).transpose()
}

pub(super) async fn principal<C: GenericClient + Sync>(
    tx: &C,
    runtime: &str,
) -> CoreResult<Option<String>> {
    Ok(tx
        .query_one(
            "SELECT health_reporting_npub FROM agent_runtimes WHERE id=$1",
            &[&runtime],
        )
        .await
        .map_err(store_error)?
        .get(0))
}

/// Called with the billing/runtime locks and no active control. The receipt
/// remains retained after residency changes; nothing here deletes recovery data.
pub(super) async fn reconcile<C: GenericClient + Sync>(
    tx: &C,
    project: &Project,
    runtime: &AgentRuntime,
    blocked: bool,
    resume_allowed: bool,
    now: &str,
) -> CoreResult<bool> {
    let archive = tx.query_opt("SELECT archive_request_id,snapshot,reclaimed_at IS NOT NULL,restore_request_id FROM trial_runtime_archives WHERE agent_runtime_id=$1 AND restored_at IS NULL FOR UPDATE", &[&runtime.id]).await.map_err(store_error)?;
    if let Some(row) = archive {
        let id: String = row.get(0);
        let reclaimed: bool = row.get(2);
        if reclaimed {
            if !blocked && resume_allowed && row.get::<_, Option<String>>(3).is_none() {
                let snapshot: TrialArchiveSnapshot =
                    serde_json::from_value(row.get(1)).map_err(json_error)?;
                let request = enqueue_restore(tx, project, runtime, &snapshot, now).await?;
                tx.execute("UPDATE trial_runtime_archives SET restore_request_id=$2 WHERE archive_request_id=$1", &[&id,&request]).await.map_err(store_error)?;
            }
            return Ok(true);
        }
        if blocked {
            let reclaim = postgres_enqueue_runtime_control_request_bound(
                tx,
                project,
                &project.owner_user_id,
                RuntimeControlKind::ReclaimTrial,
                None,
                now,
                None,
            )
            .await?;
            tx.execute("UPDATE trial_runtime_archives SET reclaim_request_id=$2 WHERE archive_request_id=$1", &[&id,&reclaim.id]).await.map_err(store_error)?;
            return Ok(true);
        }
        // Payment won before a reclaim lease existed. Keep the off-host copy,
        // retain local state and let the existing suspension marker restart it.
        tx.execute("UPDATE trial_runtime_archives SET restored_at=$2::text::timestamptz WHERE archive_request_id=$1", &[&id,&now]).await.map_err(store_error)?;
        return Ok(false);
    }
    if blocked
        && resume_allowed
        && runtime.host_facts.runtime_status == RuntimeSummaryStatus::Offline
        && runtime.supports_runtime_control(RuntimeControlKind::ArchiveTrial)
        && std::env::var("FC_CORE_TRIAL_ARCHIVES_ENABLED").as_deref() == Ok("true")
    {
        let pinned = principal(tx, &runtime.id)
            .await?
            .ok_or(CoreError::RuntimeSpecMismatch)?;
        if !valid_agent_npub(&pinned) {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        postgres_enqueue_runtime_control_request_bound(
            tx,
            project,
            &project.owner_user_id,
            RuntimeControlKind::ArchiveTrial,
            None,
            now,
            None,
        )
        .await?;
        return Ok(true);
    }
    Ok(false)
}

pub(super) async fn store_snapshot<C: GenericClient + Sync>(
    tx: &C,
    request: &RuntimeControlRequest,
    snapshot: &TrialArchiveSnapshot,
    now: &str,
) -> CoreResult<()> {
    let runtime = select_agent_runtime(tx, &request.agent_runtime_id)
        .await?
        .ok_or(CoreError::ProjectRuntimeNotFound)?;
    let value: Value=tx.query_one("SELECT runtime_spec FROM agent_creation_requests WHERE agent_runtime_id=$1 AND runtime_spec IS NOT NULL ORDER BY created_at DESC,id DESC LIMIT 1", &[&runtime.id]).await.map_err(store_error)?.get(0);
    let spec: RuntimeSpecEnvelope = serde_json::from_value(value).map_err(json_error)?;
    let mut archive_request = request.clone();
    archive_request.kind = RuntimeControlKind::Destroy;
    validate_runtime_retirement_snapshot_receipt(
        &snapshot.receipt,
        &archive_request,
        &runtime,
        &spec,
        now,
    )?;
    if !valid_sha256_hex(&snapshot.durable_state_manifest_sha256)
        || principal(tx, &runtime.id).await?.as_deref() != Some(snapshot.agent_principal.as_str())
    {
        return Err(CoreError::RuntimeRetirementSnapshotMismatch);
    }
    let value = serde_json::to_value(snapshot).map_err(json_error)?;
    tx.execute("INSERT INTO trial_runtime_archives(archive_request_id,agent_runtime_id,snapshot,archive_lease_sha256) VALUES($1,$2,$3,$4) ON CONFLICT(archive_request_id) DO NOTHING", &[&request.id,&runtime.id,&value,&runtime_credentials::digest(request.lease_token.as_deref().ok_or(CoreError::RuntimeControlRequestLeaseConflict)?)]).await.map_err(store_error)?;
    let stored: Value = tx
        .query_one(
            "SELECT snapshot FROM trial_runtime_archives WHERE archive_request_id=$1",
            &[&request.id],
        )
        .await
        .map_err(store_error)?
        .get(0);
    if stored != value {
        return Err(CoreError::RuntimeRetirementSnapshotConflict);
    }
    Ok(())
}

async fn enqueue_restore<C: GenericClient + Sync>(
    tx: &C,
    project: &Project,
    runtime: &AgentRuntime,
    snapshot: &TrialArchiveSnapshot,
    now: &str,
) -> CoreResult<String> {
    let row=tx.query_one("SELECT id FROM agent_creation_requests WHERE agent_runtime_id=$1 AND status='running' AND runtime_spec IS NOT NULL ORDER BY created_at DESC,id DESC LIMIT 1", &[&runtime.id]).await.map_err(store_error)?;
    let mut request = locked_agent_creation_request(tx, &row.get::<_, String>(0)).await?;
    let artifact = select_runtime_artifact(tx, &snapshot.receipt.runtime_artifact_id)
        .await?
        .ok_or(CoreError::RuntimeArtifactNotFound)?;
    let id = new_agent_creation_request_id()?;
    let spec = runtime_operation_spec_v1(
        request
            .runtime_spec
            .as_ref()
            .ok_or(CoreError::RuntimeSpecMismatch)?,
        RuntimeSpecIdentity {
            operation_id: &id,
            project_id: &project.id,
            agent_runtime_id: &runtime.id,
            placement: runtime.placement.ok_or(CoreError::RuntimeSpecMismatch)?,
        },
        &artifact,
        &artifact,
        RuntimeBootIntent::Normal,
        None,
        None,
    )?;
    request.id = id.clone();
    request.idempotency_key = format!("trial-restore:{}", snapshot.receipt.request_id);
    request.runtime_spec = Some(spec);
    request.target_source_host_id = None;
    request.relocation = Some(RuntimeRelocationEnvelope::V1(RuntimeRelocationV1 {
        source_host_id: runtime.source_host_id.clone(),
        source_machine_id: runtime.source_machine_id.clone(),
        target_source_host_id: String::new(),
        expected_agent_npub: snapshot.agent_principal.clone(),
        durable_state_manifest_sha256: snapshot.durable_state_manifest_sha256.clone(),
        source_compute_absent: true,
        trial_archive: Some(snapshot.clone()),
    }));
    request.status = AgentCreationRequestStatus::Requested;
    request.runner_id = None;
    request.lease_token = None;
    request.lease_expires_at = None;
    request.failure_message = None;
    request.created_at = now.into();
    request.updated_at = now.into();
    if !upsert_agent_creation_request_row_with_conflict(
        tx,
        &request,
        AgentCreationInsertConflict::SingleFlight,
    )
    .await?
    {
        return Err(CoreError::RuntimeControlOperationConflict);
    }
    Ok(id)
}

/// Serializes archive restoration admission with billing changes; no new work
/// is authorized merely because an owner opened the dashboard.
pub(super) async fn authorize_restore<C: GenericClient + Sync>(
    tx: &C,
    request: &AgentCreationRequest,
) -> CoreResult<()> {
    if request
        .relocation
        .as_ref()
        .is_none_or(|r| r.v1().trial_archive.is_none())
    {
        return Ok(());
    }
    tx.query_one("SELECT customer_org_id FROM customer_billing_accounts WHERE customer_org_id=$1 FOR UPDATE NOWAIT", &[&request.customer_org_id]).await.map_err(store_error)?;
    // The claimant already holds the request row. NOWAIT avoids reversing
    // the reconciler's billing -> Runtime -> request lock order.
    tx.query_one(
        "SELECT id FROM agent_runtimes WHERE id=$1 FOR UPDATE NOWAIT",
        &[&request.agent_runtime_id],
    )
    .await
    .map_err(store_error)?;
    let allowed:bool=tx.query_one("SELECT NOT core_trial_access_blocked($1,clock_timestamp()) AND EXISTS(SELECT 1 FROM trial_runtime_archives a JOIN trial_runtime_suspensions s ON s.agent_runtime_id=a.agent_runtime_id WHERE a.restore_request_id=$2 AND a.reclaimed_at IS NOT NULL AND a.restored_at IS NULL AND s.resume_allowed) AND NOT EXISTS(SELECT 1 FROM runtime_control_requests WHERE agent_runtime_id=$3 AND status IN ('requested','launching','compute_up','ready'))", &[&request.customer_org_id,&request.id,&request.agent_runtime_id]).await.map_err(store_error)?.get(0);
    if !allowed {
        return Err(CoreError::BillingRequired);
    }
    Ok(())
}

impl CoreStore {
    pub async fn renew_trial_restore(
        &self,
        input: crate::RenewRuntimeControlRequestInput,
    ) -> CoreResult<()> {
        self.renew_trial_restore_mode(input, false).await
    }

    pub async fn renew_trial_restore_pause(
        &self,
        input: crate::RenewRuntimeControlRequestInput,
    ) -> CoreResult<()> {
        self.renew_trial_restore_mode(input, true).await
    }

    async fn renew_trial_restore_mode(
        &self,
        input: crate::RenewRuntimeControlRequestInput,
        pause_only: bool,
    ) -> CoreResult<()> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let request = locked_agent_creation_request(&*tx, &input.request_id).await?;
        if request
            .relocation
            .as_ref()
            .is_none_or(|r| r.v1().trial_archive.is_none())
        {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        if pause_only {
            verify_agent_creation_lease(&request, &input.runner_id, &input.lease_token)?;
            let live:bool=tx.query_one("SELECT COALESCE(lease_expires_at > clock_timestamp(),false) FROM agent_creation_requests WHERE id=$1", &[&request.id]).await.map_err(store_error)?.get(0);
            if !live {
                return Err(CoreError::AgentCreationRequestLeaseConflict);
            }
        } else {
            verify_agent_creation_lease_active(
                &*tx,
                &request,
                &input.runner_id,
                &input.lease_token,
            )
            .await?;
        }
        let seconds = input
            .lease_seconds
            .unwrap_or(crate::DEFAULT_AGENT_CREATION_LEASE_SECONDS);
        if !(1..=crate::MAX_AGENT_CREATION_LEASE_SECONDS).contains(&seconds) {
            return Err(CoreError::InvalidAgentCreationLeaseDuration);
        }
        tx.execute("UPDATE agent_creation_requests SET lease_expires_at=clock_timestamp()+make_interval(secs=>$2::double precision) WHERE id=$1", &[&request.id,&(seconds as f64)]).await.map_err(store_error)?;
        self.finish(tx).await
    }
}
