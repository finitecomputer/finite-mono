//! Core-owned bearer bootstrap for an exact runtime assignment.
//! Bootstrap is private launch material, never part of RuntimeSpec or runtime views.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// No Debug: launch credentials must not become diagnostic data.
pub struct ProvisionRuntimeCredential {
    pub creation_request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    /// Derived from the authenticated Runner, not request JSON.
    pub source_host_id: String,
    /// Trusted Core host configuration, never a caller-supplied preference.
    pub prepare_hosted_access: bool,
}

// Existing-agent delivery is tied to a live upgrade lease, not a runtime ID
// supplied by a caller. No Debug: the lease token is private.
pub struct ProvisionUpgradeCredential {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub source_host_id: String,
    pub prepare_hosted_access: bool,
}

// Serialize only for the dedicated, authenticated provisioning response.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeBootstrapCredential {
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedRuntime {
    pub agent_runtime_id: String,
    pub project_id: String,
    pub creation_request_id: String,
}

pub(super) fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn hex_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub(crate) fn new_secret() -> CoreResult<String> {
    let mut bytes = [0; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| CoreError::Store("runtime credential generation failed".into()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn validate_runtime_core_url(value: &str) -> CoreResult<String> {
    let url = reqwest::Url::parse(value).map_err(|_| CoreError::RuntimeSpecMismatch)?;
    let local = url
        .host_str()
        .is_some_and(|host| host == "localhost" || host == "127.0.0.1" || host == "[::1]");
    if (url.scheme() != "https" && !(url.scheme() == "http" && local))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

impl CoreStore {
    /// Enroll an existing assignment as part of its already-authorized upgrade.
    /// Retries return the same credential. Ambiguous, revoked or changed
    /// assignments fail closed; this is not an ownership/relocation repair API.
    pub async fn provision_upgrade_credential(
        &self,
        input: ProvisionUpgradeCredential,
    ) -> CoreResult<RuntimeBootstrapCredential> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let request = locked_runtime_control_request(&*tx, &input.request_id).await?;
        verify_runtime_control_lease(&request, &input.runner_id, &input.lease_token)?;
        if request.kind != RuntimeControlKind::Upgrade
            || request.source_host_id != input.source_host_id
        {
            return Err(CoreError::ProviderOperationIdentityMismatch);
        }
        let row = tx
            .query_opt(
                "SELECT p.owner_user_id FROM agent_runtimes r
             JOIN projects p ON p.id=r.project_id
             JOIN project_runtime_links l ON l.project_id=p.id AND l.agent_runtime_id=r.id
             WHERE r.id=$1 AND r.project_id=$2 AND r.source_host_id=$3
               AND r.source_machine_id=$4 AND l.active AND p.import_candidate_id IS NULL
             FOR UPDATE OF r,p,l",
                &[
                    &request.agent_runtime_id,
                    &request.project_id,
                    &input.source_host_id,
                    &request.source_machine_id,
                ],
            )
            .await
            .map_err(store_error)?
            .ok_or(CoreError::ProviderOperationIdentityMismatch)?;
        let owner: String = row.get(0);
        // The DB-enforced primary creation is a stable origin reference, not
        // placement authority. Completed relocations remain history; the
        // current lease/runtime/link above authorize delivery. Never sort history.
        let creations = tx
            .query(
                "SELECT id,owner_user_id FROM agent_creation_requests
             WHERE agent_runtime_id=$1 AND project_id=$2 AND relocation_spec IS NULL AND status='running' FOR UPDATE",
                &[&request.agent_runtime_id, &request.project_id],
            )
            .await
            .map_err(store_error)?;
        if creations.len() != 1 || creations[0].get::<_, String>(1) != owner {
            return Err(CoreError::ProviderOperationIdentityMismatch);
        }
        let creation: String = creations[0].get(0);
        let existing = tx.query_opt(
            "SELECT bootstrap_secret,creation_request_id,source_host_id,source_machine_id,owner_user_id,revoked,activated,agent_runtime_id
             FROM runtime_core_credentials WHERE agent_runtime_id=$1 OR creation_request_id=$2 FOR UPDATE",
            &[&request.agent_runtime_id, &creation],
        ).await.map_err(store_error)?;
        // Validate wall-clock expiry after waiting for all state locks.
        let live: bool = tx.query_one(
            "SELECT COALESCE(lease_expires_at>clock_timestamp(),FALSE) FROM runtime_control_requests WHERE id=$1",
            &[&request.id],
        ).await.map_err(store_error)?.get(0);
        if !live {
            return Err(CoreError::RuntimeControlRequestLeaseConflict);
        }
        let secret = if let Some(row) = existing {
            if row.get::<_, String>(1) != creation
                || row.get::<_, String>(2) != input.source_host_id
                || row.get::<_, Option<String>>(3).as_deref() != Some(&request.source_machine_id)
                || row.get::<_, String>(4) != owner
                || row.get::<_, bool>(5)
                || !row.get::<_, bool>(6)
                || row.get::<_, Option<String>>(7).as_deref() != Some(&request.agent_runtime_id)
            {
                return Err(CoreError::ProviderOperationTransitionConflict);
            }
            row.get(0)
        } else {
            let secret = new_secret()?;
            tx.execute(
                "INSERT INTO runtime_core_credentials
                 (creation_request_id,agent_runtime_id,source_host_id,source_machine_id,owner_user_id,
                  bootstrap_secret,token_sha256,lease_sha256,activated)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,TRUE)",
                &[&creation,&request.agent_runtime_id,&input.source_host_id,&request.source_machine_id,
                  &owner,&secret,&digest(&secret),&digest(&input.lease_token)],
            ).await.map_err(store_error)?;
            if input.prepare_hosted_access {
                super::hosted_hermes::prepare_initial_access(&*tx, &creation).await?;
            }
            secret
        };
        self.finish(tx).await?;
        Ok(RuntimeBootstrapCredential { secret })
    }

    /// Core returns the same secret for retries of this creation, including a
    /// new live lease on the same host. It never rotates an installed secret as
    /// a side effect of retry. A revoked/replaced creation cannot be re-enrolled.
    pub async fn provision_runtime_credential(
        &self,
        input: ProvisionRuntimeCredential,
    ) -> CoreResult<RuntimeBootstrapCredential> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let request = locked_agent_creation_request(&*tx, &input.creation_request_id).await?;
        verify_agent_creation_lease_active(&*tx, &request, &input.runner_id, &input.lease_token)
            .await?;
        if request.relocation.is_some()
            || request
                .target_source_host_id
                .as_deref()
                .is_some_and(|host| host != input.source_host_id)
        {
            return Err(CoreError::ProviderOperationIdentityMismatch);
        }
        if let Some(runtime_id) = request.agent_runtime_id.as_deref() {
            let row = tx
                .query_opt(
                    "SELECT source_host_id FROM agent_runtimes WHERE id=$1 FOR UPDATE",
                    &[&runtime_id],
                )
                .await
                .map_err(store_error)?;
            if let Some(row) = row {
                if row.get::<_, String>(0) != input.source_host_id {
                    return Err(CoreError::ProviderOperationIdentityMismatch);
                }
                check_initial_runtime(&*tx, &request.project_id, runtime_id).await?;
            }
        }
        // Lock before wall-clock validation, even if only the pending row exists.
        let existing = tx.query_opt("SELECT bootstrap_secret, source_host_id, revoked FROM runtime_core_credentials WHERE creation_request_id=$1 FOR UPDATE", &[&request.id]).await.map_err(store_error)?;
        ensure_live_now(&*tx, &request.id).await?;
        let lease_hash = digest(
            request
                .lease_token
                .as_deref()
                .ok_or(CoreError::AgentCreationRequestLeaseConflict)?,
        );
        let secret = match existing {
            Some(row) => {
                if row.get::<_, bool>(2) || row.get::<_, String>(1) != input.source_host_id {
                    return Err(CoreError::ProviderOperationTransitionConflict);
                }
                let secret: String = row.get(0);
                tx.execute("UPDATE runtime_core_credentials SET lease_sha256=$2 WHERE creation_request_id=$1", &[&request.id, &lease_hash]).await.map_err(store_error)?;
                secret
            }
            None => {
                let secret = new_secret()?;
                tx.execute("INSERT INTO runtime_core_credentials (creation_request_id,source_host_id,bootstrap_secret,token_sha256,lease_sha256,owner_user_id) SELECT $1,$2,$3,$4,$5,p.owner_user_id FROM projects p JOIN agent_creation_requests q ON q.project_id=p.id WHERE q.id=$1", &[&request.id,&input.source_host_id,&secret,&digest(&secret),&lease_hash]).await.map_err(store_error)?;
                if input.prepare_hosted_access {
                    super::hosted_hermes::prepare_initial_access(&*tx, &request.id).await?;
                }
                secret
            }
        };
        // If provisioning is retried after runtime registration, bind it now.
        bind_bootstrap(&*tx, &request.id).await?;
        self.finish(tx).await?;
        Ok(RuntimeBootstrapCredential { secret })
    }

    /// Observational reader. Mutation consumers use the transactional method below.
    pub async fn authenticate_runtime_credential(
        &self,
        secret: &str,
    ) -> CoreResult<Option<AuthenticatedRuntime>> {
        if !hex_secret(secret) {
            return Ok(None);
        }
        let client = self.connection().await?;
        authenticated(&**client, secret).await
    }

    pub async fn revoke_runtime_credential(
        &self,
        runtime_id: &str,
        expected_creation_request_id: &str,
    ) -> CoreResult<bool> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let changed = tx.execute("UPDATE runtime_core_credentials SET revoked=TRUE WHERE agent_runtime_id=$1 AND creation_request_id=$2 AND NOT revoked", &[&runtime_id,&expected_creation_request_id]).await.map_err(store_error)?;
        self.finish(tx).await?;
        Ok(changed == 1)
    }
}

async fn ensure_live_now<C: GenericClient + Sync>(client: &C, request_id: &str) -> CoreResult<()> {
    let live: bool = client.query_one("SELECT COALESCE(lease_expires_at>clock_timestamp(),FALSE) FROM agent_creation_requests WHERE id=$1", &[&request_id]).await.map_err(store_error)?.get(0);
    if !live {
        return Err(CoreError::AgentCreationRequestLeaseConflict);
    }
    Ok(())
}
async fn check_initial_runtime<C: GenericClient + Sync>(
    client: &C,
    project_id: &str,
    runtime_id: &str,
) -> CoreResult<()> {
    let allowed: bool = client.query_one("SELECT EXISTS(SELECT 1 FROM project_runtime_links WHERE project_id=$1 AND agent_runtime_id=$2 AND active) AND NOT EXISTS(SELECT 1 FROM runtime_control_requests WHERE agent_runtime_id=$2)", &[&project_id,&runtime_id]).await.map_err(store_error)?.get(0);
    if !allowed {
        return Err(CoreError::ProviderOperationTransitionConflict);
    }
    Ok(())
}

/// The creation row is locked by the caller. Validate submitted source facts
/// before runtime upsert, which preserves existing source fields on conflict.
pub(super) async fn validate_bootstrap_source<C: GenericClient + Sync>(
    client: &C,
    request_id: &str,
    host: &str,
    machine: &str,
) -> CoreResult<()> {
    let source = client.query_opt(
        "SELECT source_host_id,source_machine_id FROM runtime_core_credentials WHERE creation_request_id=$1",
        &[&request_id],
    ).await.map_err(store_error)?;
    if let Some(row) = source
        && (row.get::<_, String>(0) != host
            || row
                .get::<_, Option<String>>(1)
                .is_some_and(|value| value != machine))
    {
        return Err(CoreError::ProviderOperationIdentityMismatch);
    }
    Ok(())
}

/// Called by the actual runtime-registration and completion transactions.
/// Old launchers have no bootstrap row; those paths remain unchanged.
pub(super) async fn bind_bootstrap<C: GenericClient + Sync>(
    client: &C,
    request_id: &str,
) -> CoreResult<()> {
    let row = client
        .query_opt(
            "SELECT r.id,r.project_id,r.source_host_id,r.source_machine_id,q.lease_token
         FROM agent_creation_requests q JOIN agent_runtimes r ON r.id=q.agent_runtime_id
         WHERE q.id=$1",
            &[&request_id],
        )
        .await
        .map_err(store_error)?;
    let Some(row) = row else {
        return Ok(());
    };
    let runtime: String = row.get(0);
    client
        .query_one(
            "SELECT id FROM agent_runtimes WHERE id=$1 FOR UPDATE",
            &[&runtime],
        )
        .await
        .map_err(store_error)?;
    let origin = client.query_opt("SELECT source_host_id,lease_sha256,revoked,agent_runtime_id,source_machine_id FROM runtime_core_credentials WHERE creation_request_id=$1 FOR UPDATE", &[&request_id]).await.map_err(store_error)?;
    let Some(origin) = origin else {
        return Ok(());
    };
    if origin
        .get::<_, Option<String>>(3)
        .is_some_and(|id| id != runtime)
        || origin
            .get::<_, Option<String>>(4)
            .is_some_and(|machine| machine != row.get::<_, String>(3))
    {
        return Err(CoreError::ProviderOperationIdentityMismatch);
    }
    let lease: Option<String> = row.get(4);
    if origin.get::<_, bool>(2)
        || origin.get::<_, String>(0) != row.get::<_, String>(2)
        || lease.as_deref().map(digest).as_deref() != Some(origin.get::<_, String>(1).as_str())
    {
        return Err(CoreError::ProviderOperationTransitionConflict);
    }
    check_initial_runtime(client, &row.get::<_, String>(1), &runtime).await?;
    ensure_live_now(client, request_id).await?;
    client.execute("UPDATE runtime_core_credentials SET agent_runtime_id=$2,source_machine_id=$3 WHERE creation_request_id=$1", &[&request_id,&runtime,&row.get::<_, String>(3)]).await.map_err(store_error)?;
    Ok(())
}
pub(super) async fn authenticated<C: GenericClient + Sync>(
    client: &C,
    secret: &str,
) -> CoreResult<Option<AuthenticatedRuntime>> {
    let row = client.query_opt(
        "SELECT r.id,r.project_id,q.id,q.status,q.lease_token,c.lease_sha256 FROM runtime_core_credentials c
         JOIN agent_runtimes r ON r.id=c.agent_runtime_id
         JOIN agent_creation_requests q ON q.id=c.creation_request_id
         JOIN projects p ON p.id=r.project_id AND p.owner_user_id=c.owner_user_id
         WHERE c.token_sha256=$1 AND NOT c.revoked
           AND q.agent_runtime_id=r.id AND q.project_id=r.project_id
           AND c.source_host_id=r.source_host_id AND c.source_machine_id=r.source_machine_id
           AND EXISTS(SELECT 1 FROM project_runtime_links l WHERE l.project_id=r.project_id AND l.agent_runtime_id=r.id AND l.active)
           AND ((q.status='launching' AND q.lease_expires_at>clock_timestamp()
                 AND q.lease_token IS NOT NULL)
                OR (q.status='running' AND c.activated))",
        &[&digest(secret)],
    ).await.map_err(store_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.get::<_, String>(3) == "launching" {
        let lease: Option<String> = row.get(4);
        if lease.as_deref().map(digest).as_deref() != Some(row.get::<_, String>(5).as_str()) {
            return Ok(None);
        }
    }
    Ok(Some(AuthenticatedRuntime {
        agent_runtime_id: row.get(0),
        project_id: row.get(1),
        creation_request_id: row.get(2),
    }))
}

#[cfg(test)]
pub(crate) mod tests;
