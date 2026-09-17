use super::*;

impl CoreStore {
    pub async fn admin_runtime_overviews(&self) -> CoreResult<Vec<AdminRuntimeOverview>> {
        let client = self.connection().await?;
        postgres_admin_runtime_overviews(&**client).await
    }

    /// The standing-health poll targets for one host, for the runner's
    /// startup registry reconcile.
    pub async fn runtime_health_targets_for_host(
        &self,
        source_host_id: &str,
    ) -> CoreResult<RuntimeHealthTargetList> {
        let client = self.connection().await?;
        postgres_runtime_health_targets_for_host(&**client, source_host_id).await
    }

    pub async fn record_runtime_health_report(
        &self,
        input: RecordRuntimeHealthReportInput,
    ) -> CoreResult<RuntimeHealthReportAck> {
        let client = self.connection().await?;
        postgres_record_runtime_health_report(&**client, input).await
    }
}

/// Clear the stored health report so the derived status reads `unknown`
/// until the runner's standing poller first reports on the incarnation that
/// just came up, and seed or keep the attribution pin. Runs in the same
/// transaction as the completion it follows.
pub(super) async fn reset_runtime_health<C>(
    client: &C,
    agent_runtime_id: &str,
    pin: HealthPin,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let (keep_pin, seed) = match pin {
        HealthPin::Keep => (true, None),
        HealthPin::Seed(seed) => (false, seed),
    };
    client
        .execute(
            "UPDATE agent_runtimes
             SET health_reported_at = NULL,
                 health_observed_at = NULL,
                 health_ready = NULL,
                 health_reason = NULL,
                 health_report_interval_seconds = NULL,
                 health_reporting_npub = CASE WHEN $2 THEN health_reporting_npub ELSE $3 END
             WHERE id = $1",
            &[&agent_runtime_id, &keep_pin, &seed],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

/// Read the latest stored health report columns off a runtime row that
/// selected them (`health_*`, with the timestamps passed through
/// `core_rfc3339`).
pub(super) fn stored_runtime_health_from_row(row: &Row) -> StoredRuntimeHealth {
    StoredRuntimeHealth {
        reported_at: row.get("health_reported_at"),
        observed_at: row.get("health_observed_at"),
        ready: row.get("health_ready"),
        reason: row.get("health_reason"),
        report_interval_seconds: row
            .get::<_, Option<i32>>("health_report_interval_seconds")
            .map(i64::from),
        reporting_npub: row.get("health_reporting_npub"),
    }
}

/// The runtimes a host's runner polls for standing health each cycle: every
/// live (not offboarding) runtime on that host whose lifecycle latch is not
/// `offline`. Scoped by the runner credential's host, like reports.
async fn postgres_runtime_health_targets_for_host<C>(
    client: &C,
    source_host_id: &str,
) -> CoreResult<RuntimeHealthTargetList>
where
    C: GenericClient + Sync,
{
    let source_host_id =
        trim_to_option(Some(source_host_id)).ok_or(CoreError::MissingSourceHostId)?;
    let rows = client
        .query(
            "SELECT id AS agent_runtime_id, source_machine_id, contact_endpoint, host_facts,
                    health_reporting_npub, health_report_interval_seconds
             FROM agent_runtimes
             WHERE source_host_id = $1
               AND offboarding_phase IS NULL
               AND COALESCE(host_facts->>'runtime_status', '') <> $2
             ORDER BY id",
            &[&source_host_id, &RuntimeSummaryStatus::Offline.as_str()],
        )
        .await
        .map_err(store_error)?;
    let targets = rows
        .iter()
        .map(|row| {
            let host_facts: HostOwnedRuntimeFacts = json_column(row, "host_facts")?;
            let contact_endpoint = normalize_runtime_contact_endpoint(
                row.get::<_, Option<String>>("contact_endpoint").as_deref(),
            )
            .ok()
            .flatten()
            .or_else(|| {
                host_facts
                    .published_app_urls
                    .iter()
                    .find(|url| url.ends_with("/contact"))
                    .cloned()
            });
            Ok(RuntimeHealthTarget {
                agent_runtime_id: row.get("agent_runtime_id"),
                source_machine_id: row.get("source_machine_id"),
                contact_endpoint,
                agent_npub: row.get("health_reporting_npub"),
                lifecycle_status: host_facts.runtime_status,
                report_interval_seconds: row
                    .get::<_, Option<i32>>("health_report_interval_seconds")
                    .map(i64::from),
            })
        })
        .collect::<CoreResult<Vec<_>>>()?;
    Ok(RuntimeHealthTargetList {
        source_host_id,
        targets,
    })
}

async fn postgres_admin_runtime_overviews<C>(client: &C) -> CoreResult<Vec<AdminRuntimeOverview>>
where
    C: GenericClient + Sync,
{
    let now = current_time_iso()?;
    let rows = client
        .query(
            "SELECT runtime.id AS agent_runtime_id, runtime.project_id, runtime.source_host_id,
                    runtime.source_machine_id, runtime.runtime_artifact_id, runtime.host_facts,
                    runtime.offboarding_phase,
                    core_rfc3339(runtime.updated_at) AS runtime_updated_at,
                    project.display_name AS project_display_name,
                    owner.normalized_email AS owner_email,
                    artifact.version_label AS runtime_artifact_version_label,
                    runtime.runtime_capabilities,
                    core_rfc3339(runtime.health_reported_at) AS health_reported_at,
                    core_rfc3339(runtime.health_observed_at) AS health_observed_at,
                    runtime.health_ready,
                    runtime.health_reason,
                    runtime.health_report_interval_seconds,
                    runtime.health_reporting_npub,
                    EXISTS (
                      SELECT 1 FROM project_runtime_links link
                      WHERE link.agent_runtime_id = runtime.id AND link.active
                    ) AS runtime_link_active,
                    (
                      SELECT COUNT(*) FROM finite_private_api_keys key
                      WHERE key.status = 'active'
                        AND (key.agent_runtime_id = runtime.id OR key.project_id = runtime.project_id)
                    )::BIGINT AS active_finite_private_key_count
             FROM agent_runtimes AS runtime
             LEFT JOIN projects AS project ON project.id = runtime.project_id
             LEFT JOIN users AS owner ON owner.id = project.owner_user_id
             LEFT JOIN runtime_artifacts AS artifact ON artifact.id = runtime.runtime_artifact_id
             ORDER BY runtime.source_host_id, runtime.source_machine_id, runtime.id",
            &[],
        )
        .await
        .map_err(store_error)?;
    rows.iter()
        .map(|row| {
            let host_facts: HostOwnedRuntimeFacts = json_column(row, "host_facts")?;
            let runtime_capabilities: Option<RuntimeCapabilitiesEnvelope> =
                optional_json_column(row, "runtime_capabilities")?
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(json_error)?;
            let offboarding_phase: Option<String> = row.get("offboarding_phase");
            let offboarding_phase = offboarding_phase
                .as_deref()
                .map(|value| {
                    parse_offboarding_phase(value).ok_or_else(|| {
                        CoreError::Store(format!("invalid offboarding phase {value}"))
                    })
                })
                .transpose()?;
            let project_display_name: Option<String> = row.get("project_display_name");
            let runtime_health = project_runtime_health(
                host_facts.runtime_status,
                &stored_runtime_health_from_row(row),
                &now,
            )?;
            Ok(AdminRuntimeOverview {
                project_id: row.get("project_id"),
                project_display_name: project_display_name
                    .unwrap_or_else(|| host_facts.display_name.clone()),
                owner_email: row.get("owner_email"),
                agent_runtime_id: row.get("agent_runtime_id"),
                source_host_id: row.get("source_host_id"),
                source_machine_id: row.get("source_machine_id"),
                runtime_artifact_id: row.get("runtime_artifact_id"),
                runtime_artifact_version_label: row.get("runtime_artifact_version_label"),
                runtime_status: derive_runtime_summary_status(
                    host_facts.runtime_status,
                    &runtime_health,
                ),
                lifecycle_status: host_facts.runtime_status,
                // runtime_status_snapshots has no writer; the wire fields stay
                // serialized as null for dashboard compatibility until the
                // gated table drop and wire-type change land together.
                last_heartbeat_at: None,
                status_updated_at: None,
                runtime_updated_at: row.get("runtime_updated_at"),
                hermes_available: host_facts.hermes_available,
                published_app_urls: host_facts.published_app_urls.clone(),
                active_finite_private_key_count: row.get("active_finite_private_key_count"),
                runtime_link_active: row.get("runtime_link_active"),
                runtime_capabilities: runtime_capabilities
                    .as_ref()
                    .map(|capabilities| *capabilities.v1()),
                offboarding_phase,
                runtime_health,
            })
        })
        .collect()
}

/// Record one runner-ferried standing-readiness report on the runtime row.
/// The source host comes from the runner credential and scopes the UPDATE, so
/// a body naming another host's runtime (or an unknown runtime) misses every
/// row and fails closed as not-found without leaking cross-host existence.
async fn postgres_record_runtime_health_report<C>(
    client: &C,
    input: RecordRuntimeHealthReportInput,
) -> CoreResult<RuntimeHealthReportAck>
where
    C: GenericClient + Sync,
{
    let now = input.now.clone().unwrap_or(current_time_iso()?);
    let agent_runtime_id =
        trim_to_option(Some(&input.agent_runtime_id)).ok_or(CoreError::MissingAgentRuntimeId)?;
    let source_host_id =
        trim_to_option(Some(&input.source_host_id)).ok_or(CoreError::MissingSourceHostId)?;
    let reason = trim_to_option(input.reason.as_deref());
    if reason
        .as_ref()
        .is_some_and(|value| value.chars().count() > MAX_RUNTIME_HEALTH_REPORT_REASON_CHARS)
    {
        return Err(CoreError::InvalidRuntimeHealthReport);
    }
    // The observation time is runner-clock evidence; it must still parse.
    parse_time(&input.observed_at)?;
    let agent_npub = trim_to_option(input.agent_npub.as_deref());
    if agent_npub
        .as_ref()
        .is_some_and(|value| !valid_agent_npub(value))
    {
        return Err(CoreError::InvalidRuntimeHealthReport);
    }
    let interval_seconds = input.report_interval_seconds;
    if interval_seconds.is_some_and(|value| {
        !(RUNTIME_HEALTH_REPORT_MIN_INTERVAL_SECONDS..=RUNTIME_HEALTH_REPORT_MAX_INTERVAL_SECONDS)
            .contains(&value)
    }) {
        return Err(CoreError::InvalidRuntimeHealthReport);
    }
    let interval_seconds = interval_seconds
        .map(i32::try_from)
        .transpose()
        .map_err(|_| CoreError::InvalidRuntimeHealthReport)?;
    // The attribution pin: a report speaks for the runtime only when it
    // presents the principal on record (seeded at completion from the
    // launch-verified principal, or by the first report when the completing
    // runner did not say). Anything else is a reallocated port wearing this
    // runtime's name, and is refused rather than recorded.
    let pinned = client
        .query_opt(
            "SELECT health_reporting_npub FROM agent_runtimes
             WHERE id = $1 AND source_host_id = $2",
            &[&agent_runtime_id, &source_host_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::ProjectRuntimeNotFound)?
        .get::<_, Option<String>>("health_reporting_npub");
    if let Some(pinned) = pinned.as_deref()
        && agent_npub.as_deref() != Some(pinned)
    {
        eprintln!(
            "warning: rejecting health report for runtime {agent_runtime_id} on host \
             {source_host_id}: it presents {} but the Agent Principal on record is {pinned}",
            agent_npub.as_deref().unwrap_or("no principal")
        );
        return Err(CoreError::RuntimeHealthReportPrincipalMismatch);
    }
    let row = client
        .query_opt(
            "UPDATE agent_runtimes
             SET health_reported_at = $3::text::timestamptz,
                 health_observed_at = $4::text::timestamptz,
                 health_ready = $5,
                 health_reason = $6,
                 health_report_interval_seconds = $7,
                 health_reporting_npub = COALESCE(health_reporting_npub, $8)
             WHERE id = $1 AND source_host_id = $2
             RETURNING id",
            &[
                &agent_runtime_id,
                &source_host_id,
                &now,
                &input.observed_at,
                &input.ready,
                &reason,
                &interval_seconds,
                &agent_npub,
            ],
        )
        .await
        .map_err(store_error)?;
    let Some(row) = row else {
        return Err(CoreError::ProjectRuntimeNotFound);
    };
    Ok(RuntimeHealthReportAck {
        agent_runtime_id: row.get("id"),
        recorded_at: now,
    })
}
