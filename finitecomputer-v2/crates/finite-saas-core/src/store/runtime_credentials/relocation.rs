//! A relocation prepares a successor, then atomically transfers authority at
//! completion. The predecessor remains auditable and can never authenticate again.
use super::*;

impl CoreStore {
    /// Only a live, exact relocation lease can receive the successor secret.
    /// Unenrolled runtimes remain unenrolled; this is not a credential repair API.
    #[tracing::instrument(skip_all, fields(creation_request_id = input.creation_request_id))]
    pub async fn provision_relocation_credential(
        &self,
        input: ProvisionRuntimeCredential,
    ) -> CoreResult<Option<RuntimeBootstrapCredential>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let request = locked_agent_creation_request(&*tx, &input.creation_request_id).await?;
        verify_agent_creation_lease_active(&*tx, &request, &input.runner_id, &input.lease_token)
            .await?;
        let relocation = request
            .relocation
            .as_ref()
            .ok_or(CoreError::RuntimeSpecMismatch)?
            .v1();
        if input.source_host_id != relocation.target_source_host_id
            || request.target_source_host_id.as_deref() != Some(input.source_host_id.as_str())
        {
            return Err(CoreError::ProviderOperationIdentityMismatch);
        }
        let runtime = lock_assignment(
            &*tx,
            &request,
            &relocation.source_host_id,
            &relocation.source_machine_id,
        )
        .await?;
        let existing_runtime = select_agent_runtime(&*tx, &runtime).await?;
        validate_runtime_relocation_registration(
            &request,
            existing_runtime.as_ref(),
            &relocation.target_source_host_id,
            &relocation.source_machine_id,
        )?;
        let handoff = lock_handoff(&*tx, &runtime, &request).await?;
        ensure_live_now(&*tx, &request.id).await?;
        let Some((_, pending)) = handoff else {
            self.finish(tx).await?;
            return Ok(None);
        };
        let lease_hash = digest(
            request
                .lease_token
                .as_deref()
                .ok_or(CoreError::AgentCreationRequestLeaseConflict)?,
        );
        let secret = if let Some(pending) = pending {
            validate_successor(&pending, &request)?;
            tx.execute(
                "UPDATE runtime_core_credentials SET lease_sha256=$2 WHERE creation_request_id=$1",
                &[&request.id, &lease_hash],
            )
            .await
            .map_err(store_error)?;
            pending.get("bootstrap_secret")
        } else {
            let secret = new_secret()?;
            tx.execute(
                "INSERT INTO runtime_core_credentials
                 (creation_request_id,source_host_id,source_machine_id,owner_user_id,
                  bootstrap_secret,token_sha256,lease_sha256)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)",
                &[
                    &request.id,
                    &relocation.target_source_host_id,
                    &relocation.source_machine_id,
                    &request.owner_user_id,
                    &secret,
                    &digest(&secret),
                    &lease_hash,
                ],
            )
            .await
            .map_err(store_error)?;
            secret
        };
        self.finish(tx).await?;
        Ok(Some(RuntimeBootstrapCredential {
            secret,
            expected_previous_credential_sha256: None,
        }))
    }
}

/// Called only after the completion transaction has validated and replaced the
/// runtime placement. Registration alone deliberately keeps the source binding.
/// Returns whether a successor now holds authority; the caller must activate it
/// in the same transaction.
#[tracing::instrument(skip_all, fields(creation_request_id = request.id))]
pub(super) async fn complete<C: GenericClient + Sync>(
    client: &C,
    request: &AgentCreationRequest,
) -> CoreResult<bool> {
    let relocation = request
        .relocation
        .as_ref()
        .ok_or(CoreError::RuntimeSpecMismatch)?
        .v1();
    let runtime = lock_assignment(
        client,
        request,
        &relocation.target_source_host_id,
        &relocation.source_machine_id,
    )
    .await?;
    let Some((current, pending)) = lock_handoff(client, &runtime, request).await? else {
        return Ok(false);
    };
    ensure_live_now(client, &request.id).await?;
    let pending = pending.ok_or_else(|| conflict("successor credential missing at completion"))?;
    validate_successor(&pending, request)?;
    if request.lease_token.as_deref().map(digest).as_deref()
        != Some(pending.get::<_, String>("lease_sha256").as_str())
    {
        return Err(CoreError::AgentCreationRequestLeaseConflict);
    }
    let predecessor: String = current.get("creation_request_id");
    // Copy the latest owner-controlled configuration under the same locks;
    // provisioning-time snapshots would lose changes made during launch.
    // A new process must acknowledge application before routing is ready again.
    client
        .execute(
            "UPDATE runtime_core_credentials successor SET
           hosted_enabled=predecessor.hosted_enabled,
           hosted_generation=predecessor.hosted_generation,
           hosted_username=predecessor.hosted_username,
           hosted_password=predecessor.hosted_password,
           hosted_signing_secret=predecessor.hosted_signing_secret,
           hosted_applied_generation=NULL, hosted_apply_status='pending'
         FROM runtime_core_credentials predecessor
         WHERE successor.creation_request_id=$1 AND predecessor.creation_request_id=$2",
            &[&request.id, &predecessor],
        )
        .await
        .map_err(store_error)?;
    client.execute(
        "UPDATE runtime_core_credentials SET revoked=TRUE, activated=FALSE, agent_runtime_id=NULL
         WHERE creation_request_id=$1",
        &[&predecessor],
    ).await.map_err(store_error)?;
    client
        .execute(
            "UPDATE runtime_core_credentials SET agent_runtime_id=$2 WHERE creation_request_id=$1",
            &[&request.id, &runtime],
        )
        .await
        .map_err(store_error)?;
    Ok(true)
}

/// Locks the Runtime's current credential and this attempt's successor, then
/// validates the predecessor. `None` means the Runtime was never enrolled.
async fn lock_handoff<C: GenericClient + Sync>(
    client: &C,
    runtime: &str,
    request: &AgentCreationRequest,
) -> CoreResult<Option<(Row, Option<Row>)>> {
    let current = client
        .query_opt(
            "SELECT * FROM runtime_core_credentials WHERE agent_runtime_id=$1 FOR UPDATE",
            &[&runtime],
        )
        .await
        .map_err(store_error)?;
    let pending = client
        .query_opt(
            "SELECT * FROM runtime_core_credentials WHERE creation_request_id=$1 FOR UPDATE",
            &[&request.id],
        )
        .await
        .map_err(store_error)?;
    let Some(current) = current else {
        if pending.is_some() {
            return Err(conflict(
                "successor credential without a current predecessor",
            ));
        }
        require_unenrolled(client, runtime).await?;
        return Ok(None);
    };
    validate_predecessor(client, &current, request).await?;
    Ok(Some((current, pending)))
}

/// Every rejection names its check; credential values never reach the log.
fn conflict(check: &'static str) -> CoreError {
    tracing::warn!(check, "relocation credential handoff rejected");
    CoreError::ProviderOperationTransitionConflict
}

async fn require_unenrolled<C: GenericClient + Sync>(client: &C, runtime: &str) -> CoreResult<()> {
    let enrolled: bool = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM runtime_core_credentials c
         JOIN agent_creation_requests q ON q.id=c.creation_request_id
         WHERE q.agent_runtime_id=$1)",
            &[&runtime],
        )
        .await
        .map_err(store_error)?
        .get(0);
    if enrolled {
        return Err(conflict("credential history without a current credential"));
    }
    Ok(())
}

async fn lock_assignment<C: GenericClient + Sync>(
    client: &C,
    request: &AgentCreationRequest,
    host: &str,
    machine: &str,
) -> CoreResult<String> {
    let runtime = request
        .agent_runtime_id
        .as_deref()
        .ok_or(CoreError::RuntimeSpecMismatch)?;
    let row = client
        .query_opt(
            "SELECT r.id FROM agent_runtimes r
         JOIN projects p ON p.id=r.project_id
         JOIN project_runtime_links l ON l.project_id=p.id AND l.agent_runtime_id=r.id
         WHERE r.id=$1 AND r.project_id=$2 AND p.owner_user_id=$3
           AND r.source_host_id=$4 AND r.source_machine_id=$5 AND l.active
           AND p.import_candidate_id IS NULL AND r.offboarding_phase IS NULL
           AND NOT EXISTS (SELECT 1 FROM runtime_control_requests control
             WHERE control.agent_runtime_id=r.id
               AND control.status IN ('requested','launching','compute_up','ready'))
         FOR UPDATE OF r,p,l",
            &[
                &runtime,
                &request.project_id,
                &request.owner_user_id,
                &host,
                &machine,
            ],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::ProviderOperationIdentityMismatch)?;
    Ok(row.get(0))
}

async fn validate_predecessor<C: GenericClient + Sync>(
    client: &C,
    row: &Row,
    request: &AgentCreationRequest,
) -> CoreResult<()> {
    let relocation = request
        .relocation
        .as_ref()
        .ok_or(CoreError::RuntimeSpecMismatch)?
        .v1();
    if row.get::<_, bool>("revoked") {
        return Err(conflict("current credential is revoked"));
    }
    if row.get::<_, String>("creation_request_id") == request.id
        || row.get::<_, String>("owner_user_id") != request.owner_user_id
        || row.get::<_, String>("source_host_id") != relocation.source_host_id
        || row.get::<_, Option<String>>("source_machine_id").as_deref()
            != Some(relocation.source_machine_id.as_str())
    {
        return Err(conflict(
            "current credential does not match the relocation source",
        ));
    }
    // A typed stop suspends activation, so activated=false is legitimate here.
    // Its completed creation lineage must still authorize this exact Runtime.
    let origin =
        locked_agent_creation_request(client, &row.get::<_, String>("creation_request_id")).await?;
    if origin.status != AgentCreationRequestStatus::Running
        || origin.agent_runtime_id != request.agent_runtime_id
        || origin.project_id != request.project_id
        || origin.owner_user_id != request.owner_user_id
        || origin.relocation.as_ref().is_some_and(|previous| {
            previous.v1().target_source_host_id != relocation.source_host_id
                || previous.v1().source_machine_id != relocation.source_machine_id
                || origin.target_source_host_id.as_deref()
                    != Some(relocation.source_host_id.as_str())
        })
    {
        return Err(conflict(
            "current credential lineage does not authorize this Runtime",
        ));
    }
    Ok(())
}

fn validate_successor(row: &Row, request: &AgentCreationRequest) -> CoreResult<()> {
    let relocation = request
        .relocation
        .as_ref()
        .ok_or(CoreError::RuntimeSpecMismatch)?
        .v1();
    if row.get::<_, bool>("revoked")
        || row.get::<_, bool>("activated")
        || row.get::<_, Option<String>>("agent_runtime_id").is_some()
        || row.get::<_, String>("owner_user_id") != request.owner_user_id
        || row.get::<_, String>("source_host_id") != relocation.target_source_host_id
        || row.get::<_, Option<String>>("source_machine_id").as_deref()
            != Some(relocation.source_machine_id.as_str())
    {
        return Err(conflict(
            "successor credential does not match this relocation",
        ));
    }
    Ok(())
}
