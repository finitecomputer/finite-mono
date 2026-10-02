use super::*;
use finite_saas_core::{CancelRelocationExactInput, RelocationCancelOutcome};

#[derive(Debug, clap::Args)]
pub(crate) struct RuntimeRelocationCancelExactArgs {
    #[arg(long)]
    pub(crate) relocation_request_id: String,
    #[arg(long)]
    pub(crate) expected_agent_runtime_id: String,
    #[arg(long)]
    pub(crate) expected_target_source_host_id: String,
    /// Attest that no late Runner cycle can act on the target and that target
    /// shutdown was proved on the host, or the host is fenced. Core cannot
    /// check this. Needed only to release a cancelled relocation whose lease
    /// has expired without its Runner releasing it.
    #[arg(long)]
    pub(crate) confirm_target_compute_stopped: bool,
    /// Run the exact transaction, then roll it back.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

/// Prints a redacted projection: never the lease token.
pub(crate) async fn runtime_relocation_cancel_command(
    args: RuntimeRelocationCancelExactArgs,
) -> Result<RelocationCancelOutcome> {
    let store = postgres_store_from_env(ImportMode::from_dry_run(args.dry_run)).await?;
    store
        .cancel_relocation_exact(CancelRelocationExactInput {
            relocation_request_id: args.relocation_request_id,
            expected_agent_runtime_id: args.expected_agent_runtime_id,
            expected_target_source_host_id: args.expected_target_source_host_id,
            confirm_target_compute_stopped: args.confirm_target_compute_stopped,
            now: None,
        })
        .await
        .map_err(Into::into)
}
