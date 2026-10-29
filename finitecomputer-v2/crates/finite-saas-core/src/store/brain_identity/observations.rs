//! One transaction for trusted key observations, with optional legacy Brain
//! participation context. Hosted registration never fabricates that context.
use super::*;
use crate::brain_identity::hosted::HostedAccountObservationRequest;

struct AccountObservation<'a> {
    operation_id: &'a str,
    observed_at: &'a str,
    payload_sha256: String,
    human_key: Option<&'a str>,
    brain: Option<&'a BrainAccountObservationRequest>,
}

impl CoreStore {
    pub async fn record_brain_account_observation(
        &self,
        workos_user_id: &str,
        issuer: &str,
        request: &BrainAccountObservationRequest,
        now: time::OffsetDateTime,
    ) -> Result<ObservationOutcome, BrainObservationError> {
        self.record_account_observation(
            workos_user_id,
            issuer,
            AccountObservation {
                operation_id: &request.operation_id,
                observed_at: &request.observed_at,
                payload_sha256: request.payload_sha256(),
                human_key: request.observed_human_public_key_hex.as_deref(),
                brain: Some(request),
            },
            now,
        )
        .await
    }

    /// An existing Hosted Device key observed for a verified Account. Reuses
    /// the association constraints and receipt namespace of Brain actions;
    /// the distinct versioned payload prevents cross-operation replay.
    pub async fn record_hosted_account_observation(
        &self,
        workos_user_id: &str,
        issuer: &str,
        request: &HostedAccountObservationRequest,
        now: time::OffsetDateTime,
    ) -> Result<ObservationOutcome, BrainObservationError> {
        self.record_account_observation(
            workos_user_id,
            issuer,
            AccountObservation {
                operation_id: &request.operation_id,
                observed_at: &request.observed_at,
                payload_sha256: request.payload_sha256(),
                human_key: Some(&request.public_key_hex),
                brain: None,
            },
            now,
        )
        .await
    }

    /// Atomically record one trusted hosted observation: the human key
    /// association (when present), the optional account/Brain sharing scope and the
    /// idempotency receipt. An exact retry returns the stored outcome without
    /// re-checking freshness or re-applying any write.
    async fn record_account_observation(
        &self,
        workos_user_id: &str,
        issuer: &str,
        request: AccountObservation<'_>,
        now: time::OffsetDateTime,
    ) -> Result<ObservationOutcome, BrainObservationError> {
        let payload_sha256 = request.payload_sha256;
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.batch_execute("SET LOCAL statement_timeout = '5s'")
            .await
            .map_err(store_error)?;
        // Resolve an existing linked account only; no user, personal org or
        // link is created or changed here. The row lock (scoped to this
        // account) serializes its observations, so a concurrent duplicate of
        // one operation reads the first attempt's receipt instead of failing
        // on its unique constraint. It does not block key-share readers.
        let user_id: String = tx
            .query_opt(
                "SELECT id FROM users
                 WHERE workos_user_id = $1 AND link_status = 'linked'
                 FOR NO KEY UPDATE",
                &[&workos_user_id],
            )
            .await
            .map_err(store_error)?
            .ok_or(BrainObservationError::AccountNotLinked)?
            .get("id");
        let user_id = user_id.as_str();

        if let Some(receipt) = tx
            .query_opt(
                "SELECT issuer, payload_sha256, outcome FROM brain_account_observation_receipts
                 WHERE user_id = $1 AND operation_id = $2",
                &[&user_id, &request.operation_id],
            )
            .await
            .map_err(store_error)?
        {
            tx.rollback().await.map_err(store_error)?;
            let stored_issuer: String = receipt.get("issuer");
            let stored_payload: String = receipt.get("payload_sha256");
            if stored_issuer != issuer || stored_payload != payload_sha256 {
                return Err(BrainObservationError::OperationReused);
            }
            let outcome: String = receipt.get("outcome");
            return ObservationOutcome::parse(&outcome).ok_or_else(|| {
                CoreError::Store("stored Brain observation outcome is invalid".to_string()).into()
            });
        }

        let observed = parse_time(request.observed_at)?;
        let age = now - observed;
        if age > time::Duration::seconds(OBSERVATION_WINDOW_SECONDS + OBSERVATION_SKEW_SECONDS)
            || -age > time::Duration::seconds(OBSERVATION_SKEW_SECONDS)
        {
            return Err(BrainObservationError::Stale);
        }
        let now_text = now
            .format(&Rfc3339)
            .map_err(|_| CoreError::InvalidTimestamp)?;

        if let Some(brain) = request
            .brain
            .filter(|brain| brain.action_kind == ObservationActionKind::OwnedAgentHostedAction)
        {
            let npub = npub_for_hex(&brain.participating_public_key_hex)
                .ok_or(BrainObservationError::AgentNotOwned)?;
            let rows = agent_pin_rows(&*tx, &[npub]).await?;
            let refs = rows.iter().collect::<Vec<_>>();
            let owned_and_live = matches!(
                agent_evidence(&refs),
                AgentEvidence::Agent {
                    pin,
                    lifecycle: Some(AgentLifecycle::Active | AgentLifecycle::Offboarding),
                    ..
                } if pin.owner_user_id == user_id
            );
            if !owned_and_live {
                return Err(BrainObservationError::AgentNotOwned);
            }
        }

        let mut changed = false;
        if let Some(human_key) = request.human_key {
            let npub = npub_for_hex(human_key).ok_or_else(|| {
                CoreError::Store("validated human key did not encode".to_string())
            })?;
            if !agent_pin_rows(&*tx, &[npub]).await?.is_empty() {
                return Err(BrainObservationError::KeyIsAgentRecord);
            }
            let association_id = generate_surrogate_id("account_key")?;
            let inserted = tx
                .query_opt(
                    "INSERT INTO account_brain_principals (
                       id, user_id, public_key_hex, source, issuer, status,
                       first_observed_at, last_observed_at, revision
                     ) VALUES ($1, $2, $3, 'hosted_device_observation', $4, 'active',
                               $5::text::timestamptz, $5::text::timestamptz, 1)
                     ON CONFLICT (public_key_hex) WHERE status = 'active' DO NOTHING
                     RETURNING id",
                    &[&association_id, &user_id, &human_key, &issuer, &now_text],
                )
                .await
                .map_err(store_error)?;
            if inserted.is_some() {
                changed = true;
            } else {
                // The conflicting active row may in principle have changed
                // since the insert; anything but this account's own active
                // association is refused.
                let owner: Option<String> = tx
                    .query_opt(
                        "SELECT user_id FROM account_brain_principals
                         WHERE public_key_hex = $1 AND status = 'active'",
                        &[&human_key],
                    )
                    .await
                    .map_err(store_error)?
                    .map(|row| row.get("user_id"));
                if owner.as_deref() != Some(user_id) {
                    return Err(BrainObservationError::KeyAssociatedElsewhere);
                }
                // Refreshes evidence time only; `revision` names the
                // association itself, which is unchanged.
                tx.execute(
                    "UPDATE account_brain_principals
                     SET last_observed_at = GREATEST(last_observed_at, $3::text::timestamptz)
                     WHERE public_key_hex = $1 AND user_id = $2 AND status = 'active'",
                    &[&human_key, &user_id, &now_text],
                )
                .await
                .map_err(store_error)?;
            }
        }

        if let Some(brain) = request.brain {
            let scope_id = generate_surrogate_id("brain_scope")?;
            let scope_inserted = tx
                .execute(
                    "INSERT INTO account_brain_sharing_scopes (
                       id, user_id, brain_server, brain_id, issuer, established_at, revoked_at, revision
                     ) VALUES ($1, $2, $3, $4, $5, $6::text::timestamptz, NULL, 1)
                     ON CONFLICT (user_id, brain_server, brain_id) WHERE revoked_at IS NULL DO NOTHING",
                    &[
                        &scope_id,
                        &user_id,
                        &brain.brain_server,
                        &brain.brain_id,
                        &issuer,
                        &now_text,
                    ],
                )
                .await
                .map_err(store_error)?;
            changed |= scope_inserted == 1;
        }

        let outcome = if changed {
            ObservationOutcome::Recorded
        } else {
            ObservationOutcome::Unchanged
        };
        let receipt_id = generate_surrogate_id("brain_observation")?;
        tx.execute(
            "INSERT INTO brain_account_observation_receipts (
               id, user_id, operation_id, issuer, payload_sha256, outcome, recorded_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7::text::timestamptz)",
            &[
                &receipt_id,
                &user_id,
                &request.operation_id,
                &issuer,
                &payload_sha256,
                &outcome.as_str(),
                &now_text,
            ],
        )
        .await
        .map_err(store_error)?;
        self.finish(tx).await?;
        Ok(outcome)
    }
}
