use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeArtifactRolloutScope {
    Projects(Vec<String>),
    All { canary_project_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeArtifactRolloutExpectedBinding {
    pub(crate) agent_runtime_id: String,
    pub(crate) source_machine_id: String,
}

#[derive(Debug)]
pub(crate) struct RuntimeArtifactRolloutInput {
    pub(crate) target_artifact_id: String,
    pub(crate) source_host_id: String,
    pub(crate) admin_email: String,
    pub(crate) admin_workos_user_id: String,
    pub(crate) scope: RuntimeArtifactRolloutScope,
    pub(crate) expected_binding: Option<RuntimeArtifactRolloutExpectedBinding>,
    pub(crate) plan_only: bool,
    pub(crate) wait_timeout: Duration,
}

impl TryFrom<RuntimeArtifactRolloutCliArgs> for RuntimeArtifactRolloutInput {
    type Error = anyhow::Error;

    fn try_from(args: RuntimeArtifactRolloutCliArgs) -> Result<Self> {
        let target_artifact_id = required_cli_value(args.artifact_id, "--artifact-id")?;
        let source_host_id = required_cli_value(args.source_host_id, "--source-host-id")?;
        let admin_email = required_cli_value(args.admin_email, "--admin-email")?;
        let admin_workos_user_id =
            required_cli_value(args.admin_workos_user_id, "--admin-workos-user-id")?;
        let mut project_ids = Vec::new();
        let mut seen_project_ids = BTreeSet::new();
        for project_id in args.project_ids {
            let project_id = required_cli_value(project_id, "--project-id")?;
            if seen_project_ids.insert(project_id.clone()) {
                project_ids.push(project_id);
            }
        }
        let explicit_project_count = project_ids.len();
        let scope = match (args.all, project_ids.is_empty(), args.canary_project_id) {
            (false, false, None) => RuntimeArtifactRolloutScope::Projects(project_ids),
            (true, true, Some(canary_project_id)) => RuntimeArtifactRolloutScope::All {
                canary_project_id: required_cli_value(canary_project_id, "--canary-project-id")?,
            },
            _ => bail!(
                "choose exactly one rollout scope: repeat --project-id, or use --all with --canary-project-id"
            ),
        };
        let expected_binding = match (
            args.expected_agent_runtime_id,
            args.expected_source_machine_id,
        ) {
            (None, None) => None,
            (Some(agent_runtime_id), Some(source_machine_id)) => {
                if args.all || explicit_project_count != 1 {
                    bail!(
                        "exact Runtime binding requires exactly one --project-id and cannot be used with --all"
                    );
                }
                Some(RuntimeArtifactRolloutExpectedBinding {
                    agent_runtime_id: required_cli_value(
                        agent_runtime_id,
                        "--expected-agent-runtime-id",
                    )?,
                    source_machine_id: required_cli_value(
                        source_machine_id,
                        "--expected-source-machine-id",
                    )?,
                })
            }
            _ => bail!(
                "--expected-agent-runtime-id and --expected-source-machine-id must be provided together"
            ),
        };
        Ok(Self {
            target_artifact_id,
            source_host_id,
            admin_email,
            admin_workos_user_id,
            scope,
            expected_binding,
            plan_only: args.plan_only,
            wait_timeout: Duration::from_secs(args.wait_timeout_seconds),
        })
    }
}

pub(crate) fn required_cli_value(value: String, flag: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{flag} must not be empty");
    }
    Ok(value.to_string())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct RuntimeArtifactRolloutPlanEntry {
    pub(crate) project_id: String,
    pub(crate) agent_runtime_id: String,
    pub(crate) project_display_name: String,
    pub(crate) source_host_id: String,
    pub(crate) source_machine_id: String,
    pub(crate) target_artifact_id: String,
}

impl RuntimeArtifactRolloutPlanEntry {
    pub(crate) fn from_overview(overview: &AdminRuntimeOverview, target_artifact_id: &str) -> Self {
        Self {
            project_id: overview.project_id.clone(),
            agent_runtime_id: overview.agent_runtime_id.clone(),
            project_display_name: overview.project_display_name.clone(),
            source_host_id: overview.source_host_id.clone(),
            source_machine_id: overview.source_machine_id.clone(),
            target_artifact_id: target_artifact_id.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct RuntimeArtifactRolloutSkippedEntry {
    pub(crate) project_id: String,
    pub(crate) agent_runtime_id: Option<String>,
    pub(crate) project_display_name: Option<String>,
    pub(crate) source_host_id: Option<String>,
    pub(crate) source_machine_id: Option<String>,
    pub(crate) reason: String,
}

impl RuntimeArtifactRolloutSkippedEntry {
    pub(crate) fn for_overview(overview: &AdminRuntimeOverview, reason: &str) -> Self {
        Self {
            project_id: overview.project_id.clone(),
            agent_runtime_id: Some(overview.agent_runtime_id.clone()),
            project_display_name: Some(overview.project_display_name.clone()),
            source_host_id: Some(overview.source_host_id.clone()),
            source_machine_id: Some(overview.source_machine_id.clone()),
            reason: reason.to_string(),
        }
    }

    pub(crate) fn missing_project(project_id: String) -> Self {
        Self {
            project_id,
            agent_runtime_id: None,
            project_display_name: None,
            source_host_id: None,
            source_machine_id: None,
            reason: "project_not_found".to_string(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuntimeArtifactRolloutPlan {
    pub(crate) planned: Vec<RuntimeArtifactRolloutPlanEntry>,
    pub(crate) skipped: Vec<RuntimeArtifactRolloutSkippedEntry>,
    pub(crate) execution_blocked_reason: Option<String>,
}

pub(crate) fn consider_runtime_for_rollout(
    overview: &AdminRuntimeOverview,
    source_host_id: &str,
    target_artifact_id: &str,
    planned: &mut Vec<RuntimeArtifactRolloutPlanEntry>,
    skipped: &mut Vec<RuntimeArtifactRolloutSkippedEntry>,
) {
    let skip_reason = offboarding_rollout_skip_reason(overview.offboarding_phase).or_else(|| {
        if overview.source_host_id != source_host_id {
            Some("wrong_source_host")
        } else if !overview.runtime_link_active {
            Some("inactive_runtime_link")
        } else if overview.runtime_artifact_id.as_deref() == Some(target_artifact_id) {
            Some("already_on_target_artifact")
        } else if !overview
            .runtime_capabilities
            .is_some_and(|capabilities| capabilities.runtime_upgrade)
        {
            Some("runtime_upgrade_not_supported")
        } else {
            None
        }
    });
    if let Some(reason) = skip_reason {
        skipped.push(RuntimeArtifactRolloutSkippedEntry::for_overview(
            overview, reason,
        ));
    } else {
        planned.push(RuntimeArtifactRolloutPlanEntry::from_overview(
            overview,
            target_artifact_id,
        ));
    }
}

/// The recorded offboarding phase classifies a Runtime out of every rollout:
/// an in-progress offboarding is skipped with its exact phase (never planned,
/// never manually listed), while a terminal `archived` Runtime falls through
/// to the ordinary inactive-link classification below.
pub(crate) fn offboarding_rollout_skip_reason(
    phase: Option<OffboardingPhase>,
) -> Option<&'static str> {
    match phase {
        Some(OffboardingPhase::RetirementRequested) => Some("offboarding_retirement_requested"),
        Some(OffboardingPhase::ReceiptVerified) => Some("offboarding_receipt_verified"),
        Some(OffboardingPhase::ComputeRemoved) => Some("offboarding_compute_removed"),
        Some(OffboardingPhase::LinkDeactivated) => Some("offboarding_link_deactivated"),
        Some(OffboardingPhase::Archived) | None => None,
    }
}

pub(crate) fn plan_runtime_artifact_rollout(
    mut overviews: Vec<AdminRuntimeOverview>,
    scope: &RuntimeArtifactRolloutScope,
    source_host_id: &str,
    target_artifact_id: &str,
) -> RuntimeArtifactRolloutPlan {
    overviews.sort_by(|left, right| {
        left.project_id
            .cmp(&right.project_id)
            .then_with(|| left.agent_runtime_id.cmp(&right.agent_runtime_id))
    });

    let mut planned = Vec::new();
    let mut skipped = Vec::new();

    match scope {
        RuntimeArtifactRolloutScope::Projects(project_ids) => {
            for project_id in project_ids {
                let matching = overviews
                    .iter()
                    .filter(|overview| overview.project_id == *project_id)
                    .collect::<Vec<_>>();
                if matching.is_empty() {
                    skipped.push(RuntimeArtifactRolloutSkippedEntry::missing_project(
                        project_id.clone(),
                    ));
                } else {
                    let matching_on_host = matching
                        .iter()
                        .copied()
                        .filter(|overview| overview.source_host_id == source_host_id)
                        .collect::<Vec<_>>();
                    if matching_on_host.is_empty() {
                        skipped.push(RuntimeArtifactRolloutSkippedEntry::for_overview(
                            matching[0],
                            "wrong_source_host",
                        ));
                    } else {
                        for overview in matching_on_host {
                            consider_runtime_for_rollout(
                                overview,
                                source_host_id,
                                target_artifact_id,
                                &mut planned,
                                &mut skipped,
                            );
                        }
                    }
                }
            }
        }
        RuntimeArtifactRolloutScope::All { .. } => {
            for overview in overviews
                .iter()
                .filter(|overview| overview.source_host_id == source_host_id)
            {
                consider_runtime_for_rollout(
                    overview,
                    source_host_id,
                    target_artifact_id,
                    &mut planned,
                    &mut skipped,
                );
            }
        }
    }

    let execution_blocked_reason = match scope {
        RuntimeArtifactRolloutScope::Projects(_) => {
            let unavailable_projects = skipped
                .iter()
                .filter(|entry| {
                    entry.reason == "project_not_found" || entry.reason == "wrong_source_host"
                })
                .map(|entry| entry.project_id.as_str())
                .collect::<Vec<_>>();
            (!unavailable_projects.is_empty()).then(|| {
                format!(
                    "explicitly requested project(s) were not found on source host {source_host_id}: {}",
                    unavailable_projects.join(", ")
                )
            })
        }
        RuntimeArtifactRolloutScope::All { canary_project_id } => {
            planned.sort_by(|left, right| {
                (left.project_id != *canary_project_id)
                    .cmp(&(right.project_id != *canary_project_id))
                    .then_with(|| left.project_id.cmp(&right.project_id))
                    .then_with(|| left.agent_runtime_id.cmp(&right.agent_runtime_id))
            });
            let canary_is_planned = planned
                .iter()
                .any(|entry| entry.project_id == *canary_project_id);
            let canary_is_already_ready = overviews.iter().any(|overview| {
                overview.project_id == *canary_project_id
                    && overview.source_host_id == source_host_id
                    && overview.runtime_link_active
                    && overview.runtime_artifact_id.as_deref() == Some(target_artifact_id)
                    && overview.lifecycle_status == RuntimeSummaryStatus::Online
            });
            (!canary_is_planned && !canary_is_already_ready).then(|| {
                format!(
                    "canary project {canary_project_id} has no eligible active Runtime and is not already online on the target artifact"
                )
            })
        }
    };
    skipped.sort_by(|left, right| {
        left.project_id
            .cmp(&right.project_id)
            .then_with(|| left.agent_runtime_id.cmp(&right.agent_runtime_id))
            .then_with(|| left.reason.cmp(&right.reason))
    });

    RuntimeArtifactRolloutPlan {
        planned,
        skipped,
        execution_blocked_reason,
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeArtifactRolloutOutcomeStatus {
    Succeeded,
    EnqueueFailed,
    RequestFailed,
    TimedOut,
    PollFailed,
    PostconditionFailed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct RuntimeArtifactRolloutOutcome {
    pub(crate) project_id: String,
    pub(crate) agent_runtime_id: String,
    pub(crate) request_id: Option<String>,
    pub(crate) status: RuntimeArtifactRolloutOutcomeStatus,
    pub(crate) detail: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct RuntimeArtifactRolloutReport {
    pub(crate) target_artifact_id: String,
    pub(crate) source_host_id: String,
    pub(crate) plan_only: bool,
    pub(crate) planned: Vec<RuntimeArtifactRolloutPlanEntry>,
    pub(crate) skipped: Vec<RuntimeArtifactRolloutSkippedEntry>,
    pub(crate) outcomes: Vec<RuntimeArtifactRolloutOutcome>,
    pub(crate) halted: bool,
    pub(crate) halted_reason: Option<String>,
}

pub(crate) trait RuntimeArtifactRolloutStore {
    async fn rollout_admin_runtime_overviews(&self) -> CoreResult<Vec<AdminRuntimeOverview>>;
    async fn rollout_admin_request_runtime_upgrade(
        &self,
        input: AdminRuntimeUpgradeExactInput,
    ) -> CoreResult<RuntimeControlRequest>;
    async fn rollout_runtime_control_request(
        &self,
        request_id: &str,
    ) -> CoreResult<RuntimeControlRequest>;
}

impl RuntimeArtifactRolloutStore for CoreStore {
    async fn rollout_admin_runtime_overviews(&self) -> CoreResult<Vec<AdminRuntimeOverview>> {
        self.admin_runtime_overviews().await
    }

    async fn rollout_admin_request_runtime_upgrade(
        &self,
        input: AdminRuntimeUpgradeExactInput,
    ) -> CoreResult<RuntimeControlRequest> {
        self.admin_request_runtime_upgrade_exact(input).await
    }

    async fn rollout_runtime_control_request(
        &self,
        request_id: &str,
    ) -> CoreResult<RuntimeControlRequest> {
        self.runtime_control_request(request_id).await
    }
}

pub(crate) enum RuntimeArtifactRolloutWaitResult {
    Terminal(RuntimeControlRequest),
    TimedOut(RuntimeControlRequest),
}

pub(crate) async fn wait_for_runtime_artifact_upgrade<S: RuntimeArtifactRolloutStore>(
    store: &S,
    request: RuntimeControlRequest,
    wait_timeout: Duration,
    poll_interval: Duration,
) -> CoreResult<RuntimeArtifactRolloutWaitResult> {
    let deadline = Instant::now() + wait_timeout;
    let mut last = request;
    loop {
        last = store.rollout_runtime_control_request(&last.id).await?;
        if last.status.is_terminal() {
            return Ok(RuntimeArtifactRolloutWaitResult::Terminal(last));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(RuntimeArtifactRolloutWaitResult::TimedOut(last));
        }
        sleep(poll_interval.min(deadline.saturating_duration_since(now))).await;
    }
}

pub(crate) fn rollout_outcome(
    entry: &RuntimeArtifactRolloutPlanEntry,
    request_id: Option<String>,
    status: RuntimeArtifactRolloutOutcomeStatus,
    detail: Option<String>,
) -> RuntimeArtifactRolloutOutcome {
    RuntimeArtifactRolloutOutcome {
        project_id: entry.project_id.clone(),
        agent_runtime_id: entry.agent_runtime_id.clone(),
        request_id,
        status,
        detail,
    }
}

pub(crate) async fn runtime_artifact_rollout<S: RuntimeArtifactRolloutStore>(
    store: &S,
    input: RuntimeArtifactRolloutInput,
    poll_interval: Duration,
) -> Result<RuntimeArtifactRolloutReport> {
    let plan = plan_runtime_artifact_rollout(
        store.rollout_admin_runtime_overviews().await?,
        &input.scope,
        &input.source_host_id,
        &input.target_artifact_id,
    );
    let exact_binding_mismatch = input.expected_binding.as_ref().and_then(|expected| {
        (plan.planned.len() != 1
            || plan.planned[0].agent_runtime_id != expected.agent_runtime_id
            || plan.planned[0].source_machine_id != expected.source_machine_id)
            .then(|| {
                "active Runtime no longer matches the exact preflighted rollout binding".to_string()
            })
    });
    let mut report = RuntimeArtifactRolloutReport {
        target_artifact_id: input.target_artifact_id.clone(),
        source_host_id: input.source_host_id.clone(),
        plan_only: input.plan_only,
        planned: plan.planned.clone(),
        skipped: plan.skipped,
        outcomes: Vec::new(),
        halted: plan.execution_blocked_reason.is_some() || exact_binding_mismatch.is_some(),
        halted_reason: plan.execution_blocked_reason.or(exact_binding_mismatch),
    };
    if input.plan_only || report.halted {
        return Ok(report);
    }

    for entry in &plan.planned {
        let request = match store
            .rollout_admin_request_runtime_upgrade(AdminRuntimeUpgradeExactInput {
                admin_verified_email: input.admin_email.clone(),
                admin_workos_user_id: input.admin_workos_user_id.clone(),
                project_id: entry.project_id.clone(),
                expected_agent_runtime_id: entry.agent_runtime_id.clone(),
                expected_source_host_id: entry.source_host_id.clone(),
                expected_source_machine_id: entry.source_machine_id.clone(),
                target_runtime_artifact_id: input.target_artifact_id.clone(),
                now: None,
            })
            .await
        {
            Ok(request) => request,
            Err(error) => {
                let detail = format!("failed to enqueue upgrade: {error}");
                report.outcomes.push(rollout_outcome(
                    entry,
                    None,
                    RuntimeArtifactRolloutOutcomeStatus::EnqueueFailed,
                    Some(detail.clone()),
                ));
                report.halted = true;
                report.halted_reason = Some(detail);
                return Ok(report);
            }
        };
        let request_id = request.id.clone();
        let terminal = match wait_for_runtime_artifact_upgrade(
            store,
            request,
            input.wait_timeout,
            poll_interval,
        )
        .await
        {
            Ok(RuntimeArtifactRolloutWaitResult::Terminal(request)) => request,
            Ok(RuntimeArtifactRolloutWaitResult::TimedOut(request)) => {
                let detail = format!(
                    "timed out while request {} remained {}; the live request was not cancelled",
                    request.id,
                    request.status.as_str()
                );
                report.outcomes.push(rollout_outcome(
                    entry,
                    Some(request.id),
                    RuntimeArtifactRolloutOutcomeStatus::TimedOut,
                    Some(detail.clone()),
                ));
                report.halted = true;
                report.halted_reason = Some(detail);
                return Ok(report);
            }
            Err(error) => {
                let detail = format!("failed to read exact request {request_id}: {error}");
                report.outcomes.push(rollout_outcome(
                    entry,
                    Some(request_id),
                    RuntimeArtifactRolloutOutcomeStatus::PollFailed,
                    Some(detail.clone()),
                ));
                report.halted = true;
                report.halted_reason = Some(detail);
                return Ok(report);
            }
        };
        if terminal.status == RuntimeControlRequestStatus::Failed {
            let detail = terminal
                .failure_message
                .clone()
                .unwrap_or_else(|| "runtime upgrade request failed without detail".to_string());
            report.outcomes.push(rollout_outcome(
                entry,
                Some(terminal.id),
                RuntimeArtifactRolloutOutcomeStatus::RequestFailed,
                Some(detail.clone()),
            ));
            report.halted = true;
            report.halted_reason = Some(detail);
            return Ok(report);
        }

        let overviews = match store.rollout_admin_runtime_overviews().await {
            Ok(overviews) => overviews,
            Err(error) => {
                let detail = format!("failed to verify upgraded Runtime overview: {error}");
                report.outcomes.push(rollout_outcome(
                    entry,
                    Some(terminal.id),
                    RuntimeArtifactRolloutOutcomeStatus::PostconditionFailed,
                    Some(detail.clone()),
                ));
                report.halted = true;
                report.halted_reason = Some(detail);
                return Ok(report);
            }
        };
        let postcondition_met = overviews.iter().any(|overview| {
            overview.project_id == entry.project_id
                && overview.agent_runtime_id == entry.agent_runtime_id
                && overview.source_host_id == entry.source_host_id
                && overview.source_machine_id == entry.source_machine_id
                && overview.runtime_artifact_id.as_deref()
                    == Some(input.target_artifact_id.as_str())
                // The lifecycle latch, not the health-derived status: right
                // after completion the standing poller has not reported yet.
                && overview.lifecycle_status == RuntimeSummaryStatus::Online
        });
        if !postcondition_met {
            let detail = format!(
                "request {} succeeded, but Runtime {} is not online on artifact {}",
                terminal.id, entry.agent_runtime_id, input.target_artifact_id
            );
            report.outcomes.push(rollout_outcome(
                entry,
                Some(terminal.id),
                RuntimeArtifactRolloutOutcomeStatus::PostconditionFailed,
                Some(detail.clone()),
            ));
            report.halted = true;
            report.halted_reason = Some(detail);
            return Ok(report);
        }
        report.outcomes.push(rollout_outcome(
            entry,
            Some(terminal.id),
            RuntimeArtifactRolloutOutcomeStatus::Succeeded,
            None,
        ));
    }
    Ok(report)
}

pub(crate) async fn runtime_artifact_rollout_command(
    args: RuntimeArtifactRolloutCliArgs,
) -> Result<()> {
    let input = RuntimeArtifactRolloutInput::try_from(args)?;
    let store = postgres_store_from_env(ImportMode::Commit).await?;
    let report = runtime_artifact_rollout(&store, input, Duration::from_secs(2)).await?;
    let halted = report.halted;
    print_json(&report)?;
    if halted {
        bail!("runtime artifact rollout halted; see JSON report");
    }
    Ok(())
}
