use super::*;

pub(crate) async fn runtime_artifact_upsert(
    input: UpsertRuntimeArtifactInput,
    mode: ImportMode,
) -> Result<RuntimeArtifact> {
    let store = core_store_for_mode(mode).await?;
    store
        .upsert_runtime_artifact(input)
        .await
        .map_err(Into::into)
}

pub(crate) async fn runtime_archive_unrecoverable_command(
    args: RuntimeArchiveUnrecoverableCliArgs,
) -> Result<finite_saas_core::UnrecoverableRuntimeArchiveReceipt> {
    let store = postgres_store_from_env(ImportMode::Commit).await?;
    store
        .admin_archive_unrecoverable_runtime(AdminArchiveUnrecoverableRuntimeInput {
            admin_verified_email: args.admin_email,
            admin_workos_user_id: args.admin_workos_user_id,
            project_id: args.project_id,
            expected_agent_runtime_id: args.expected_agent_runtime_id,
            expected_source_host_id: args.expected_source_host_id,
            expected_source_machine_id: args.expected_source_machine_id,
            expected_owner_email: args.expected_owner_email,
            operator_observed_compute_absent: args.confirm_compute_absent,
            operator_observed_durable_state_absent: args.confirm_durable_state_absent,
            owner_acknowledged_unrecoverable: args.confirm_owner_acknowledged_unrecoverable,
            now: args.now,
        })
        .await
        .map_err(Into::into)
}

pub(crate) async fn runtime_offboard_retired_exact_command(
    args: RuntimeOffboardRetiredExactCliArgs,
) -> Result<finite_saas_core::RetiredRuntimeOffboardReceipt> {
    let store = postgres_store_from_env(ImportMode::Commit).await?;
    store
        .admin_offboard_retired_runtime(AdminOffboardRetiredRuntimeInput {
            admin_verified_email: args.admin_email,
            admin_workos_user_id: args.admin_workos_user_id,
            project_id: args.project_id,
            expected_agent_runtime_id: args.expected_agent_runtime_id,
            expected_source_host_id: args.expected_source_host_id,
            expected_source_machine_id: args.expected_source_machine_id,
            expected_owner_email: args.expected_owner_email,
            operator_observed_compute_absent: args.confirm_compute_absent,
            now: args.now,
        })
        .await
        .map_err(Into::into)
}

pub(crate) async fn runtime_retire_exact_command(args: RuntimeRetireExactCliArgs) -> Result<()> {
    let project_id = required_cli_value(args.project_id, "--project-id")?;
    let expected_agent_runtime_id = required_cli_value(
        args.expected_agent_runtime_id,
        "--expected-agent-runtime-id",
    )?;
    let expected_source_host_id =
        required_cli_value(args.expected_source_host_id, "--expected-source-host-id")?;
    let expected_source_machine_id = required_cli_value(
        args.expected_source_machine_id,
        "--expected-source-machine-id",
    )?;
    let admin_verified_email = required_cli_value(args.admin_email, "--admin-email")?;
    let admin_workos_user_id =
        required_cli_value(args.admin_workos_user_id, "--admin-workos-user-id")?;
    let store = postgres_store_from_env(ImportMode::Commit).await?;
    let request = store
        .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
            admin_verified_email,
            admin_workos_user_id,
            project_id,
            expected_agent_runtime_id,
            expected_source_host_id,
            expected_source_machine_id,
            now: args.now,
        })
        .await?;

    let deadline = Instant::now() + Duration::from_secs(args.wait_timeout_seconds);
    let mut current = request;
    loop {
        current = store.runtime_control_request(&current.id).await?;
        if current.status.is_terminal() {
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            print_json(&current)?;
            bail!("runtime retirement timed out; the same request remains retryable");
        }
        sleep(Duration::from_secs(2).min(deadline.saturating_duration_since(now))).await;
    }
    print_json(&current)?;
    // A retired runtime confirms into the Stopped terminal; Succeeded is
    // reserved for runtimes that proved ready.
    if current.status != RuntimeControlRequestStatus::Stopped {
        bail!("runtime retirement failed; the same request remains retryable");
    }
    Ok(())
}

pub(crate) async fn runtime_cold_relocate_exact_command(
    args: RuntimeColdRelocateExactCliArgs,
) -> Result<finite_saas_core::AgentCreationRequest> {
    let store = postgres_store_from_env(ImportMode::Commit).await?;
    store
        .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
            admin_verified_email: args.admin_email,
            admin_workos_user_id: args.admin_workos_user_id,
            project_id: args.project_id,
            expected_agent_runtime_id: args.expected_agent_runtime_id,
            expected_source_host_id: args.expected_source_host_id,
            expected_source_machine_id: args.expected_source_machine_id,
            target_source_host_id: args.target_source_host_id,
            expected_agent_npub: args.expected_agent_npub,
            durable_state_manifest_sha256: args.durable_state_manifest_sha256,
            operator_observed_compute_absent: args.source_compute_absent,
            now: args.now,
        })
        .await
        .map_err(Into::into)
}
