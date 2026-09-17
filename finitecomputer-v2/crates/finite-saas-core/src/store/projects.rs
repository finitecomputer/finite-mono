use super::*;

impl CoreStore {
    pub async fn visible_projects_for_workos_user(
        &self,
        workos_user_id: &str,
    ) -> CoreResult<Vec<VisibleProject>> {
        let client = self.connection().await?;
        postgres_visible_projects_for_workos_user(&**client, workos_user_id).await
    }

    /// Resolve only the exact current runtime of a self-service Project owned
    /// by this WorkOS user. Location discovery never grants membership or uses
    /// source-machine aliases, and cannot revive an inactive runtime link.
    pub(crate) async fn owned_runtime_source_host(
        &self,
        runtime_id: &str,
        workos_user_id: &str,
    ) -> CoreResult<Option<String>> {
        let client = self.connection().await?;
        let row = client
            .query_opt(
                "SELECT runtime.source_host_id
             FROM agent_runtimes AS runtime
             JOIN projects AS project ON project.id = runtime.project_id
             JOIN users AS owner ON owner.id = project.owner_user_id
             JOIN project_runtime_links AS link
               ON link.project_id = project.id AND link.agent_runtime_id = runtime.id
             WHERE runtime.id = $1 AND owner.workos_user_id = $2
               AND link.active AND project.import_candidate_id IS NULL",
                &[&runtime_id, &workos_user_id],
            )
            .await
            .map_err(store_error)?;
        Ok(row.map(|row| row.get(0)))
    }

    pub async fn agent_creation_requests_for_workos_user(
        &self,
        workos_user_id: &str,
    ) -> CoreResult<Vec<AgentCreationRequest>> {
        let client = self.connection().await?;
        postgres_agent_creation_requests_for_workos_user(&**client, workos_user_id).await
    }
}

async fn postgres_visible_projects_for_workos_user<C>(
    client: &C,
    workos_user_id: &str,
) -> CoreResult<Vec<VisibleProject>>
where
    C: GenericClient + Sync,
{
    let Some(user_id) = client
        .query_opt(
            "SELECT id FROM users WHERE workos_user_id = $1",
            &[&workos_user_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| row.get::<_, String>("id"))
    else {
        return Ok(Vec::new());
    };
    postgres_visible_projects_for_user(client, &user_id).await
}

/// Visible projects for an already-resolved internal user id.
///
/// Split out so the WorkOS-keyed entry point and callers that already hold the
/// internal id share one query.
pub(super) async fn postgres_visible_projects_for_user<C>(
    client: &C,
    user_id: &str,
) -> CoreResult<Vec<VisibleProject>>
where
    C: GenericClient + Sync,
{
    let now = current_time_iso()?;
    let rows = client
        .query(
            "SELECT project.id AS project_id, project.customer_org_id, project.owner_user_id,
                    project.display_name, project.agent_email, project.import_candidate_id,
                    project.hosting_tier,
                    project.placement_runner_class, project.runtime_resource_class,
                    core_rfc3339(project.created_at) AS created_at,
                    core_rfc3339(project.updated_at) AS updated_at,
                    runtime.id AS runtime_id, runtime.project_id AS runtime_project_id,
                    runtime.source_host_id, runtime.source_machine_id, runtime.source_import_key,
                    runtime.runtime_artifact_id, runtime.state_schema_version,
                    runtime.placement_runner_class AS runtime_placement_runner_class,
                    runtime.runtime_resource_class AS runtime_runtime_resource_class,
                    runtime.provider_runtime_handle, runtime.provider_runtime_handle_history,
                    runtime.contact_endpoint, runtime.runtime_capabilities,
                    runtime.host_facts, core_rfc3339(runtime.created_at) AS runtime_created_at,
                    core_rfc3339(runtime.updated_at) AS runtime_updated_at,
                    core_rfc3339(runtime.health_reported_at) AS health_reported_at,
                    core_rfc3339(runtime.health_observed_at) AS health_observed_at,
                    runtime.health_ready, runtime.health_reason,
                    runtime.health_report_interval_seconds, runtime.health_reporting_npub,
                    control.id AS control_id, control.project_id AS control_project_id,
                    control.agent_runtime_id AS control_agent_runtime_id,
                    control.source_host_id AS control_source_host_id,
                    control.source_machine_id AS control_source_machine_id,
                    control.requested_by_user_id AS control_requested_by_user_id,
                    control.kind AS control_kind,
                    control.target_runtime_artifact_id AS control_target_runtime_artifact_id,
                    control.status AS control_status,
                    control.failure_stage AS control_failure_stage,
                    control.runner_id AS control_runner_id,
                    control.lease_token AS control_lease_token,
                    core_rfc3339(control.lease_expires_at) AS control_lease_expires_at,
                    control.failure_message AS control_failure_message,
                    core_rfc3339(control.created_at) AS control_created_at,
                    core_rfc3339(control.updated_at) AS control_updated_at,
                    core_rfc3339(control.completed_at) AS control_completed_at
             FROM project_room_memberships AS membership
             JOIN chat_identities AS identity ON identity.id = membership.chat_identity_id
             JOIN projects AS project ON project.id = membership.project_id
             LEFT JOIN project_runtime_links AS link
               ON link.project_id = project.id AND link.active
             LEFT JOIN agent_runtimes AS runtime ON runtime.id = link.agent_runtime_id
             LEFT JOIN LATERAL (
               SELECT request.*
               FROM runtime_control_requests AS request
               WHERE request.agent_runtime_id = runtime.id
                 AND request.status IN ('requested', 'launching', 'compute_up', 'ready')
               ORDER BY request.created_at, request.id
               LIMIT 1
             ) AS control ON TRUE
             WHERE identity.user_id = $1
               AND membership.archived_at IS NULL
               AND NOT EXISTS (
                 SELECT 1 FROM agent_creation_requests hidden
                 WHERE hidden.project_id = project.id
                   AND hidden.status = 'cancelled'
                   AND hidden.agent_runtime_id IS NULL
               )
             ORDER BY project.created_at, project.id",
            &[&user_id],
        )
        .await
        .map_err(store_error)?;
    rows.into_iter()
        .map(|row| {
            let project = Project {
                id: row.get("project_id"),
                customer_org_id: row.get("customer_org_id"),
                owner_user_id: row.get("owner_user_id"),
                display_name: row.get("display_name"),
                agent_email: row.get("agent_email"),
                import_candidate_id: row.get("import_candidate_id"),
                hosting_tier: optional_hosting_tier_column(&row, "hosting_tier")?,
                placement: optional_runtime_placement_columns(
                    &row,
                    "placement_runner_class",
                    "runtime_resource_class",
                )?,
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            };
            let runtime = row
                .get::<_, Option<String>>("runtime_id")
                .map(|id| {
                    Ok::<AgentRuntime, CoreError>(AgentRuntime {
                        id,
                        project_id: row.get("runtime_project_id"),
                        source_host_id: row.get("source_host_id"),
                        source_machine_id: row.get("source_machine_id"),
                        source_import_key: row.get("source_import_key"),
                        runtime_artifact_id: row.get("runtime_artifact_id"),
                        state_schema_version: row.get("state_schema_version"),
                        placement: optional_runtime_placement_columns(
                            &row,
                            "runtime_placement_runner_class",
                            "runtime_runtime_resource_class",
                        )?,
                        provider_runtime_handle: optional_json_column(
                            &row,
                            "provider_runtime_handle",
                        )?
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(json_error)?,
                        provider_runtime_handle_history: optional_json_column(
                            &row,
                            "provider_runtime_handle_history",
                        )?
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(json_error)?
                        .unwrap_or_default(),
                        contact_endpoint: row.get("contact_endpoint"),
                        runtime_capabilities: optional_json_column(&row, "runtime_capabilities")?
                            .map(serde_json::from_value)
                            .transpose()
                            .map_err(json_error)?,
                        host_facts: json_column(&row, "host_facts")?,
                        created_at: row.get("runtime_created_at"),
                        updated_at: row.get("runtime_updated_at"),
                    })
                })
                .transpose()?;
            let runtime_health = runtime
                .as_ref()
                .map(|runtime| {
                    project_runtime_health(
                        runtime.host_facts.runtime_status,
                        &stored_runtime_health_from_row(&row),
                        &now,
                    )
                })
                .transpose()?;
            let active_runtime_control = row
                .get::<_, Option<String>>("control_id")
                .map(|id| {
                    let status = parse_runtime_control_request_status(
                        &row.get::<_, String>("control_status"),
                    )
                    .ok_or_else(|| CoreError::Store("invalid runtime control status".into()))?;
                    let failure_stage = if status == RuntimeControlRequestStatus::Failed {
                        Some(
                            parse_runtime_lifecycle_stage(
                                &row.get::<_, String>("control_failure_stage"),
                            )
                            .ok_or_else(|| {
                                CoreError::Store("invalid runtime control failure stage".into())
                            })?,
                        )
                    } else {
                        None
                    };
                    Ok::<RuntimeControlRequest, CoreError>(RuntimeControlRequest {
                        id,
                        project_id: row.get("control_project_id"),
                        agent_runtime_id: row.get("control_agent_runtime_id"),
                        source_host_id: row.get("control_source_host_id"),
                        source_machine_id: row.get("control_source_machine_id"),
                        requested_by_user_id: row.get("control_requested_by_user_id"),
                        kind: parse_runtime_control_kind(&row.get::<_, String>("control_kind"))
                            .ok_or_else(|| {
                                CoreError::Store("invalid runtime control kind".into())
                            })?,
                        target_runtime_artifact_id: row.get("control_target_runtime_artifact_id"),
                        status,
                        failure_stage,
                        runner_id: row.get("control_runner_id"),
                        lease_token: row.get("control_lease_token"),
                        lease_expires_at: row.get("control_lease_expires_at"),
                        failure_message: row.get("control_failure_message"),
                        created_at: row.get("control_created_at"),
                        updated_at: row.get("control_updated_at"),
                        completed_at: row.get("control_completed_at"),
                    })
                })
                .transpose()?;
            Ok(VisibleProject {
                project,
                runtime,
                runtime_health,
                active_runtime_control,
            })
        })
        .collect()
}

async fn postgres_agent_creation_requests_for_workos_user<C>(
    client: &C,
    workos_user_id: &str,
) -> CoreResult<Vec<AgentCreationRequest>>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT request.id, request.customer_org_id, request.owner_user_id,
                    request.project_id, request.idempotency_key, request.display_name,
                    request.runner_class, request.hosting_tier,
                    request.placement_runner_class, request.runtime_resource_class,
                    request.desired_runtime_artifact_id, request.runtime_spec, request.target_source_host_id, request.relocation_spec,
                    request.profile_picture_url,
                    request.owner_chat_account_id,
                    request.status, request.requested_launch_code, request.agent_runtime_id,
                    request.runner_id, request.lease_token, core_rfc3339(request.lease_expires_at) AS lease_expires_at,
                    request.failure_message, core_rfc3339(request.created_at) AS created_at, core_rfc3339(request.updated_at) AS updated_at
             FROM agent_creation_requests AS request
             JOIN users AS owner ON owner.id = request.owner_user_id
             WHERE owner.workos_user_id = $1
             ORDER BY request.created_at, request.id",
            &[&workos_user_id],
        )
        .await
        .map_err(store_error)?;
    rows.iter().map(agent_creation_request_from_row).collect()
}

pub(super) async fn select_project<C>(client: &C, project_id: &str) -> CoreResult<Option<Project>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, customer_org_id, owner_user_id, display_name, agent_email,
                    import_candidate_id,
                    hosting_tier, placement_runner_class, runtime_resource_class,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
             FROM projects WHERE id = $1",
            &[&project_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| project_from_row(&row))
        .transpose()
}

pub(super) async fn ensure_hosted_web_membership_row<C>(
    client: &C,
    user: &CoreUser,
    project_id: &str,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let identity_id = chat_identity_id_for_user(&user.id);
    client
        .execute(
            "INSERT INTO chat_identities (id, user_id, kind, device_id, created_at)
             VALUES ($1, $2, 'hosted_web', 'dashboard-bridge-v1', $3::text::timestamptz)
             ON CONFLICT (id) DO NOTHING",
            &[&identity_id, &user.id, &now],
        )
        .await
        .map_err(store_error)?;
    let membership_id = project_room_membership_id_for(project_id, &identity_id);
    client
        .execute(
            "INSERT INTO project_room_memberships (id, project_id, chat_identity_id, role, created_at)
             VALUES ($1, $2, $3, $4, $5::text::timestamptz)
             ON CONFLICT (id) DO NOTHING",
            &[
                &membership_id,
                &project_id,
                &identity_id,
                &ProjectMembershipRole::Owner.as_str(),
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

pub(super) async fn upsert_project_row<C>(client: &C, project: &Project) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let placement_runner_class = project
        .placement
        .map(|placement| placement.runner_class.as_str());
    let runtime_resource_class = project
        .placement
        .map(|placement| placement.runtime_resource_class.as_str());
    client
        .execute(
            "INSERT INTO projects
               (id, customer_org_id, owner_user_id, display_name, agent_email,
                import_candidate_id, hosting_tier, placement_runner_class,
                runtime_resource_class, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                     $10::text::timestamptz, $11::text::timestamptz)
             ON CONFLICT (id) DO UPDATE SET
               display_name = EXCLUDED.display_name,
               agent_email = COALESCE(projects.agent_email, EXCLUDED.agent_email),
               hosting_tier = EXCLUDED.hosting_tier,
               placement_runner_class = EXCLUDED.placement_runner_class,
               runtime_resource_class = EXCLUDED.runtime_resource_class,
               updated_at = EXCLUDED.updated_at",
            &[
                &project.id,
                &project.customer_org_id,
                &project.owner_user_id,
                &project.display_name,
                &project.agent_email,
                &project.import_candidate_id,
                &project.hosting_tier.map(HostingTier::as_str),
                &placement_runner_class,
                &runtime_resource_class,
                &project.created_at,
                &project.updated_at,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

pub(super) async fn activate_project_runtime_link<C>(
    client: &C,
    project_id: &str,
    runtime_id: &str,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    // A verified retirement receipt is terminal: re-registration of a retired
    // Runtime must never flip its retired link back to active. Live runtimes
    // have no snapshot rows, so normal launch and registration are unaffected.
    if client
        .query_opt(
            "SELECT 1
             FROM runtime_retirement_snapshots
             WHERE agent_runtime_id = $1 AND verified_at IS NOT NULL
             LIMIT 1",
            &[&runtime_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::RuntimeRetirementSnapshotConflict);
    }
    client
        .execute(
            "UPDATE project_runtime_links SET active = false WHERE project_id = $1",
            &[&project_id],
        )
        .await
        .map_err(store_error)?;
    let link_id = project_runtime_link_id_for(project_id, runtime_id);
    client
        .execute(
            "INSERT INTO project_runtime_links (id, project_id, agent_runtime_id, active, created_at)
             VALUES ($1, $2, $3, true, $4::text::timestamptz)
             ON CONFLICT (id) DO UPDATE SET active = true",
            &[&link_id, &project_id, &runtime_id, &now],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}
