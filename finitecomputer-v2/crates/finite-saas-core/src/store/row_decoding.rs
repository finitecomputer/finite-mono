use super::*;

pub(super) fn core_user_from_row(row: &Row) -> CoreResult<CoreUser> {
    let status: String = row.get("link_status");
    Ok(CoreUser {
        id: row.get("id"),
        email: row.get("normalized_email"),
        status: parse_user_link_status(&status)
            .ok_or_else(|| CoreError::Store(format!("invalid user link status {status}")))?,
        workos_user_id: row.get("workos_user_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(super) fn customer_org_from_row(row: &Row) -> CoreResult<CustomerOrganization> {
    let billing_class: String = row.get("billing_class");
    Ok(CustomerOrganization {
        id: row.get("id"),
        owner_user_id: row.get("owner_user_id"),
        name: row.get("name"),
        billing_class: parse_billing_class(&billing_class)
            .ok_or_else(|| CoreError::Store(format!("invalid billing class {billing_class}")))?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(crate) fn optional_hosting_tier_column(
    row: &Row,
    name: &str,
) -> CoreResult<Option<HostingTier>> {
    let value: Option<String> = row.get(name);
    value
        .map(|value| {
            parse_hosting_tier(&value)
                .ok_or_else(|| CoreError::Store(format!("invalid hosting tier {value}")))
        })
        .transpose()
}

pub(super) fn optional_runtime_placement_columns(
    row: &Row,
    runner_name: &str,
    resource_name: &str,
) -> CoreResult<Option<RuntimePlacement>> {
    let runner: Option<String> = row.get(runner_name);
    let resource: Option<String> = row.get(resource_name);
    match (runner, resource) {
        (None, None) => Ok(None),
        (Some(runner), Some(resource)) => Ok(Some(RuntimePlacement {
            runner_class: parse_runner_class(&runner)
                .ok_or_else(|| CoreError::Store(format!("invalid agent runner class {runner}")))?,
            runtime_resource_class: parse_runtime_resource_class(&resource).ok_or_else(|| {
                CoreError::Store(format!("invalid runtime resource class {resource}"))
            })?,
        })),
        _ => Err(CoreError::Store(
            "incomplete persisted runtime placement".to_string(),
        )),
    }
}

pub(super) fn agent_creation_entitlement_from_row(
    row: &Row,
) -> CoreResult<AgentCreationEntitlement> {
    Ok(AgentCreationEntitlement {
        id: row.get("id"),
        customer_org_id: row.get("customer_org_id"),
        hosting_tier: optional_hosting_tier_column(row, "hosting_tier")?,
        allowed_new_agent_runtimes: row.get("allowed_new_agent_runtimes"),
        launch_code: row.get("launch_code"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(super) fn project_from_row(row: &Row) -> CoreResult<Project> {
    Ok(Project {
        id: row.get("id"),
        customer_org_id: row.get("customer_org_id"),
        owner_user_id: row.get("owner_user_id"),
        display_name: row.get("display_name"),
        agent_email: row.get("agent_email"),
        import_candidate_id: row.get("import_candidate_id"),
        hosting_tier: optional_hosting_tier_column(row, "hosting_tier")?,
        placement: optional_runtime_placement_columns(
            row,
            "placement_runner_class",
            "runtime_resource_class",
        )?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(super) fn agent_creation_request_from_row(row: &Row) -> CoreResult<AgentCreationRequest> {
    let status: String = row.get("status");
    let runner_class: String = row.get("runner_class");
    let runtime_spec = optional_json_column(row, "runtime_spec")?;
    let relocation = optional_json_column(row, "relocation_spec")?;
    Ok(AgentCreationRequest {
        id: row.get("id"),
        customer_org_id: row.get("customer_org_id"),
        owner_user_id: row.get("owner_user_id"),
        project_id: row.get("project_id"),
        idempotency_key: row.get("idempotency_key"),
        display_name: row.get("display_name"),
        runner_class: parse_runner_class(&runner_class).ok_or_else(|| {
            CoreError::Store(format!("invalid agent runner class {runner_class}"))
        })?,
        hosting_tier: optional_hosting_tier_column(row, "hosting_tier")?,
        placement: optional_runtime_placement_columns(
            row,
            "placement_runner_class",
            "runtime_resource_class",
        )?,
        desired_runtime_artifact_id: row.get("desired_runtime_artifact_id"),
        runtime_spec: runtime_spec
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_error)?,
        target_source_host_id: row.get("target_source_host_id"),
        relocation: relocation
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_error)?,
        profile_picture_url: row.get("profile_picture_url"),
        owner_chat_account_id: row.get("owner_chat_account_id"),
        status: parse_agent_creation_request_status(&status).ok_or_else(|| {
            CoreError::Store(format!("invalid agent creation request status {status}"))
        })?,
        requested_launch_code: row.get("requested_launch_code"),
        agent_runtime_id: row.get("agent_runtime_id"),
        runner_id: row.get("runner_id"),
        lease_token: row.get("lease_token"),
        lease_expires_at: row.get("lease_expires_at"),
        failure_message: row.get("failure_message"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(super) fn runtime_artifact_from_row(row: &Row) -> CoreResult<RuntimeArtifact> {
    let kind: String = row.get("kind");
    Ok(RuntimeArtifact {
        id: row.get("id"),
        kind: parse_runtime_artifact_kind(&kind)
            .ok_or_else(|| CoreError::Store(format!("invalid runtime artifact kind {kind}")))?,
        reference: row.get("reference"),
        version_label: row.get("version_label"),
        source_git_sha: row.get("source_git_sha"),
        finitec_version: row.get("finitec_version"),
        hermes_source_ref: row.get("hermes_source_ref"),
        finite_platform_plugin_ref: row.get("finite_platform_plugin_ref"),
        state_schema_version: row.get("state_schema_version"),
        base_image: row.get("base_image"),
        recover_known_good_chat: row.get("recover_known_good_chat"),
        created_at: row.get("created_at"),
        promoted_at: row.get("promoted_at"),
        retired_at: row.get("retired_at"),
    })
}

pub(super) fn agent_runtime_from_row(row: &Row) -> CoreResult<AgentRuntime> {
    let provider_runtime_handle = optional_json_column(row, "provider_runtime_handle")?;
    let provider_runtime_handle_history =
        optional_json_column(row, "provider_runtime_handle_history")?;
    let runtime_capabilities = optional_json_column(row, "runtime_capabilities")?;
    Ok(AgentRuntime {
        id: row.get("id"),
        project_id: row.get("project_id"),
        source_host_id: row.get("source_host_id"),
        source_machine_id: row.get("source_machine_id"),
        source_import_key: row.get("source_import_key"),
        runtime_artifact_id: row.get("runtime_artifact_id"),
        state_schema_version: row.get("state_schema_version"),
        placement: optional_runtime_placement_columns(
            row,
            "placement_runner_class",
            "runtime_resource_class",
        )?,
        provider_runtime_handle: provider_runtime_handle
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_error)?,
        provider_runtime_handle_history: provider_runtime_handle_history
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_error)?
            .unwrap_or_default(),
        contact_endpoint: row.get("contact_endpoint"),
        runtime_capabilities: runtime_capabilities
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_error)?,
        host_facts: json_column(row, "host_facts")?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(super) fn finite_private_grant_from_row(row: &Row) -> CoreResult<FinitePrivateGrant> {
    let status: String = row.get("status");
    Ok(FinitePrivateGrant {
        id: row.get("id"),
        user_id: row.get("user_id"),
        limit_profile_id: row.get("limit_profile_id"),
        status: parse_finite_private_grant_status(&status).ok_or_else(|| {
            CoreError::Store(format!("invalid finite private grant status {status}"))
        })?,
        current_window_started_at: row.get("current_window_started_at"),
        current_window_used_units: row.get("current_window_used_units"),
        burst_window_epoch: row.get("burst_window_epoch"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(super) fn finite_private_api_key_from_row(row: &Row) -> CoreResult<FinitePrivateApiKey> {
    let status: String = row.get("status");
    Ok(FinitePrivateApiKey {
        id: row.get("id"),
        grant_id: row.get("grant_id"),
        project_id: row.get("project_id"),
        agent_runtime_id: row.get("agent_runtime_id"),
        key_hash: row.get("key_hash"),
        status: parse_finite_private_api_key_status(&status).ok_or_else(|| {
            CoreError::Store(format!("invalid finite private API key status {status}"))
        })?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}
