use super::*;

impl CoreStore {
    pub async fn issue_launch_code_batch(
        &self,
        input: IssueLaunchCodeBatchInput,
    ) -> CoreResult<IssuedLaunchCodeBatch> {
        let prepared = prepare_launch_code_batch(input)?;
        let response = IssuedLaunchCodeBatch {
            batch: prepared.batch.clone(),
            codes: prepared.issued_codes,
        };
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let code_count = i32::try_from(prepared.batch.code_count)
            .map_err(|_| CoreError::InvalidLaunchCodeBatchSize)?;
        tx.execute(
            "INSERT INTO launch_code_batches
               (id, name, hosting_tier, code_count, expires_at, revoked_at,
                revoked_by_workos_user_id, created_by_workos_user_id, created_at)
             VALUES ($1, $2, $3, $4, $5::text::timestamptz, NULL, NULL, $6,
                     $7::text::timestamptz)",
            &[
                &prepared.batch.id,
                &prepared.batch.name,
                &prepared.batch.hosting_tier.map(HostingTier::as_str),
                &code_count,
                &prepared.batch.expires_at,
                &prepared.batch.created_by_workos_user_id,
                &prepared.batch.created_at,
            ],
        )
        .await
        .map_err(store_error)?;
        for record in prepared.records {
            tx.execute(
                "INSERT INTO launch_codes
                   (id, batch_id, code_hash, redeemed_customer_org_id,
                    redemption_idempotency_key, redeemed_at, created_at)
                 VALUES ($1, $2, $3, NULL, NULL, NULL, $4::text::timestamptz)",
                &[
                    &record.id,
                    &record.batch_id,
                    &record.code_hash,
                    &record.created_at,
                ],
            )
            .await
            .map_err(store_error)?;
        }
        self.finish(tx).await?;
        Ok(response)
    }

    /// Root-only operator command: bind an unused code issued by the named
    /// existing operator. No user/project/runtime row is rewritten.
    pub async fn target_launch_code_exact(
        &self,
        code_id: &str,
        expected_batch_id: &str,
        source_host_id: &str,
        operator_email: &str,
        operator_workos_user_id: &str,
    ) -> CoreResult<()> {
        let host = normalize_source_host_id(source_host_id)?;
        let email =
            normalize_owner_email(Some(operator_email)).ok_or(CoreError::MissingVerifiedEmail)?;
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let operator = select_user_by_email(&*tx, &email)
            .await?
            .filter(|user| user.workos_user_id.as_deref() == Some(operator_workos_user_id))
            .ok_or(CoreError::WorkosUserConflict)?;
        lock_unused_standard_launch_code(&*tx, code_id, expected_batch_id, operator_workos_user_id)
            .await?;
        let existing = tx
            .query_opt(
                "SELECT source_host_id FROM launch_code_host_targets WHERE launch_code_id = $1",
                &[&code_id],
            )
            .await
            .map_err(store_error)?;
        if let Some(existing) = existing {
            if existing.get::<_, String>("source_host_id") != host {
                return Err(CoreError::RuntimeSpecMismatch);
            }
        } else {
            // This command admits an empty qualification host only. A unique
            // host index serializes competing reservations for different codes.
            let occupied: bool = tx
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM agent_runtimes WHERE source_host_id = $1)",
                    &[&host],
                )
                .await
                .map_err(store_error)?
                .get(0);
            if occupied {
                return Err(CoreError::RuntimeSpecMismatch);
            }
            tx.execute(
                "INSERT INTO launch_code_host_targets
                 (launch_code_id, source_host_id, created_by_workos_user_id, created_at)
                 VALUES ($1, $2, $3, CURRENT_TIMESTAMP)",
                &[&code_id, &host, &operator_workos_user_id],
            )
            .await
            .map_err(store_error)?;
            insert_finite_private_admin_audit_event(&*tx, FinitePrivateAdminAuditInsert {
                action: "launch_code.target_host",
                target_type: "launch_code",
                target_id: code_id,
                grant_id: None,
                api_key_id: None,
                actor: Some(&email),
                metadata: serde_json::json!({"sourceHostId":host,"batchId":expected_batch_id,"operatorUserId":operator.id}),
                now: &current_time_iso()?,
            }).await?;
        }
        self.finish(tx).await
    }

    /// Preserve the original binding and append one retry after an exact,
    /// completed, untargeted misplacement. Never reset entitlement or runtime state.
    pub async fn retry_targeted_launch_code_exact(
        &self,
        input: &crate::RetryTargetedLaunchCodeInput,
    ) -> CoreResult<()> {
        let host = normalize_source_host_id(&input.target_source_host_id)?;
        let previous_host = normalize_source_host_id(&input.expected_previous_source_host_id)?;
        if host == previous_host || input.code_id == input.previous_code_id {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        let email = normalize_owner_email(Some(&input.operator_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let operator = select_user_by_email(&*tx, &email)
            .await?
            .filter(|u| u.workos_user_id.as_deref() == Some(input.operator_workos_user_id.as_str()))
            .ok_or(CoreError::WorkosUserConflict)?;
        // Lock the original code, reservation, request and Runtime while checking
        // the failure record. Competing retries serialize on the parent binding.
        let previous = tx.query_opt(
            "SELECT target.launch_code_id
             FROM launch_code_host_targets target
             JOIN launch_codes code ON code.id=target.launch_code_id
             JOIN launch_code_batches batch ON batch.id=code.batch_id
             JOIN agent_creation_requests request ON request.requested_launch_code=code.id
             JOIN agent_runtimes runtime ON runtime.id=request.agent_runtime_id
             WHERE code.id=$1 AND target.source_host_id=$2
               AND target.retry_of_launch_code_id IS NULL AND target.cohort_of_launch_code_id IS NULL
               AND batch.created_by_workos_user_id=$3
               AND batch.code_count=1 AND COALESCE(batch.hosting_tier, 'standard')='standard'
               AND target.created_by_workos_user_id=$3
               AND request.id=$4 AND request.project_id=$5 AND runtime.project_id=$5
               AND runtime.id=$6 AND runtime.source_host_id=$7
               AND request.owner_user_id=$8
               AND code.redeemed_at IS NOT NULL
               AND code.redeemed_customer_org_id=request.customer_org_id
               AND code.redemption_idempotency_key=request.idempotency_key
               AND request.status='running' AND request.target_source_host_id IS NULL
               AND EXISTS(SELECT 1 FROM project_runtime_links link WHERE link.project_id=$5 AND link.agent_runtime_id=$6 AND link.active)
               AND (SELECT count(*) FROM agent_creation_requests r WHERE r.requested_launch_code=code.id)=1
             FOR UPDATE OF target, code, batch, request, runtime",
            &[&input.previous_code_id, &host, &input.operator_workos_user_id,
              &input.expected_previous_request_id, &input.expected_previous_project_id,
              &input.expected_previous_runtime_id, &previous_host, &operator.id],
        ).await.map_err(store_error)?;
        if previous.is_none() {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        let blocked: bool = tx.query_one(
            "SELECT EXISTS(SELECT 1 FROM launch_host_reservation_releases WHERE source_host_id=$1)
                OR EXISTS(SELECT 1 FROM agent_runtimes WHERE source_host_id=$1)
                OR EXISTS(SELECT 1 FROM agent_creation_requests WHERE (target_source_host_id=$1 OR agent_runtime_id=$2) AND status IN ('requested','launching'))
                OR EXISTS(SELECT 1 FROM runtime_control_requests WHERE agent_runtime_id=$2 AND status IN ('requested','launching','compute_up','ready'))",
            &[&host, &input.expected_previous_runtime_id],
        ).await.map_err(store_error)?.get(0);
        if blocked {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        lock_unused_standard_launch_code(
            &*tx,
            &input.code_id,
            &input.expected_batch_id,
            &input.operator_workos_user_id,
        )
        .await?;
        let existing = tx.query_opt(
            "SELECT launch_code_id, source_host_id, retry_of_launch_code_id FROM launch_code_host_targets
             WHERE launch_code_id=$1 OR retry_of_launch_code_id=$2",
            &[&input.code_id, &input.previous_code_id],
        ).await.map_err(store_error)?;
        if let Some(existing) = existing {
            if existing.get::<_, String>("launch_code_id") != input.code_id
                || existing.get::<_, String>("source_host_id") != host
                || existing
                    .get::<_, Option<String>>("retry_of_launch_code_id")
                    .as_deref()
                    != Some(input.previous_code_id.as_str())
            {
                return Err(CoreError::RuntimeSpecMismatch);
            }
        } else {
            tx.execute(
                "INSERT INTO launch_code_host_targets
                 (launch_code_id, source_host_id, created_by_workos_user_id, created_at, retry_of_launch_code_id)
                 VALUES ($1,$2,$3,CURRENT_TIMESTAMP,$4)",
                &[&input.code_id, &host, &input.operator_workos_user_id, &input.previous_code_id],
            ).await.map_err(store_error)?;
            insert_finite_private_admin_audit_event(
                &*tx,
                FinitePrivateAdminAuditInsert {
                    action: "launch_code.retry_target_host",
                    target_type: "launch_code",
                    target_id: &input.code_id,
                    grant_id: None,
                    api_key_id: None,
                    actor: Some(&email),
                    metadata: serde_json::to_value(input).map_err(json_error)?,
                    now: &current_time_iso()?,
                },
            )
            .await?;
        }
        self.finish(tx).await
    }

    /// Append a release receipt; targeting rows and all user state stay intact.
    /// The operator keeps the Runner drained until postflight verifies the receipt.
    pub async fn release_launch_host_exact(
        &self,
        input: &crate::ReleaseLaunchHostInput,
    ) -> CoreResult<()> {
        let host = normalize_source_host_id(&input.source_host_id)?;
        let email = normalize_owner_email(Some(&input.operator_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        select_user_by_email(&*tx, &email)
            .await?
            .filter(|user| {
                user.workos_user_id.as_deref() == Some(input.operator_workos_user_id.as_str())
            })
            .ok_or(CoreError::WorkosUserConflict)?;
        let root = tx
            .query_opt(
                "SELECT launch_code_id FROM launch_code_host_targets
             WHERE launch_code_id=$1 AND source_host_id=$2 AND created_by_workos_user_id=$3
               AND retry_of_launch_code_id IS NULL AND cohort_of_launch_code_id IS NULL
             FOR UPDATE",
                &[
                    &input.reservation_code_id,
                    &host,
                    &input.operator_workos_user_id,
                ],
            )
            .await
            .map_err(store_error)?;
        if root.is_none() {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        if let Some(release) = tx
            .query_opt(
                "SELECT reservation_code_id, canary_runtime_id, released_by_workos_user_id
             FROM launch_host_reservation_releases WHERE source_host_id=$1",
                &[&host],
            )
            .await
            .map_err(store_error)?
        {
            if release.get::<_, String>("reservation_code_id") != input.reservation_code_id
                || release.get::<_, String>("canary_runtime_id") != input.expected_canary_runtime_id
                || release.get::<_, String>("released_by_workos_user_id")
                    != input.operator_workos_user_id
            {
                return Err(CoreError::RuntimeSpecMismatch);
            }
            return self.finish(tx).await;
        }
        // Lock every code and batch so redemption/revocation cannot invalidate
        // the check. Do not open the pool with a usable qualification code left.
        let codes = tx
            .query(
                "SELECT code.redeemed_at IS NOT NULL OR batch.revoked_at IS NOT NULL
                    OR batch.expires_at<=CURRENT_TIMESTAMP AS unusable
             FROM launch_code_host_targets target
             JOIN launch_codes code ON code.id=target.launch_code_id
             JOIN launch_code_batches batch ON batch.id=code.batch_id
             WHERE target.source_host_id=$1 ORDER BY code.id FOR UPDATE OF code, batch",
                &[&host],
            )
            .await
            .map_err(store_error)?;
        if codes.iter().any(|row| !row.get::<_, bool>("unusable")) {
            return Err(CoreError::InvalidLaunchCode);
        }
        let canary = tx.query_opt(
            "SELECT runtime.id FROM agent_runtimes runtime
             JOIN agent_creation_requests request ON request.agent_runtime_id=runtime.id
             JOIN launch_code_host_targets target ON target.launch_code_id=request.requested_launch_code
             WHERE runtime.id=$1 AND runtime.source_host_id=$2
               AND request.status='running' AND request.target_source_host_id=$2
               AND request.project_id=runtime.project_id AND target.source_host_id=$2
               AND (target.launch_code_id=$3 OR target.retry_of_launch_code_id=$3)
               AND runtime.health_ready=true
               AND runtime.health_report_interval_seconds>0
               AND runtime.health_reported_at BETWEEN
                   CURRENT_TIMESTAMP - (3 * runtime.health_report_interval_seconds) * INTERVAL '1 second'
                   AND CURRENT_TIMESTAMP
               AND EXISTS(SELECT 1 FROM project_runtime_links link
                   WHERE link.project_id=runtime.project_id AND link.agent_runtime_id=runtime.id AND link.active)
             FOR UPDATE OF runtime, request", &[&input.expected_canary_runtime_id, &host, &input.reservation_code_id],
        ).await.map_err(store_error)?;
        if canary.is_none() {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        let pending: bool = tx
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM agent_creation_requests WHERE target_source_host_id=$1
                AND status IN ('requested','launching'))
                OR EXISTS(SELECT 1 FROM runtime_control_requests WHERE agent_runtime_id=$2
                AND status IN ('requested','launching','compute_up','ready'))",
                &[&host, &input.expected_canary_runtime_id],
            )
            .await
            .map_err(store_error)?
            .get(0);
        if pending {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        tx.execute(
            "INSERT INTO launch_host_reservation_releases
             (source_host_id,reservation_code_id,canary_runtime_id,released_by_workos_user_id,released_at)
             VALUES ($1,$2,$3,$4,CURRENT_TIMESTAMP)",
            &[&host, &input.reservation_code_id, &input.expected_canary_runtime_id, &input.operator_workos_user_id],
        ).await.map_err(store_error)?;
        insert_finite_private_admin_audit_event(
            &*tx,
            FinitePrivateAdminAuditInsert {
                action: "launch_host.release_reservation",
                target_type: "source_host",
                target_id: &host,
                grant_id: None,
                api_key_id: None,
                actor: Some(&email),
                metadata: serde_json::to_value(input).map_err(json_error)?,
                now: &current_time_iso()?,
            },
        )
        .await?;
        self.finish(tx).await
    }

    pub async fn list_launch_code_batches(&self) -> CoreResult<Vec<LaunchCodeBatchDetails>> {
        let client = self.connection().await?;
        postgres_list_launch_code_batches(&**client).await
    }

    pub async fn revoke_launch_code_batch(
        &self,
        input: RevokeLaunchCodeBatchInput,
    ) -> CoreResult<LaunchCodeBatchDetails> {
        let actor = input.revoked_by_workos_user_id.trim();
        if actor.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let now = input.now.unwrap_or(current_time_iso()?);
        parse_time(&now)?;
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let row = tx
            .query_opt(
                "UPDATE launch_code_batches
                    SET revoked_at = COALESCE(revoked_at, $2::text::timestamptz),
                        revoked_by_workos_user_id = COALESCE(revoked_by_workos_user_id, $3)
                  WHERE id = $1
                  RETURNING id, name, hosting_tier, code_count, core_rfc3339(expires_at) AS expires_at,
                            core_rfc3339(revoked_at) AS revoked_at, revoked_by_workos_user_id,
                            created_by_workos_user_id, core_rfc3339(created_at) AS created_at",
                &[&input.batch_id.trim(), &now, &actor],
            )
            .await
            .map_err(store_error)?
            .ok_or(CoreError::LaunchCodeBatchNotFound)?;
        let batch = launch_code_batch_from_row(&row)?;
        let details = postgres_launch_code_batch_details(&*tx, batch).await?;
        self.finish(tx).await?;
        Ok(details)
    }
}
