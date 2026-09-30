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
                    "SELECT customer_org_id FROM trial_redemptions
                 WHERE state = 'redeemed' AND customer_org_id > $1
                 ORDER BY customer_org_id LIMIT 100",
                    &[&after],
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
        let tx = client.transaction().await.map_err(store_error)?;
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
            reconcile_runtime(&*tx, &project, blocked, now).await?;
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
        } else if matches!(stop_status.as_str(), "stopped" | "failed") {
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
