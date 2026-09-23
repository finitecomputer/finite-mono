use super::*;
use finite_saas_core::{
    CoreError, RuntimeCapabilitiesV1, RuntimeControlKind, RuntimeHealthProjection,
    RuntimeSummaryStatus,
};
use std::sync::Mutex;
mod arguments;
mod finite_private;
mod rollout_execution;
mod rollout_planning;

/// Serializes the tests that point `FC_CORE_DATABASE_URL` at a database,
/// since the admin commands read it from the process environment.
static DB_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Run `test` with `FC_CORE_DATABASE_URL` set to a migrated database, so a
/// `--dry-run` command exercises the real SQL path.
///
/// The dry-run tests can share one database because every dry run rolls
/// back: none of them can observe another's writes. Returns without running
/// when `FC_CORE_POSTGRES_TEST_URL` is unset, matching the gating the store
/// tests use.
async fn with_dry_run_database<F, Fut>(test: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let Ok(url) = env::var("FC_CORE_POSTGRES_TEST_URL") else {
        return;
    };
    let _guard = DB_ENV_LOCK.lock().await;
    CoreStore::connect(&url)
        .await
        .unwrap()
        .migrate()
        .await
        .unwrap();
    // SAFETY: every test that reads this variable holds DB_ENV_LOCK.
    unsafe { env::set_var("FC_CORE_DATABASE_URL", &url) };
    test().await;
    unsafe { env::remove_var("FC_CORE_DATABASE_URL") };
}

fn rollout_overview(
    project_id: &str,
    artifact_id: &str,
    runtime_link_active: bool,
    runtime_upgrade: bool,
    runtime_status: RuntimeSummaryStatus,
) -> AdminRuntimeOverview {
    AdminRuntimeOverview {
        project_id: project_id.to_string(),
        project_display_name: format!("Agent {project_id}"),
        owner_email: Some("owner@finite.vip".to_string()),
        agent_runtime_id: format!("runtime-{project_id}"),
        source_host_id: "lat1".to_string(),
        source_machine_id: format!("finite-kata-{project_id}"),
        runtime_artifact_id: Some(artifact_id.to_string()),
        runtime_artifact_version_label: Some(artifact_id.to_string()),
        runtime_status,
        lifecycle_status: runtime_status,
        last_heartbeat_at: Some("2026-07-15T01:00:00Z".to_string()),
        status_updated_at: Some("2026-07-15T01:00:00Z".to_string()),
        runtime_updated_at: "2026-07-15T01:00:00Z".to_string(),
        hermes_available: Some(true),
        published_app_urls: Vec::new(),
        active_finite_private_key_count: 1,
        runtime_link_active,
        runtime_capabilities: Some(RuntimeCapabilitiesV1 {
            native_hermes_chat: false,
            restart: true,
            recover_known_good_chat: true,
            runtime_upgrade,
            stop: true,
            runtime_retirement: false,
        }),
        offboarding_phase: None,
        runtime_health: RuntimeHealthProjection::unreported(),
    }
}

fn rollout_input(
    scope: RuntimeArtifactRolloutScope,
    plan_only: bool,
) -> RuntimeArtifactRolloutInput {
    RuntimeArtifactRolloutInput {
        target_artifact_id: "artifact-v2".to_string(),
        source_host_id: "lat1".to_string(),
        admin_email: "admin@finite.vip".to_string(),
        admin_workos_user_id: "workos-admin".to_string(),
        scope,
        expected_binding: None,
        plan_only,
        wait_timeout: Duration::from_secs(1),
    }
}

struct FakeRolloutStore {
    initial_overviews: Vec<AdminRuntimeOverview>,
    refreshed_overviews: Vec<AdminRuntimeOverview>,
    overview_reads: Mutex<usize>,
    enqueued_projects: Mutex<Vec<String>>,
    enqueue_runtime_overrides: BTreeMap<String, String>,
    terminal_statuses: BTreeMap<String, RuntimeControlRequestStatus>,
}

impl FakeRolloutStore {
    fn new(
        initial_overviews: Vec<AdminRuntimeOverview>,
        refreshed_overviews: Vec<AdminRuntimeOverview>,
        terminal_statuses: BTreeMap<String, RuntimeControlRequestStatus>,
    ) -> Self {
        Self {
            initial_overviews,
            refreshed_overviews,
            overview_reads: Mutex::new(0),
            enqueued_projects: Mutex::new(Vec::new()),
            enqueue_runtime_overrides: BTreeMap::new(),
            terminal_statuses,
        }
    }

    fn with_enqueue_runtime_override(mut self, project_id: &str, agent_runtime_id: &str) -> Self {
        self.enqueue_runtime_overrides
            .insert(project_id.to_string(), agent_runtime_id.to_string());
        self
    }

    fn request(
        &self,
        project_id: &str,
        status: RuntimeControlRequestStatus,
    ) -> CoreResult<RuntimeControlRequest> {
        let overview = self
            .initial_overviews
            .iter()
            .find(|overview| overview.project_id == project_id)
            .ok_or(CoreError::ProjectNotFound)?;
        Ok(RuntimeControlRequest {
            id: format!("request-{project_id}"),
            project_id: project_id.to_string(),
            agent_runtime_id: overview.agent_runtime_id.clone(),
            source_host_id: overview.source_host_id.clone(),
            source_machine_id: overview.source_machine_id.clone(),
            requested_by_user_id: Some("user-admin".to_string()),
            kind: RuntimeControlKind::Upgrade,
            target_runtime_artifact_id: Some("artifact-v2".to_string()),
            status,
            failure_stage: (status == RuntimeControlRequestStatus::Failed)
                .then_some(finite_saas_core::RuntimeLifecycleStage::Unknown),
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            failure_message: (status == RuntimeControlRequestStatus::Failed)
                .then(|| "synthetic runner failure".to_string()),
            created_at: "2026-07-15T01:00:00Z".to_string(),
            updated_at: "2026-07-15T01:00:00Z".to_string(),
            completed_at: status
                .is_terminal()
                .then(|| "2026-07-15T01:00:01Z".to_string()),
        })
    }
}

impl RuntimeArtifactRolloutStore for FakeRolloutStore {
    async fn rollout_admin_runtime_overviews(&self) -> CoreResult<Vec<AdminRuntimeOverview>> {
        let mut reads = self.overview_reads.lock().unwrap();
        let overviews = if *reads == 0 {
            self.initial_overviews.clone()
        } else {
            self.refreshed_overviews.clone()
        };
        *reads += 1;
        Ok(overviews)
    }

    async fn rollout_admin_request_runtime_upgrade(
        &self,
        input: AdminRuntimeUpgradeExactInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let actual_runtime_id = self
            .enqueue_runtime_overrides
            .get(&input.project_id)
            .cloned()
            .or_else(|| {
                self.initial_overviews
                    .iter()
                    .find(|overview| overview.project_id == input.project_id)
                    .map(|overview| overview.agent_runtime_id.clone())
            })
            .ok_or(CoreError::ProjectNotFound)?;
        if actual_runtime_id != input.expected_agent_runtime_id {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        self.enqueued_projects
            .lock()
            .unwrap()
            .push(input.project_id.clone());
        self.request(&input.project_id, RuntimeControlRequestStatus::Requested)
    }

    async fn rollout_runtime_control_request(
        &self,
        request_id: &str,
    ) -> CoreResult<RuntimeControlRequest> {
        let project_id = request_id
            .strip_prefix("request-")
            .ok_or(CoreError::RuntimeControlRequestNotFound)?;
        let status = self
            .terminal_statuses
            .get(project_id)
            .copied()
            .unwrap_or(RuntimeControlRequestStatus::Succeeded);
        self.request(project_id, status)
    }
}
