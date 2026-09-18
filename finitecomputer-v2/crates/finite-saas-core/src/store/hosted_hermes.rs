//! Durable native serving intent. Core is the only credential writer; the
//! assignment-authenticated runtime reads and reports application. Account APIs
//! never serialize native credentials or the runtime bootstrap.
use super::runtime_credentials::{authenticated, new_secret};
use super::*;
use serde::{Deserialize, Serialize};

/// Credential-free input to Runner's local container verification. This is a
/// readiness snapshot, not a reservation or permission to reuse an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedRouteTarget {
    pub runtime_id: String,
    pub project_id: String,
    pub source_machine_id: String,
    pub generation: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedAccess {
    pub runtime_id: String,
    pub enrolled: bool,
    pub enabled: bool,
    pub generation: i64,
    pub applied_generation: Option<i64>,
    pub apply_status: String,
}

// Deliberately no Debug. Serialize only on the scoped runtime listener.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedDesired {
    pub runtime_id: String,
    pub generation: i64,
    pub enabled: bool,
    pub public_url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub signing_secret: Option<String>,
    pub access_ttl_seconds: u64,
}

// Used internally before and after native authentication to fence authority
// changes during the network request. Never serialized in account responses.
#[derive(Clone, PartialEq, Eq)]
pub struct HostedLogin {
    pub runtime_id: String,
    pub creation_request_id: String,
    pub generation: i64,
    pub base_url: String,
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetHostedAccess {
    pub enabled: bool,
    pub expected_generation: i64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostedReport {
    pub generation: i64,
    pub status: ApplyStatus,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyStatus {
    Applied,
    Error,
}

impl CoreStore {
    /// Only completed, current assignments with acknowledged serving intent
    /// qualify. Pending launches may authenticate to pull intent, but cannot be
    /// published yet. One SQL snapshot avoids mixing placement and readiness
    /// across reads; Runner still verifies local ownership under its lifecycle
    /// fence before publishing and rebuilds the projection on reconciliation.
    pub async fn hosted_route_targets_for_host(
        &self,
        source_host_id: &str,
    ) -> CoreResult<Vec<HostedRouteTarget>> {
        let client = self.connection().await?;
        let limit = crate::hosted_hermes::MAX_HOSTED_HERMES_ROUTES as i64 + 1;
        let rows = client
            .query(
                "SELECT r.id, r.project_id, r.source_machine_id, c.hosted_generation
             FROM runtime_core_credentials c
             JOIN agent_runtimes r ON r.id=c.agent_runtime_id
             JOIN agent_creation_requests q ON q.id=c.creation_request_id
             JOIN projects p ON p.id=r.project_id AND p.owner_user_id=c.owner_user_id
             WHERE r.source_host_id=$1 AND c.source_host_id=r.source_host_id
               AND c.source_machine_id=r.source_machine_id
               AND q.agent_runtime_id=r.id AND q.project_id=r.project_id
               AND q.status='running' AND c.activated AND NOT c.revoked
               AND p.import_candidate_id IS NULL AND r.offboarding_phase IS NULL
               AND COALESCE(r.host_facts->>'runtime_status', '') <> 'offline'
               AND EXISTS(SELECT 1 FROM project_runtime_links l
                   WHERE l.project_id=r.project_id AND l.agent_runtime_id=r.id AND l.active)
               AND c.hosted_enabled AND c.hosted_apply_status='applied'
               AND c.hosted_applied_generation=c.hosted_generation
             ORDER BY r.id LIMIT $2",
                &[&source_host_id, &limit],
            )
            .await
            .map_err(store_error)?;
        if rows.len() > crate::hosted_hermes::MAX_HOSTED_HERMES_ROUTES {
            // Never silently truncate: consumers replace the whole route set.
            return Err(CoreError::Store(
                "hosted Hermes route limit exceeded".into(),
            ));
        }
        Ok(rows
            .iter()
            .map(|row| HostedRouteTarget {
                runtime_id: row.get(0),
                project_id: row.get(1),
                source_machine_id: row.get(2),
                generation: row.get(3),
            })
            .collect())
    }

    pub async fn hosted_access(&self, runtime: &str, user: &str) -> CoreResult<HostedAccess> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        lock_owner(&*tx, runtime, user).await?;
        let row = hosted_row(&*tx, runtime).await?;
        let result = access(runtime, row.as_ref());
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn set_hosted_access(
        &self,
        runtime: &str,
        user: &str,
        input: SetHostedAccess,
    ) -> CoreResult<HostedAccess> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        lock_owner(&*tx, runtime, user).await?;
        let row = hosted_row(&*tx, runtime)
            .await?
            .ok_or(CoreError::ProviderOperationTransitionConflict)?;
        if row.get::<_, i64>("hosted_generation") != input.expected_generation {
            return Err(CoreError::ProviderOperationTransitionConflict);
        }
        if row.get::<_, bool>("hosted_enabled") != input.enabled {
            // Disable drops Core's native material. Re-enable always generates
            // a new signing key; old credentials/sessions cannot revive.
            let (username, password, signing) = if input.enabled {
                (
                    Some(format!("finite-{}", new_secret()?)),
                    Some(new_secret()?),
                    Some(new_secret()?),
                )
            } else {
                (None, None, None)
            };
            tx.execute("UPDATE runtime_core_credentials SET hosted_enabled=$2,
                hosted_generation=hosted_generation+1, hosted_username=$3, hosted_password=$4,
                hosted_signing_secret=$5, hosted_applied_generation=NULL, hosted_apply_status='pending'
                WHERE agent_runtime_id=$1", &[&runtime,&input.enabled,&username,&password,&signing]).await.map_err(store_error)?;
        }
        let result = access(runtime, hosted_row(&*tx, runtime).await?.as_ref());
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn hosted_desired(
        &self,
        secret: &str,
        origins: &crate::hosted_hermes::HostedHermesOrigins,
    ) -> CoreResult<Option<HostedDesired>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let Some(runtime) = authenticated(&*tx, secret).await? else {
            return Ok(None);
        };
        // Same lock order as account mutations; authenticate again after locks.
        lock_runtime(&*tx, &runtime.agent_runtime_id).await?;
        let Some(row) = hosted_row(&*tx, &runtime.agent_runtime_id).await? else {
            return Ok(None);
        };
        if authenticated(&*tx, secret).await? != Some(runtime.clone()) {
            return Ok(None);
        }
        let enabled = row.get("hosted_enabled");
        let base = origins
            .location(
                &row.get::<_, String>("source_host_id"),
                &runtime.agent_runtime_id,
            )
            .base_url;
        if enabled && base.is_none() {
            return Err(CoreError::ProviderOperationTransitionConflict);
        }
        let result = HostedDesired {
            runtime_id: runtime.agent_runtime_id,
            generation: row.get("hosted_generation"),
            enabled,
            public_url: if enabled { base } else { None },
            username: row.get("hosted_username"),
            password: row.get("hosted_password"),
            signing_secret: row.get("hosted_signing_secret"),
            access_ttl_seconds: 60,
        };
        tx.commit().await.map_err(store_error)?;
        Ok(Some(result))
    }

    pub async fn report_hosted(&self, secret: &str, input: HostedReport) -> CoreResult<bool> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let Some(runtime) = authenticated(&*tx, secret).await? else {
            return Ok(false);
        };
        lock_runtime(&*tx, &runtime.agent_runtime_id).await?;
        let Some(row) = hosted_row(&*tx, &runtime.agent_runtime_id).await? else {
            return Ok(false);
        };
        if authenticated(&*tx, secret).await? != Some(runtime.clone()) {
            return Ok(false);
        }
        if row.get::<_, i64>("hosted_generation") != input.generation {
            return Err(CoreError::ProviderOperationTransitionConflict);
        }
        let status = match input.status {
            ApplyStatus::Applied => "applied",
            ApplyStatus::Error => "error",
        };
        tx.execute("UPDATE runtime_core_credentials SET hosted_applied_generation=$2, hosted_apply_status=$3 WHERE agent_runtime_id=$1", &[&runtime.agent_runtime_id,&input.generation,&status]).await.map_err(store_error)?;
        self.finish(tx).await?;
        Ok(true)
    }

    pub async fn hosted_login(
        &self,
        runtime: &str,
        user: &str,
        origins: &crate::hosted_hermes::HostedHermesOrigins,
    ) -> CoreResult<HostedLogin> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        lock_owner(&*tx, runtime, user).await?;
        let row = hosted_row(&*tx, runtime)
            .await?
            .ok_or(CoreError::ProviderOperationTransitionConflict)?;
        let generation: i64 = row.get("hosted_generation");
        if !row.get::<_, bool>("hosted_enabled")
            || row.get::<_, String>("hosted_apply_status") != "applied"
            || row.get::<_, Option<i64>>("hosted_applied_generation") != Some(generation)
            || authenticated(&*tx, &row.get::<_, String>("bootstrap_secret"))
                .await?
                .is_none()
        {
            return Err(CoreError::ProviderOperationTransitionConflict);
        }
        let result = HostedLogin {
            runtime_id: runtime.into(),
            creation_request_id: row.get("creation_request_id"),
            generation,
            base_url: origins
                .location(&row.get::<_, String>("source_host_id"), runtime)
                .base_url
                .ok_or(CoreError::ProviderOperationTransitionConflict)?,
            username: row
                .get::<_, Option<String>>("hosted_username")
                .ok_or(CoreError::ProviderOperationTransitionConflict)?,
            password: row
                .get::<_, Option<String>>("hosted_password")
                .ok_or(CoreError::ProviderOperationTransitionConflict)?,
        };
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }
}

fn access(runtime: &str, row: Option<&Row>) -> HostedAccess {
    HostedAccess {
        runtime_id: runtime.into(),
        enrolled: row.is_some(),
        enabled: row.is_some_and(|r| r.get("hosted_enabled")),
        generation: row.map_or(0, |r| r.get("hosted_generation")),
        applied_generation: row.and_then(|r| r.get("hosted_applied_generation")),
        apply_status: row.map_or_else(|| "pending".into(), |r| r.get("hosted_apply_status")),
    }
}

async fn lock_runtime<C: GenericClient + Sync>(tx: &C, runtime: &str) -> CoreResult<()> {
    tx.query_opt("SELECT r.id FROM agent_runtimes r JOIN projects p ON p.id=r.project_id WHERE r.id=$1 FOR UPDATE OF r,p", &[&runtime]).await.map_err(store_error)?.ok_or(CoreError::ProjectRuntimeNotFound)?;
    Ok(())
}
async fn lock_owner<C: GenericClient + Sync>(tx: &C, runtime: &str, user: &str) -> CoreResult<()> {
    lock_runtime(tx, runtime).await?;
    let allowed = tx.query_opt("SELECT r.id FROM agent_runtimes r JOIN projects p ON p.id=r.project_id JOIN users u ON u.id=p.owner_user_id JOIN project_runtime_links l ON l.project_id=p.id AND l.agent_runtime_id=r.id WHERE r.id=$1 AND u.workos_user_id=$2 AND l.active AND p.import_candidate_id IS NULL FOR UPDATE OF l", &[&runtime,&user]).await.map_err(store_error)?;
    allowed.ok_or(CoreError::ProjectRuntimeNotFound)?;
    Ok(())
}
async fn hosted_row<C: GenericClient + Sync>(tx: &C, runtime: &str) -> CoreResult<Option<Row>> {
    tx.query_opt("SELECT c.* FROM runtime_core_credentials c JOIN agent_runtimes r ON r.id=c.agent_runtime_id JOIN projects p ON p.id=r.project_id
        WHERE c.agent_runtime_id=$1 AND NOT c.revoked AND c.source_host_id=r.source_host_id
        AND c.source_machine_id=r.source_machine_id AND c.owner_user_id=p.owner_user_id FOR UPDATE OF c", &[&runtime]).await.map_err(store_error)
}

#[cfg(test)]
mod tests;
