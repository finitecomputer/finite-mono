//! Boot-time public flags share Core's operator configuration authority.
//! No new settings table, provider environment mirror, or credential response.
use super::runtime_credentials::{authenticated_for_boot, digest};
use super::*;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeEnvironment {
    pub runtime_id: String,
    pub environment: BTreeMap<String, String>,
}

impl CoreStore {
    pub async fn runtime_environment(
        &self,
        secret: &str,
    ) -> CoreResult<Option<RuntimeEnvironment>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.execute("SET TRANSACTION READ ONLY", &[])
            .await
            .map_err(store_error)?;
        // A first boot precedes runtime registration. Its bootstrap capability
        // must still be bound to the current, unexpired creation lease. Running
        // agents use the current-assignment reader. A stopped agent may fetch
        // flags during its live restart lease; normal Hermes access remains
        // inactive until completion. Otherwise boot and completion deadlock.
        let row = tx.query_opt(
            "SELECT q.id,q.agent_runtime_id,q.status,q.lease_token,c.lease_sha256,q.owner_chat_account_id,c.agent_runtime_id
             FROM runtime_core_credentials c
             JOIN agent_creation_requests q ON q.id=c.creation_request_id
             JOIN projects p ON p.id=q.project_id AND p.owner_user_id=c.owner_user_id
             WHERE c.token_sha256=$1 AND NOT c.revoked
               AND q.agent_runtime_id IS NOT NULL
               AND (q.status='running' OR
                    (q.status='launching' AND q.lease_expires_at>clock_timestamp() AND q.lease_token IS NOT NULL))",
            &[&digest(secret)],
        ).await.map_err(store_error)?;
        let Some(row) = row else { return Ok(None) };
        let runtime_id: String = row.get(1);
        if row.get::<_, String>(2) == "launching" && row.get::<_, Option<String>>(6).is_none() {
            let lease: Option<String> = row.get(3);
            if lease.as_deref().map(digest).as_deref() != Some(row.get::<_, String>(4).as_str()) {
                return Ok(None);
            }
        } else if authenticated_for_boot(&*tx, secret)
            .await?
            .is_none_or(|runtime| runtime.agent_runtime_id != runtime_id)
        {
            return Ok(None);
        }
        let mut environment = self.runtime_environment.as_ref().clone();
        // Ownership is per-agent authority, never an operator-wide allowlist.
        environment.remove(crate::runtime_spec::OWNER_CHAT_NPUBS_ENV);
        if let Some(owner) = row.get::<_, Option<String>>(5) {
            environment.insert(crate::runtime_spec::OWNER_CHAT_NPUBS_ENV.into(), owner);
        }
        validate_runtime_spec_environment(&environment)?;
        tx.commit().await.map_err(store_error)?;
        Ok(Some(RuntimeEnvironment {
            runtime_id,
            environment,
        }))
    }
}
