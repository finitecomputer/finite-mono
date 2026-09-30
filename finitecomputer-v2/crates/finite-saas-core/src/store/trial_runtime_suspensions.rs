use super::*;
#[cfg(test)]
mod tests;

impl CoreStore {
    /// Reconcile deadlines without depending on a webhook or a dashboard visit.
    /// Each org uses its own short transaction; a broken runtime cannot prevent
    /// unrelated accounts from being suspended or restored.
    pub async fn reconcile_trial_runtimes(&self, now: Option<&str>) -> CoreResult<()> {
        let now = now.map(str::to_owned).unwrap_or(current_time_iso()?);
        let mut after = String::new();
        loop {
            let rows = self
                .connection()
                .await?
                .query(
                    // Only orgs with work to do: currently blocked, or holding a
                    // suspension marker that may need a resume. Converted, paid
                    // orgs without markers are not locked on every sweep.
                    "SELECT t.customer_org_id FROM trial_redemptions t
                 WHERE t.state = 'redeemed' AND t.customer_org_id > $1
                   AND (core_trial_access_blocked(t.customer_org_id, $2::text::timestamptz)
                        OR EXISTS (
                            SELECT 1 FROM trial_runtime_suspensions s
                            JOIN agent_runtimes r ON r.id = s.agent_runtime_id
                            JOIN projects p ON p.id = r.project_id
                            WHERE p.customer_org_id = t.customer_org_id))
                 ORDER BY t.customer_org_id LIMIT 100",
                    &[&after, &now],
                )
                .await
                .map_err(store_error)?;
            if rows.is_empty() {
                return Ok(());
            }
            for row in rows {
                let org: String = row.get(0);
                after.clone_from(&org);
                if let Err(error) = self.reconcile_trial_org(&org, &now).await {
                    tracing::error!(operation = "reconcile_trial_runtimes", org_id = %org,
                        error = %error, "trial runtime reconciliation failed; will retry");
                }
            }
        }
    }

    #[tracing::instrument(skip(self), fields(operation = "reconcile_trial_org"))]
    pub(super) async fn reconcile_trial_org(&self, org: &str, now: &str) -> CoreResult<()> {
        let mut client = self.connection().await?;
        let mut tx = client.transaction().await.map_err(store_error)?;
        // Serialize with accepted billing updates and other Core replicas. We
        // always derive desired access from persisted billing, never the event
        // payload that happened to wake this reconciliation.
        if billing::select_customer_billing_account(&*tx, org, true)
            .await?
            .is_none()
        {
            tx.query_opt(
                "SELECT id FROM customer_orgs WHERE id = $1 FOR UPDATE",
                &[&org],
            )
            .await
            .map_err(store_error)?;
        }
        let Some(access) = trials_access::trial_access(&*tx, org, now).await? else {
            return Ok(());
        };
        let blocked = access.blocked;
        let rows = tx
            .query(
                "SELECT id FROM projects WHERE customer_org_id = $1 ORDER BY id",
                &[&org],
            )
            .await
            .map_err(store_error)?;
        for row in rows {
            let project = select_project(&*tx, &row.get::<_, String>(0))
                .await?
                .ok_or(CoreError::ProjectNotFound)?;
            // Isolate each runtime: an unsupported capability, lock contention
            // or other failure on one agent must not stop its siblings from
            // being suspended or restored. The next sweep retries it.
            let savepoint = tx.savepoint("trial_runtime").await.map_err(store_error)?;
            match reconcile_runtime(&*savepoint, &project, blocked, now).await {
                Ok(()) => savepoint.commit().await.map_err(store_error)?,
                Err(error) => {
                    tracing::error!(operation = "reconcile_trial_runtime", project_id = %project.id,
                        error = %error, "trial runtime reconciliation failed; will retry");
                    savepoint.rollback().await.map_err(store_error)?;
                }
            }
        }
        self.finish(tx).await
    }
}

async fn reconcile_runtime<C: GenericClient + Sync>(
    tx: &C,
    project: &Project,
    blocked: bool,
    now: &str,
) -> CoreResult<()> {
    let Some(runtime) = postgres_active_runtime_for_project(tx, &project.id).await? else {
        return Ok(());
    };
    let marker = tx.query_opt(
        "SELECT s.stop_request_id, stop.status, s.resume_request_id, resume.status, s.resume_allowed
         FROM trial_runtime_suspensions s
         JOIN runtime_control_requests stop ON stop.id = s.stop_request_id
         LEFT JOIN runtime_control_requests resume ON resume.id = s.resume_request_id
         WHERE s.agent_runtime_id = $1", &[&runtime.id],
    ).await.map_err(store_error)?;
    let active = tx.query_opt(
        "SELECT id, kind, status, (lease_expires_at IS NULL OR lease_expires_at <= $2::text::timestamptz) AS expired FROM runtime_control_requests WHERE agent_runtime_id = $1
         AND status IN ('requested', 'launching', 'compute_up', 'ready') FOR UPDATE NOWAIT",
        &[&runtime.id, &now],
    ).await.map_err(store_error)?;
    let mut uncertain_compute = false;
    if let Some(active) = active {
        if blocked
            && (active.get::<_, String>(2) == "requested"
                || (active.get::<_, String>(2) == "launching" && active.get::<_, bool>(3)))
            && !matches!(active.get::<_, String>(1).as_str(), "stop" | "destroy")
        {
            // The provider may already have started compute, even if Core's
            // last confirmed lifecycle latch still says Offline.
            uncertain_compute = active.get::<_, String>(2) == "launching";
            // A queued up-bound control must not hold the lifecycle slot forever
            // after access expires. Expired leases cannot complete; their old
            // tokens remain fenced by the terminal status. Never invalidate a
            // Runner's live lease.
            tx.execute(
                "UPDATE runtime_control_requests SET status = 'failed', failure_stage = 'launch',
                 failure_message = 'Trial access requires payment',
                 updated_at = $2::text::timestamptz, completed_at = $2::text::timestamptz
                 WHERE id = $1",
                &[&active.get::<_, String>(0), &now],
            )
            .await
            .map_err(store_error)?;
        } else {
            return Ok(());
        }
    }
    if blocked {
        if runtime.host_facts.runtime_status == RuntimeSummaryStatus::Offline && !uncertain_compute
        {
            return Ok(());
        }
        if !retry_backoff_elapsed(tx, &runtime.id, "stop", now).await? {
            return Ok(());
        }
        let stop = postgres_enqueue_runtime_control_request_bound(
            tx,
            project,
            &project.owner_user_id,
            RuntimeControlKind::Stop,
            None,
            now,
            None,
        )
        .await?;
        tx.execute(
            "INSERT INTO trial_runtime_suspensions (agent_runtime_id, stop_request_id)
             VALUES ($1, $2) ON CONFLICT (agent_runtime_id) DO UPDATE
             SET stop_request_id = EXCLUDED.stop_request_id, resume_request_id = NULL",
            &[&runtime.id, &stop.id],
        )
        .await
        .map_err(store_error)?;
    } else if let Some(marker) = marker {
        if !marker.get::<_, bool>(4) {
            return Ok(());
        }
        let stop_status: String = marker.get(1);
        let resume_status: Option<String> = marker.get(3);
        if resume_status.as_deref() == Some("succeeded") {
            tx.execute(
                "DELETE FROM trial_runtime_suspensions WHERE agent_runtime_id = $1",
                &[&runtime.id],
            )
            .await
            .map_err(store_error)?;
        } else if matches!(stop_status.as_str(), "stopped" | "failed")
            && retry_backoff_elapsed(tx, &runtime.id, "restart", now).await?
        {
            // Restart uses the existing RuntimeSpec, identity, handle and durable
            // mount. No agent-creation request, key rotation or data deletion.
            let resume = postgres_enqueue_runtime_control_request_bound(
                tx,
                project,
                &project.owner_user_id,
                RuntimeControlKind::Restart,
                None,
                now,
                None,
            )
            .await?;
            tx.execute("UPDATE trial_runtime_suspensions SET resume_request_id = $2 WHERE agent_runtime_id = $1",
                &[&runtime.id, &resume.id]).await.map_err(store_error)?;
        }
    }
    Ok(())
}

/// Failed enforcement and recovery operations retry through the same control
/// path, but not on every five-second sweep. Only consecutive failures since
/// the last non-failed operation of that kind count. The first retry is
/// immediate; later ones back off exponentially from 15 seconds to a 30 minute
/// ceiling. The reconciler's own fencing of queued controls is not an
/// operation failure.
async fn retry_backoff_elapsed<C: GenericClient + Sync>(
    tx: &C,
    runtime_id: &str,
    kind: &str,
    now: &str,
) -> CoreResult<bool> {
    let row = tx
        .query_one(
            "SELECT count(*) <= 1 OR max(failed.completed_at)
                    + LEAST(interval '15 seconds' * power(2, LEAST(count(*) - 2, 7)),
                            interval '30 minutes') <= $3::text::timestamptz
             FROM runtime_control_requests failed
             WHERE failed.agent_runtime_id = $1 AND failed.kind = $2 AND failed.status = 'failed'
               AND failed.failure_message IS DISTINCT FROM 'Trial access requires payment'
               AND failed.created_at > COALESCE((
                   SELECT max(settled.created_at) FROM runtime_control_requests settled
                   WHERE settled.agent_runtime_id = $1 AND settled.kind = $2
                     AND settled.status <> 'failed'
               ), '-infinity'::timestamptz)",
            &[&runtime_id, &kind, &now],
        )
        .await
        .map_err(store_error)?;
    Ok(row.get(0))
}
