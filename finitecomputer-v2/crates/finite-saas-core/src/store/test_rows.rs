use super::*;

/// Typed row reads used only by tests.
///
/// The production store API is task-shaped (request / lease / complete), so
/// tests that used to inspect `BridgeCoreState`'s public maps have no reader
/// for a bare row. These reuse the production `select_*` + `*_from_row` pair,
/// so a test decodes a row exactly the way production does — including the
/// column list and timestamp rendering.
///
/// List readers select ids and then reuse the per-id reader rather than
/// duplicating each entity's column list, which would drift.
#[cfg(test)]
impl CoreStore {
    async fn ids(&self, table: &str) -> Vec<String> {
        self.ids_by(table, "id").await
    }

    /// Primary keys of `table`, for tables whose key column is not `id`.
    async fn ids_by(&self, table: &str, key: &str) -> Vec<String> {
        let client = self.connection().await.unwrap();
        client
            .query(
                &format!("SELECT {key} AS id FROM {table} ORDER BY {key}"),
                &[],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get::<_, String>("id"))
            .collect()
    }

    pub(crate) async fn agent_runtime(&self, id: &str) -> Option<AgentRuntime> {
        let client = self.connection().await.unwrap();
        select_agent_runtime(&**client, id).await.unwrap()
    }

    pub(crate) async fn all_agent_runtimes(&self) -> Vec<AgentRuntime> {
        let mut out = Vec::new();
        for id in self.ids("agent_runtimes").await {
            out.push(self.agent_runtime(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn user_by_email(&self, email: &str) -> Option<CoreUser> {
        let client = self.connection().await.unwrap();
        select_user_by_email(&**client, email).await.unwrap()
    }

    pub(crate) async fn personal_org_by_owner(
        &self,
        owner_user_id: &str,
    ) -> Option<CustomerOrganization> {
        let client = self.connection().await.unwrap();
        select_personal_org_by_owner(&**client, owner_user_id)
            .await
            .unwrap()
    }

    pub(crate) async fn project(&self, id: &str) -> Option<Project> {
        let client = self.connection().await.unwrap();
        select_project(&**client, id).await.unwrap()
    }

    pub(crate) async fn all_projects(&self) -> Vec<Project> {
        let mut out = Vec::new();
        for id in self.ids("projects").await {
            out.push(self.project(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn finite_private_grant(&self, id: &str) -> Option<FinitePrivateGrant> {
        let client = self.connection().await.unwrap();
        select_finite_private_grant(&**client, id, false)
            .await
            .unwrap()
    }

    pub(crate) async fn runtime_artifact_row(&self, id: &str) -> Option<RuntimeArtifact> {
        let client = self.connection().await.unwrap();
        select_runtime_artifact(&**client, id).await.unwrap()
    }

    /// Column list mirrors the production lease/read queries so a test decodes
    /// the row exactly as production does.
    pub(crate) async fn agent_creation_request(&self, id: &str) -> Option<AgentCreationRequest> {
        let client = self.connection().await.unwrap();
        client
            .query_opt(
                "SELECT id, customer_org_id, owner_user_id, project_id, idempotency_key,
                        display_name, runner_class, hosting_tier, placement_runner_class,
                        runtime_resource_class, desired_runtime_artifact_id, runtime_spec,
                        target_source_host_id, relocation_spec,
                        profile_picture_url, owner_chat_account_id, status, requested_launch_code, agent_runtime_id,
                        runner_id, lease_token, core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message,
                        core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
                 FROM agent_creation_requests WHERE id = $1",
                &[&id],
            )
            .await
            .unwrap()
            .map(|row| agent_creation_request_from_row(&row).unwrap())
    }

    pub(crate) async fn all_agent_creation_requests(&self) -> Vec<AgentCreationRequest> {
        let mut out = Vec::new();
        for id in self.ids("agent_creation_requests").await {
            out.push(self.agent_creation_request(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn finite_private_api_key(&self, id: &str) -> Option<FinitePrivateApiKey> {
        let client = self.connection().await.unwrap();
        client
            .query_opt(
                "SELECT id, grant_id, project_id, agent_runtime_id, key_hash, status,
                        core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
                 FROM finite_private_api_keys WHERE id = $1",
                &[&id],
            )
            .await
            .unwrap()
            .map(|row| finite_private_api_key_from_row(&row).unwrap())
    }

    pub(crate) async fn all_finite_private_api_keys(&self) -> Vec<FinitePrivateApiKey> {
        let mut out = Vec::new();
        for id in self.ids("finite_private_api_keys").await {
            out.push(self.finite_private_api_key(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn all_runtime_control_requests(&self) -> Vec<RuntimeControlRequest> {
        let mut out = Vec::new();
        for id in self.ids("runtime_control_requests").await {
            out.push(self.runtime_control_request(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn user(&self, id: &str) -> Option<CoreUser> {
        let client = self.connection().await.unwrap();
        select_user_by_id(&**client, id).await.unwrap()
    }

    pub(crate) async fn all_users(&self) -> Vec<CoreUser> {
        let mut out = Vec::new();
        for id in self.ids("users").await {
            out.push(self.user(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn provider_operation(&self, id: &str) -> Option<ProviderOperationEnvelope> {
        let client = self.connection().await.unwrap();
        select_provider_operation(&**client, id).await.unwrap()
    }

    pub(crate) async fn finite_private_reservation(
        &self,
        id: &str,
    ) -> Option<FinitePrivateReservation> {
        let client = self.connection().await.unwrap();
        select_finite_private_reservation(&**client, id, false)
            .await
            .unwrap()
    }

    pub(crate) async fn all_finite_private_reservations(&self) -> Vec<FinitePrivateReservation> {
        let mut out = Vec::new();
        for id in self.ids("finite_private_reservations").await {
            out.push(self.finite_private_reservation(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn visible_projects_for_user(&self, user_id: &str) -> Vec<VisibleProject> {
        let client = self.connection().await.unwrap();
        postgres_visible_projects_for_user(&**client, user_id)
            .await
            .unwrap()
    }

    /// The key with this raw material, plus its grant.
    ///
    /// Delegates to the production lookup so a test sees the same
    /// active-key/active-grant semantics the API enforces.
    pub(crate) async fn finite_private_key_and_grant(
        &self,
        raw_key: &str,
    ) -> Option<(FinitePrivateApiKey, FinitePrivateGrant)> {
        let client = self.connection().await.unwrap();
        postgres_finite_private_key_and_grant(&**client, raw_key)
            .await
            .unwrap()
    }

    pub(crate) async fn customer_org(&self, id: &str) -> Option<CustomerOrganization> {
        let client = self.connection().await.unwrap();
        client
            .query_opt(
                "SELECT id, owner_user_id, name, billing_class,
                        core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
                 FROM customer_orgs WHERE id = $1",
                &[&id],
            )
            .await
            .unwrap()
            .map(|row| customer_org_from_row(&row).unwrap())
    }

    pub(crate) async fn all_customer_orgs(&self) -> Vec<CustomerOrganization> {
        let mut out = Vec::new();
        for id in self.ids("customer_orgs").await {
            out.push(self.customer_org(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn customer_billing_account(
        &self,
        org_id: &str,
    ) -> Option<CustomerBillingAccount> {
        let client = self.connection().await.unwrap();
        billing::select_customer_billing_account(&**client, org_id, false)
            .await
            .unwrap()
    }

    pub(crate) async fn agent_creation_entitlement(
        &self,
        org_id: &str,
    ) -> Option<AgentCreationEntitlement> {
        let client = self.connection().await.unwrap();
        select_agent_creation_entitlement_by_org(&**client, org_id)
            .await
            .unwrap()
    }

    pub(crate) async fn active_runtime_for_project(
        &self,
        project_id: &str,
    ) -> Option<AgentRuntime> {
        let client = self.connection().await.unwrap();
        postgres_active_runtime_for_project(&**client, project_id)
            .await
            .unwrap()
    }

    /// Weekly reserved/settled usage for a grant at `now`.
    ///
    /// Mirrors the production window: `now` minus the weekly window seconds.
    pub(crate) async fn finite_private_weekly_usage(
        &self,
        grant_id: &str,
        now: time::OffsetDateTime,
    ) -> CoreResult<(i64, Option<String>)> {
        let window_start = (now - Duration::seconds(crate::FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
            .format(&Rfc3339)?;
        let now = now.format(&Rfc3339)?;
        let client = self.connection().await?;
        postgres_finite_private_weekly_usage(&**client, grant_id, &window_start, &now).await
    }

    /// Run a statement for tests that need to stage durable state the store
    /// API cannot reach (an expired lease, a legacy row).
    pub(crate) async fn exec(&self, sql: &str) {
        let client = self.connection().await.unwrap();
        client
            .batch_execute(sql)
            .await
            .unwrap_or_else(|error| panic!("test statement failed: {error}\n{sql}"));
    }

    pub(crate) async fn table_len(&self, table: &str) -> usize {
        let key = match table {
            "runtime_retirement_snapshots" => "request_id",
            "runtime_relay_credentials" => "agent_runtime_id",
            _ => "id",
        };
        self.ids_by(table, key).await.len()
    }
}
