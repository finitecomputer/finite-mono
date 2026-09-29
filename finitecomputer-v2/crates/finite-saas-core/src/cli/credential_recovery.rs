use super::*;
use finite_saas_core::store::runtime_credentials::RecoverRelocatedCredential;

#[derive(Debug, clap::Args)]
pub(crate) struct RuntimeCredentialRecoveryArgs {
    #[arg(long)]
    admin_email: String,
    #[arg(long)]
    admin_workos_user_id: String,
    #[arg(long)]
    project_id: String,
    #[arg(long)]
    expected_agent_runtime_id: String,
    #[arg(long)]
    expected_source_host_id: String,
    #[arg(long)]
    expected_source_machine_id: String,
    #[arg(long)]
    expected_owner_email: String,
    #[arg(long)]
    expected_agent_npub: String,
    #[arg(long)]
    expected_predecessor_creation_request_id: String,
    #[arg(long)]
    expected_relocation_request_id: String,
    #[arg(long)]
    target_runtime_artifact_id: String,
    /// Attest that retained evidence attributes revocation to the old relocation bug.
    #[arg(long)]
    confirm_relocation_credential_loss: bool,
    /// Run the exact transaction, then roll it back. Prints no credentials.
    #[arg(long)]
    dry_run: bool,
}

pub(crate) async fn runtime_credential_recovery_command(
    args: RuntimeCredentialRecoveryArgs,
) -> Result<RuntimeControlRequest> {
    let store = postgres_store_from_env(ImportMode::from_dry_run(args.dry_run)).await?;
    store
        .recover_relocated_credential(RecoverRelocatedCredential {
            upgrade: AdminRuntimeUpgradeExactInput {
                admin_verified_email: args.admin_email,
                admin_workos_user_id: args.admin_workos_user_id,
                project_id: args.project_id,
                expected_agent_runtime_id: args.expected_agent_runtime_id,
                expected_source_host_id: args.expected_source_host_id,
                expected_source_machine_id: args.expected_source_machine_id,
                target_runtime_artifact_id: args.target_runtime_artifact_id,
                now: None,
            },
            expected_owner_email: args.expected_owner_email,
            expected_agent_npub: args.expected_agent_npub,
            expected_predecessor_creation_request_id: args.expected_predecessor_creation_request_id,
            expected_relocation_request_id: args.expected_relocation_request_id,
            confirm_relocation_credential_loss: args.confirm_relocation_credential_loss,
        })
        .await
        .map_err(Into::into)
}
