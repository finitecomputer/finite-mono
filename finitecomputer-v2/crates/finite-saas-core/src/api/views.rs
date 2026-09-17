use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeResponse {
    pub email: String,
    pub workos_user_id: String,
    pub projects: Vec<PublicVisibleProject>,
    pub agent_creation_requests: Vec<AgentCreationRequestSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSummaryResponse {
    pub me: MeResponse,
    pub billing: BillingOverview,
    pub finite_private_usage: Option<FinitePrivateUsageStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicProject {
    pub id: String,
    pub display_name: String,
    pub agent_email: Option<String>,
    pub hosting_tier: Option<HostingTier>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Project> for PublicProject {
    fn from(project: Project) -> Self {
        Self {
            id: project.id,
            display_name: project.display_name,
            agent_email: project.agent_email,
            hosting_tier: project.hosting_tier,
            created_at: project.created_at,
            updated_at: project.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRuntimeCapabilities {
    pub restart: bool,
    pub recover_known_good_chat: bool,
    pub runtime_upgrade: bool,
    pub stop: bool,
    pub runtime_retirement: bool,
}

impl From<&RuntimeCapabilitiesEnvelope> for PublicRuntimeCapabilities {
    fn from(capabilities: &RuntimeCapabilitiesEnvelope) -> Self {
        let capabilities = capabilities.v1();
        Self {
            restart: capabilities.restart,
            recover_known_good_chat: capabilities.recover_known_good_chat,
            runtime_upgrade: capabilities.runtime_upgrade,
            stop: capabilities.stop,
            runtime_retirement: capabilities.runtime_retirement,
        }
    }
}

/// The runtime's standing readiness as projected by Core at read time. The
/// raw report fields ride along as evidence; `status` is the derived fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRuntimeHealth {
    pub status: RuntimeHealthStatus,
    #[serde(default)]
    pub reason: Option<String>,
    /// When Core recorded the latest report (freshness is measured from this).
    #[serde(default)]
    pub reported_at: Option<String>,
    /// When the runner last read the runtime (runner clock; evidence only).
    #[serde(default)]
    pub observed_at: Option<String>,
    /// The reporter's cadence; staleness is declared after three intervals.
    #[serde(default)]
    pub report_interval_seconds: Option<i64>,
}

impl From<&RuntimeHealthProjection> for PublicRuntimeHealth {
    fn from(health: &RuntimeHealthProjection) -> Self {
        Self {
            status: health.status,
            reason: health.reason.clone(),
            reported_at: health.reported_at.clone(),
            observed_at: health.observed_at.clone(),
            report_interval_seconds: health.report_interval_seconds,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicAgentRuntime {
    pub id: String,
    pub project_id: String,
    pub contact_endpoint: Option<String>,
    /// Derived at read time from `runtime_health` freshness: `online` only
    /// while a fresh report says ready, `stale` once reports lapse, `unknown`
    /// until the runtime has been reported on. Never a frozen lifecycle
    /// outcome. Older readers keep consuming this field unchanged.
    pub runtime_status: RuntimeSummaryStatus,
    /// The raw lifecycle-latched fact (last control outcome), for operators.
    #[serde(default = "PublicAgentRuntime::default_lifecycle_status")]
    pub lifecycle_status: RuntimeSummaryStatus,
    /// Additive: the standing readiness `runtime_status` derives from.
    #[serde(default)]
    pub runtime_health: Option<PublicRuntimeHealth>,
    pub hermes_available: Option<bool>,
    /// Populated only from Core's persisted, versioned Runtime capability
    /// record. N-1 rows remain absent and Dashboard fails closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_capabilities: Option<PublicRuntimeCapabilities>,
    pub created_at: String,
    pub updated_at: String,
}

impl PublicAgentRuntime {
    fn default_lifecycle_status() -> RuntimeSummaryStatus {
        RuntimeSummaryStatus::Unknown
    }

    /// Project the user-facing runtime from the stored row plus its
    /// read-time health projection. `runtime_status` is derived here, once.
    pub fn project(runtime: AgentRuntime, health: &RuntimeHealthProjection) -> Self {
        let contact_endpoint = public_runtime_contact_endpoint(&runtime);
        let runtime_capabilities = runtime
            .runtime_capabilities
            .as_ref()
            .map(PublicRuntimeCapabilities::from);
        let lifecycle_status = runtime.host_facts.runtime_status;
        Self {
            id: runtime.id,
            project_id: runtime.project_id,
            contact_endpoint,
            runtime_status: derive_runtime_summary_status(lifecycle_status, health),
            lifecycle_status,
            runtime_health: Some(PublicRuntimeHealth::from(health)),
            hermes_available: runtime.host_facts.hermes_available,
            runtime_capabilities,
            created_at: runtime.created_at,
            updated_at: runtime.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicVisibleProject {
    pub project: PublicProject,
    pub runtime: Option<PublicAgentRuntime>,
    pub active_runtime_control: Option<PublicRuntimeControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRuntimeControl {
    pub id: String,
    pub kind: crate::RuntimeControlKind,
    pub status: crate::RuntimeControlRequestStatus,
    /// The named failure stage; present exactly when `status` is `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<crate::RuntimeLifecycleStage>,
    pub retrying: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<VisibleProject> for PublicVisibleProject {
    fn from(project: VisibleProject) -> Self {
        let active_runtime_control =
            project
                .active_runtime_control
                .map(|request| PublicRuntimeControl {
                    id: request.id,
                    kind: request.kind,
                    status: request.status,
                    failure_stage: request.failure_stage,
                    retrying: request.failure_message.is_some(),
                    created_at: request.created_at,
                    updated_at: request.updated_at,
                });
        let runtime_health = project
            .runtime_health
            .unwrap_or_else(RuntimeHealthProjection::unreported);
        Self {
            project: project.project.into(),
            runtime: project
                .runtime
                .map(|runtime| PublicAgentRuntime::project(runtime, &runtime_health)),
            active_runtime_control,
        }
    }
}

pub(super) fn public_runtime_contact_endpoint(runtime: &AgentRuntime) -> Option<String> {
    // `contact_endpoint` is the public contract. Reading the first valid old
    // published URL is an N-1 compatibility bridge for rows created before
    // that field existed; new Runner generations write the explicit fact.
    normalize_runtime_contact_endpoint(runtime.contact_endpoint.as_deref())
        .ok()
        .flatten()
        .or_else(|| {
            runtime
                .host_facts
                .published_app_urls
                .iter()
                .find_map(|url| normalize_runtime_contact_endpoint(Some(url)).ok().flatten())
        })
}

/// LEGACY-ROW GUARD. Projects with `import_candidate_id` set were created by
/// the abandoned 2026-07 existing-host import bridge (machinery deleted; rows
/// may survive in production from its near-ship test run). They never became
/// part of the hosted product surface, so they stay hidden from every
/// user-facing list. Do not remove this filter while such rows exist.
pub(super) fn public_visible_projects(projects: Vec<VisibleProject>) -> Vec<PublicVisibleProject> {
    projects
        .into_iter()
        .filter(|project| project.project.import_candidate_id.is_none())
        .map(PublicVisibleProject::from)
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeRouteResolution {
    pub project_id: String,
    pub runtime_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCreationRequestSummary {
    pub id: String,
    pub project_id: String,
    pub display_name: String,
    pub profile_picture_url: Option<String>,
    pub is_relocation: bool,
    pub status: crate::AgentCreationRequestStatus,
    pub agent_runtime_id: Option<String>,
    pub failure_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<AgentCreationRequest> for AgentCreationRequestSummary {
    fn from(request: AgentCreationRequest) -> Self {
        Self {
            id: request.id,
            project_id: request.project_id,
            display_name: request.display_name,
            profile_picture_url: request.profile_picture_url,
            is_relocation: request.relocation.is_some(),
            status: request.status,
            agent_runtime_id: request.agent_runtime_id,
            failure_message: request.failure_message,
            created_at: request.created_at,
            updated_at: request.updated_at,
        }
    }
}
