use super::*;

/// Explicit operator recovery of a completed pre-handoff relocation. Never
/// accepted by a public HTTP route or by ordinary credential provisioning.
#[derive(Clone)]
pub struct RecoverRelocatedCredential {
    pub upgrade: AdminRuntimeUpgradeExactInput,
    pub expected_owner_email: String,
    pub expected_agent_npub: String,
    pub expected_predecessor_creation_request_id: String,
    pub expected_relocation_request_id: String,
    pub confirm_relocation_credential_loss: bool,
}

impl CoreStore {
    #[tracing::instrument(skip_all, fields(operation = "runtime.recover_relocated_credential", runtime_id = input.upgrade.expected_agent_runtime_id))]
    pub async fn recover_relocated_credential(
        &self,
        input: RecoverRelocatedCredential,
    ) -> CoreResult<RuntimeControlRequest> {
        if !input.confirm_relocation_credential_loss
            || !valid_agent_npub(&input.expected_agent_npub)
        {
            return Err(rejected(
                "explicit relocation-loss attestation and Principal required",
            ));
        }
        let owner_email = normalize_owner_email(Some(&input.expected_owner_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let u = &input.upgrade;
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let assignment = tx
            .query_opt(
                "SELECT p.owner_user_id, r.runtime_artifact_id, r.host_facts,
               current_artifact.reference AS current_reference
             FROM agent_runtimes r JOIN projects p ON p.id=r.project_id
             JOIN users owner ON owner.id=p.owner_user_id
             JOIN runtime_artifacts current_artifact ON current_artifact.id=r.runtime_artifact_id
             JOIN project_runtime_links l ON l.project_id=p.id AND l.agent_runtime_id=r.id
             WHERE r.id=$1 AND p.id=$2 AND r.source_host_id=$3 AND r.source_machine_id=$4
               AND owner.normalized_email=$5 AND r.health_reporting_npub=$6 AND l.active
               AND p.import_candidate_id IS NULL AND r.offboarding_phase IS NULL
               AND r.placement_runner_class='kata'
             FOR UPDATE OF r,p,owner,l",
                &[
                    &u.expected_agent_runtime_id,
                    &u.project_id,
                    &u.expected_source_host_id,
                    &u.expected_source_machine_id,
                    &owner_email,
                    &input.expected_agent_npub,
                ],
            )
            .await
            .map_err(store_error)?
            .ok_or(CoreError::ProviderOperationIdentityMismatch)?;
        let facts: HostOwnedRuntimeFacts = serde_json::from_value(assignment.get("host_facts"))
            .map_err(|_| CoreError::RuntimeSpecMismatch)?;
        if facts.runtime_status != RuntimeSummaryStatus::Online
            || assignment
                .get::<_, Option<String>>("runtime_artifact_id")
                .as_deref()
                == Some(u.target_runtime_artifact_id.as_str())
        {
            return Err(rejected(
                "requires an online Runtime and a replacement artifact",
            ));
        }
        let target = select_runtime_artifact(&*tx, &u.target_runtime_artifact_id)
            .await?
            .ok_or(CoreError::RuntimeArtifactNotFound)?;
        if target.reference == assignment.get::<_, String>("current_reference") {
            return Err(rejected("replacement artifact must change compute"));
        }
        let owner: String = assignment.get("owner_user_id");
        let origin =
            locked_agent_creation_request(&*tx, &input.expected_predecessor_creation_request_id)
                .await?;
        let completed =
            locked_agent_creation_request(&*tx, &input.expected_relocation_request_id).await?;
        validate_lineage(&input, &owner, &origin, &completed)?;
        let relocation = completed.relocation.as_ref().unwrap().v1();
        // An exact supplied record is necessary but not sufficient: any other
        // successful relocation or active operation makes this legacy shape
        // ambiguous. Never select a winner by identifier or timestamp order.
        let unambiguous: bool = tx
            .query_one(
                "SELECT
             (SELECT count(*)=2 FROM agent_creation_requests
              WHERE agent_runtime_id=$1 AND status='running')
             AND NOT EXISTS (SELECT 1 FROM agent_creation_requests WHERE agent_runtime_id=$1
               AND status IN ('requested','launching'))
             AND NOT EXISTS (SELECT 1 FROM runtime_control_requests WHERE agent_runtime_id=$1
               AND status IN ('requested','launching','compute_up','ready'))
             AND (SELECT count(*)=1 FROM runtime_core_credentials c
               JOIN agent_creation_requests q ON q.id=c.creation_request_id
               WHERE q.agent_runtime_id=$1)",
                &[&u.expected_agent_runtime_id],
            )
            .await
            .map_err(store_error)?
            .get(0);
        if !unambiguous {
            return Err(rejected("ambiguous credential or lifecycle history"));
        }
        let predecessor = tx
            .query_opt(
                "SELECT creation_request_id FROM runtime_core_credentials
             WHERE creation_request_id=$1 AND agent_runtime_id=$2 AND owner_user_id=$3
               AND source_host_id=$4 AND source_machine_id=$5 AND revoked AND NOT activated
             FOR UPDATE",
                &[
                    &origin.id,
                    &u.expected_agent_runtime_id,
                    &owner,
                    &relocation.source_host_id,
                    &relocation.source_machine_id,
                ],
            )
            .await
            .map_err(store_error)?;
        if predecessor.is_none() {
            return Err(rejected("legacy revoked predecessor does not match"));
        }

        let secret = new_secret()?;
        tx.execute("UPDATE runtime_core_credentials SET agent_runtime_id=NULL WHERE creation_request_id=$1", &[&origin.id])
            .await.map_err(store_error)?;
        tx.execute(
            "INSERT INTO runtime_core_credentials
             (creation_request_id,agent_runtime_id,source_host_id,source_machine_id,owner_user_id,
              bootstrap_secret,token_sha256,lease_sha256,activated,
              hosted_enabled,hosted_generation,hosted_username,hosted_password,hosted_signing_secret,
              hosted_applied_generation,hosted_apply_status)
             SELECT $1,$2,$3,$4,owner_user_id,$5,$6,$7,TRUE,
               hosted_enabled,hosted_generation,hosted_username,hosted_password,hosted_signing_secret,
               NULL,'pending' FROM runtime_core_credentials WHERE creation_request_id=$8",
            &[&completed.id, &u.expected_agent_runtime_id, &u.expected_source_host_id,
              &u.expected_source_machine_id, &secret, &digest(&secret),
              &digest(&new_secret()?), &origin.id],
        ).await.map_err(store_error)?;
        // Enqueue and rotate in the same transaction. Artifact validation or
        // any enqueue error rolls back the credential changes as well.
        let now = current_time_iso()?;
        let expected = RuntimeControlExpectedBinding {
            agent_runtime_id: u.expected_agent_runtime_id.clone(),
            source_host_id: u.expected_source_host_id.clone(),
            source_machine_id: u.expected_source_machine_id.clone(),
        };
        let request = postgres_admin_request_runtime_control_bound(
            &*tx,
            AdminRuntimeControlInput {
                admin_verified_email: u.admin_verified_email.clone(),
                admin_workos_user_id: u.admin_workos_user_id.clone(),
                project_id: u.project_id.clone(),
                now: Some(now.clone()),
            },
            RuntimeControlKind::Upgrade,
            Some(u.target_runtime_artifact_id.clone()),
            Some(&expected),
        )
        .await?;
        tx.execute(
            "INSERT INTO runtime_credential_recoveries
             (runtime_control_request_id,predecessor_creation_request_id,successor_creation_request_id)
             VALUES ($1,$2,$3)", &[&request.id, &origin.id, &completed.id],
        ).await.map_err(store_error)?;
        insert_finite_private_admin_audit_event(
            &*tx,
            FinitePrivateAdminAuditInsert {
                action: "runtime.recover_relocated_credential",
                target_type: "agent_runtime",
                target_id: &u.expected_agent_runtime_id,
                grant_id: None,
                api_key_id: None,
                actor: Some(&u.admin_verified_email),
                metadata: json!({"projectId":u.project_id,"predecessorCreationRequestId":origin.id,
                "relocationRequestId":completed.id,"runtimeControlRequestId":request.id}),
                now: &now,
            },
        )
        .await?;
        self.finish(tx).await?;
        Ok(request)
    }
}

fn validate_lineage(
    input: &RecoverRelocatedCredential,
    owner: &str,
    origin: &AgentCreationRequest,
    completed: &AgentCreationRequest,
) -> CoreResult<()> {
    let u = &input.upgrade;
    for q in [origin, completed] {
        if q.status != AgentCreationRequestStatus::Running
            || q.agent_runtime_id.as_deref() != Some(u.expected_agent_runtime_id.as_str())
            || q.project_id != u.project_id
            || q.owner_user_id != owner
        {
            return Err(rejected(
                "creation lineage no longer belongs to this assignment",
            ));
        }
    }
    let relocation = completed
        .relocation
        .as_ref()
        .ok_or_else(|| rejected("completed relocation required"))?
        .v1();
    if origin.relocation.is_some()
        || origin.id == completed.id
        || completed.target_source_host_id.as_deref() != Some(u.expected_source_host_id.as_str())
        || relocation.target_source_host_id != u.expected_source_host_id
        || relocation.source_machine_id != u.expected_source_machine_id
        || relocation.expected_agent_npub != input.expected_agent_npub
    {
        return Err(rejected(
            "relocation does not prove the exact current placement",
        ));
    }
    Ok(())
}

fn rejected(check: &'static str) -> CoreError {
    tracing::warn!(check, "historical relocation credential recovery rejected");
    CoreError::ProviderOperationTransitionConflict
}

// The typed receipt is authorization, not an inference from audit text or
// historical ordering. The live request and current assignment were locked by
// the caller; recheck both credentials before permitting local replacement.
pub(super) async fn replacement_authorization(
    tx: &tokio_postgres::Transaction<'_>,
    request: &RuntimeControlRequest,
    owner: &str,
) -> CoreResult<Option<String>> {
    let Some(receipt) = tx.query_opt(
        "SELECT receipt.predecessor_creation_request_id,receipt.successor_creation_request_id
         FROM runtime_credential_recoveries receipt
         JOIN runtime_control_requests original ON original.id=receipt.runtime_control_request_id
         JOIN runtime_core_credentials successor ON successor.creation_request_id=receipt.successor_creation_request_id
         WHERE successor.agent_runtime_id=$1
           AND original.agent_runtime_id=$1 AND original.project_id=$2
           AND original.source_host_id=$3 AND original.source_machine_id=$4
           AND original.target_runtime_artifact_id=$5
           AND (original.id=$6 OR original.status='failed')",
        &[&request.agent_runtime_id,&request.project_id,&request.source_host_id,
          &request.source_machine_id,&request.target_runtime_artifact_id,&request.id],
    ).await.map_err(store_error)? else { return Ok(None); };
    let predecessor: String = receipt.get(0);
    let successor: String = receipt.get(1);
    let row = tx.query_opt(
        "SELECT old.token_sha256 FROM runtime_core_credentials old, runtime_core_credentials new
         WHERE old.creation_request_id=$1 AND new.creation_request_id=$2
           AND old.revoked AND NOT old.activated AND old.agent_runtime_id IS NULL
           AND old.owner_user_id=$3 AND new.owner_user_id=$3
           AND new.agent_runtime_id=$4 AND new.source_host_id=$5 AND new.source_machine_id=$6
           AND new.activated AND NOT new.revoked
         FOR UPDATE OF old,new",
        &[&predecessor,&successor,&owner,&request.agent_runtime_id,
          &request.source_host_id,&request.source_machine_id],
    ).await.map_err(store_error)?.ok_or_else(|| rejected("recovery authorization no longer matches"))?;
    // Waiting for predecessor locks must not extend delivery authority.
    let live: bool = tx.query_one(
        "SELECT COALESCE(lease_expires_at>clock_timestamp(),FALSE) FROM runtime_control_requests WHERE id=$1",
        &[&request.id],
    ).await.map_err(store_error)?.get(0);
    if !live {
        return Err(CoreError::RuntimeControlRequestLeaseConflict);
    }
    Ok(Some(row.get(0)))
}
