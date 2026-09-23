use super::*;

impl CoreStore {
    pub async fn admin_request_runtime_upgrade(
        &self,
        input: AdminRuntimeUpgradeInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_upgrade(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_request_runtime_upgrade_exact(
        &self,
        input: AdminRuntimeUpgradeExactInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_upgrade_exact(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_request_runtime_retire_exact(
        &self,
        input: AdminRuntimeRetireExactInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_retire_exact(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_request_runtime_relocate_exact(
        &self,
        input: AdminRuntimeRelocateExactInput,
    ) -> CoreResult<AgentCreationRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_relocate_exact(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_request_runtime_restart(
        &self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_admin_request_runtime_control(&*tx, input, RuntimeControlKind::Restart, None)
                .await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_request_runtime_recover_known_good_chat(
        &self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_control(
            &*tx,
            input,
            RuntimeControlKind::RecoverKnownGoodChatRuntime,
            None,
        )
        .await?;
        self.finish(tx).await?;
        Ok(result)
    }
}

pub(super) async fn postgres_admin_request_runtime_control_bound<C>(
    client: &C,
    input: AdminRuntimeControlInput,
    kind: RuntimeControlKind,
    target_runtime_artifact_id: Option<String>,
    expected: Option<&RuntimeControlExpectedBinding>,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let admin_workos_user_id = input.admin_workos_user_id.trim().to_string();
    if admin_workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let admin_user =
        ensure_grandfathered_linked_user(client, &admin_email, &admin_workos_user_id, &now).await?;
    let project = select_project(client, &input.project_id)
        .await?
        .ok_or(CoreError::ProjectNotFound)?;
    let request = postgres_enqueue_runtime_control_request_bound(
        client,
        &project,
        Some(&admin_user.id),
        kind,
        target_runtime_artifact_id,
        &now,
        expected,
    )
    .await?;
    let action = match kind {
        RuntimeControlKind::Restart => "runtime.admin_restart",
        RuntimeControlKind::RecoverKnownGoodChatRuntime => "runtime.admin_recover_known_good_chat",
        RuntimeControlKind::Upgrade => "runtime.admin_upgrade",
        RuntimeControlKind::Stop => "runtime.admin_stop",
        RuntimeControlKind::Destroy => "runtime.admin_destroy",
    };
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action,
            target_type: "agent_runtime",
            target_id: &request.agent_runtime_id,
            grant_id: None,
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({
                "projectId": request.project_id.clone(),
                "runtimeControlRequestId": request.id.clone(),
                "kind": kind.as_str(),
                "targetRuntimeArtifactId": request.target_runtime_artifact_id.clone(),
            }),
            now: &now,
        },
    )
    .await?;
    Ok(request)
}

async fn postgres_admin_request_runtime_upgrade<C>(
    client: &C,
    input: AdminRuntimeUpgradeInput,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    postgres_admin_request_runtime_control(
        client,
        AdminRuntimeControlInput {
            admin_verified_email: input.admin_verified_email,
            admin_workos_user_id: input.admin_workos_user_id,
            project_id: input.project_id,
            now: input.now,
        },
        RuntimeControlKind::Upgrade,
        Some(input.target_runtime_artifact_id),
    )
    .await
}

async fn postgres_admin_request_runtime_upgrade_exact<C>(
    client: &C,
    input: AdminRuntimeUpgradeExactInput,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let expected = RuntimeControlExpectedBinding {
        agent_runtime_id: input.expected_agent_runtime_id,
        source_host_id: input.expected_source_host_id,
        source_machine_id: input.expected_source_machine_id,
    };
    postgres_admin_request_runtime_control_bound(
        client,
        AdminRuntimeControlInput {
            admin_verified_email: input.admin_verified_email,
            admin_workos_user_id: input.admin_workos_user_id,
            project_id: input.project_id,
            now: input.now,
        },
        RuntimeControlKind::Upgrade,
        Some(input.target_runtime_artifact_id),
        Some(&expected),
    )
    .await
}

async fn postgres_admin_request_runtime_retire_exact<C>(
    client: &C,
    input: AdminRuntimeRetireExactInput,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let expected = RuntimeControlExpectedBinding {
        agent_runtime_id: input.expected_agent_runtime_id,
        source_host_id: input.expected_source_host_id,
        source_machine_id: input.expected_source_machine_id,
    };
    postgres_admin_request_runtime_control_bound(
        client,
        AdminRuntimeControlInput {
            admin_verified_email: input.admin_verified_email,
            admin_workos_user_id: input.admin_workos_user_id,
            project_id: input.project_id,
            now: input.now,
        },
        RuntimeControlKind::Destroy,
        None,
        Some(&expected),
    )
    .await
}
