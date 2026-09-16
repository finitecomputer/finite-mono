//! Short-lived, endpoint-bound access to the runtime's Hermes loopback service.
use super::runtime_credentials::{authenticated, hex_secret};
use super::*;
use serde::{Deserialize, Serialize};

pub const ADMISSION_SECONDS: i64 = 300;
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PeerAdmissionRequest {
    pub generation: i64,
    pub endpoint_id: String,
    pub peer_id: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HermesAdmission {
    pub generation: i64,
    pub endpoint_id: String,
    pub relay_url: String,
    pub service: String,
    pub expires_in_seconds: i64,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmittedPeer {
    pub peer_id: String,
    pub expires_in_seconds: i64,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeAdmissions {
    pub generation: i64,
    pub endpoint_id: String,
    pub service: String,
    pub peers: Vec<AdmittedPeer>,
}
impl CoreStore {
    pub async fn runtime_hermes_endpoint(
        &self,
        project: &str,
        actor: &str,
        admin: bool,
    ) -> CoreResult<serde_json::Value> {
        let client = self.connection().await?;
        authorize_owner(&**client, project, actor, admin).await?;
        let rows = client.query("SELECT c.endpoint_generation,c.endpoint_id,c.relay_url,c.hosted_access_enabled FROM runtime_core_credentials c JOIN agent_runtimes r ON r.id=c.agent_runtime_id JOIN project_runtime_links l ON l.agent_runtime_id=r.id AND l.project_id=r.project_id JOIN agent_creation_requests q ON q.id=c.creation_request_id WHERE r.project_id=$1 AND l.active AND NOT c.revoked AND c.activated AND q.status='running' AND c.endpoint_id IS NOT NULL", &[&project]).await.map_err(store_error)?;
        if rows.len() != 1 {
            return Err(CoreError::ProjectNotFound);
        }
        let r = &rows[0];
        Ok(
            serde_json::json!({"generation":r.get::<_,i64>(0),"endpointId":r.get::<_,String>(1),"relayUrl":r.get::<_,String>(2),"enabled":r.get::<_,bool>(3),"service":"hermes"}),
        )
    }

    /// API callers must authenticate the operator before calling this method.
    /// The expected endpoint prevents toggling a replacement by stale UI state.
    pub async fn set_runtime_hosted_access(
        &self,
        project: &str,
        expected: &PeerAdmissionRequest,
        enabled: bool,
    ) -> CoreResult<()> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let (creation, _) = locked_endpoint(&*tx, project, expected).await?;
        tx.execute("UPDATE runtime_core_credentials SET hosted_access_enabled=$2 WHERE creation_request_id=$1", &[&creation, &enabled]).await.map_err(store_error)?;
        if !enabled {
            tx.execute(
                "DELETE FROM runtime_peer_admissions WHERE creation_request_id=$1",
                &[&creation],
            )
            .await
            .map_err(store_error)?;
        }
        self.finish(tx).await
    }

    /// Full native Hermes access requires the Project owner or a verified admin.
    pub async fn admit_runtime_peer(
        &self,
        project: &str,
        actor: &str,
        admin: bool,
        expected: &PeerAdmissionRequest,
    ) -> CoreResult<HermesAdmission> {
        if !hex_secret(&expected.peer_id) {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        authorize_owner(&*tx, project, actor, admin).await?;
        let (creation, relay) = locked_endpoint(&*tx, project, expected).await?;
        let enabled: bool = tx.query_one("SELECT hosted_access_enabled FROM runtime_core_credentials WHERE creation_request_id=$1", &[&creation]).await.map_err(store_error)?.get(0);
        if !enabled {
            return Err(CoreError::ProjectNotFound);
        }
        // Bound retained state per runtime; expired or superseded rows have no authority.
        tx.execute("DELETE FROM runtime_peer_admissions WHERE creation_request_id=$1 AND (expires_at<=clock_timestamp() OR endpoint_generation<>$2)", &[&creation, &expected.generation]).await.map_err(store_error)?;
        let count: i64 = tx.query_one("SELECT COUNT(*) FROM runtime_peer_admissions WHERE creation_request_id=$1 AND peer_id<>$2", &[&creation, &expected.peer_id]).await.map_err(store_error)?.get(0);
        if count >= 64 {
            return Err(CoreError::ProviderOperationTransitionConflict);
        }
        let changed = tx.execute("INSERT INTO runtime_peer_admissions (creation_request_id,endpoint_generation,peer_id,workos_user_id,expires_at) VALUES ($1,$2,$3,$4,clock_timestamp()+INTERVAL '300 seconds') ON CONFLICT (creation_request_id,endpoint_generation,peer_id) DO UPDATE SET expires_at=EXCLUDED.expires_at WHERE runtime_peer_admissions.workos_user_id=EXCLUDED.workos_user_id", &[&creation, &expected.generation, &expected.peer_id, &actor]).await.map_err(store_error)?;
        if changed != 1 {
            return Err(CoreError::ProviderOperationTransitionConflict);
        }
        self.finish(tx).await?;
        Ok(HermesAdmission {
            generation: expected.generation,
            endpoint_id: expected.endpoint_id.clone(),
            relay_url: relay,
            service: "hermes".into(),
            expires_in_seconds: ADMISSION_SECONDS,
        })
    }

    pub async fn remove_runtime_peer(
        &self,
        project: &str,
        actor: &str,
        admin: bool,
        expected: &PeerAdmissionRequest,
    ) -> CoreResult<()> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        authorize_owner(&*tx, project, actor, admin).await?;
        let (creation, _) = locked_endpoint(&*tx, project, expected).await?;
        tx.execute("DELETE FROM runtime_peer_admissions WHERE creation_request_id=$1 AND endpoint_generation=$2 AND peer_id=$3 AND (workos_user_id=$4 OR $5)", &[&creation, &expected.generation, &expected.peer_id, &actor, &admin]).await.map_err(store_error)?;
        self.finish(tx).await
    }

    pub async fn runtime_admissions(
        &self,
        secret: &str,
        generation: i64,
        endpoint: &str,
    ) -> CoreResult<Option<RuntimeAdmissions>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let Some(identity) = authenticated(&*tx, secret).await? else {
            return Ok(None);
        };
        // Serialize with lifecycle completion before deciding whether this is
        // temporary suspension (401) or a superseded endpoint (409). A stop
        // racing this read must not park the resident endpoint permanently.
        tx.query_opt(
            "SELECT id FROM agent_runtimes WHERE id=$1 FOR UPDATE",
            &[&identity.agent_runtime_id],
        )
        .await
        .map_err(store_error)?;
        if authenticated(&*tx, secret).await?.is_none() {
            return Ok(None);
        }
        let Some(row) = tx
            .query_opt(
                "SELECT activated FROM runtime_core_credentials WHERE creation_request_id=$1",
                &[&identity.creation_request_id],
            )
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        let activated: bool = row.get(0);
        if !activated {
            return Ok(Some(RuntimeAdmissions {
                generation,
                endpoint_id: endpoint.into(),
                service: "hermes".into(),
                peers: Vec::new(),
            }));
        }
        let expected = PeerAdmissionRequest {
            generation,
            endpoint_id: endpoint.into(),
            peer_id: String::new(),
        };
        let (creation, _) = locked_endpoint(&*tx, &identity.project_id, &expected).await?;
        // Recheck authority after acquiring the lifecycle locks.
        if authenticated(&*tx, secret).await?.is_none() {
            return Ok(None);
        }
        let rows = tx.query("SELECT a.peer_id, FLOOR(EXTRACT(EPOCH FROM (a.expires_at-clock_timestamp())))::BIGINT FROM runtime_peer_admissions a JOIN runtime_core_credentials c USING (creation_request_id) WHERE a.creation_request_id=$1 AND a.endpoint_generation=$2 AND c.hosted_access_enabled AND a.expires_at>clock_timestamp() ORDER BY a.peer_id", &[&creation, &generation]).await.map_err(store_error)?;
        let peers = rows
            .into_iter()
            .map(|r| AdmittedPeer {
                peer_id: r.get(0),
                expires_in_seconds: r.get::<_, i64>(1).clamp(0, ADMISSION_SECONDS),
            })
            .collect();
        self.finish(tx).await?;
        Ok(Some(RuntimeAdmissions {
            generation,
            endpoint_id: endpoint.into(),
            service: "hermes".into(),
            peers,
        }))
    }
}
async fn authorize_owner<C: GenericClient + Sync>(
    client: &C,
    project: &str,
    actor: &str,
    admin: bool,
) -> CoreResult<()> {
    if admin {
        return Ok(());
    }
    let allowed: bool = client.query_one("SELECT EXISTS(SELECT 1 FROM projects p JOIN users u ON u.id=p.owner_user_id WHERE p.id=$1 AND u.workos_user_id=$2)", &[&project, &actor]).await.map_err(store_error)?.get(0);
    if allowed {
        Ok(())
    } else {
        Err(CoreError::ProjectNotFound)
    }
}
async fn locked_endpoint<C: GenericClient + Sync>(
    client: &C,
    project: &str,
    expected: &PeerAdmissionRequest,
) -> CoreResult<(String, String)> {
    if expected.generation <= 0 || !hex_secret(&expected.endpoint_id) {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    let rows = client.query("SELECT r.id FROM agent_runtimes r JOIN project_runtime_links l ON l.agent_runtime_id=r.id AND l.project_id=r.project_id WHERE r.project_id=$1 AND l.active FOR UPDATE OF r", &[&project]).await.map_err(store_error)?;
    if rows.len() != 1 {
        return Err(CoreError::ProjectNotFound);
    }
    let runtime: String = rows[0].get(0);
    let row = client.query_opt("SELECT c.creation_request_id,c.relay_url FROM runtime_core_credentials c JOIN agent_creation_requests q ON q.id=c.creation_request_id WHERE c.agent_runtime_id=$1 AND NOT c.revoked AND c.activated AND q.status='running' AND c.endpoint_generation=$2 AND c.endpoint_id=$3 FOR UPDATE OF c", &[&runtime, &expected.generation, &expected.endpoint_id]).await.map_err(store_error)?.ok_or(CoreError::ProviderOperationTransitionConflict)?;
    Ok((row.get(0), row.get(1)))
}
