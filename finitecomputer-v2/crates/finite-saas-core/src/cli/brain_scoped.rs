//! Host-local, preview-first scoped roster attribution. Never mounted as an API.
use super::*;
use finite_saas_core::brain_identity::scoped::{MAX_MANIFEST_BYTES, ScopedManifest};
use std::io::Read;
#[cfg(test)]
mod tests;

pub(crate) async fn brain_scoped_records_command(
    path: std::path::PathBuf,
    expected_digest: Option<String>,
    execute: bool,
) -> Result<()> {
    require_root()?;
    run_scoped_records_command(path, expected_digest, execute).await
}

async fn run_scoped_records_command(
    path: std::path::PathBuf,
    expected_digest: Option<String>,
    execute: bool,
) -> Result<()> {
    // Match service configuration parsing, without normalizing the manifest.
    let server = required_env("FC_CORE_BRAIN_IDENTITY_BRAIN_SERVER")?
        .trim()
        .to_owned();
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_MANIFEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        bail!("scoped manifest exceeds byte limit");
    }
    let manifest: ScopedManifest = serde_json::from_slice(&bytes)?;
    manifest.validate(&server).map_err(anyhow::Error::msg)?;
    let digest = manifest.digest();
    if expected_digest
        .as_ref()
        .is_some_and(|expected| expected != &digest)
    {
        bail!("scoped manifest digest differs from reviewed preview");
    }
    if execute && expected_digest.is_none() {
        bail!("execute requires the reviewed manifest digest");
    }
    // This is an operation on an already deployed schema. The service owns
    // migration: even repeat-safe startup DDL takes broad table locks and can
    // abort live Core work. Apply, replay and retirement must never run it.
    let database_url = required_env("FC_CORE_DATABASE_URL")?;
    let timeout = optional_duration_secs("FC_CORE_POSTGRES_CONNECT_TIMEOUT_SECS", 60)?;
    let store = tokio::time::timeout(timeout, async {
        if execute {
            CoreStore::connect(&database_url).await
        } else {
            CoreStore::connect_dry_run(&database_url).await
        }
    })
    .await
    .context("Core Postgres connection timed out")??;
    let receipt = store
        .execute_brain_scoped_manifest(&manifest, &server, time::OffsetDateTime::now_utc())
        .await?;
    print_json(
        &serde_json::json!({ "dryRun": !execute, "manifestSha256": digest, "receipt": receipt }),
    )
}

fn require_root() -> Result<()> {
    // Read the effective OS uid directly. NixOS has no /usr/bin/id, and
    // neither PATH nor an environment-provided uid is an authority.
    // The linked operator is independently checked in the transaction.
    if !rustix::process::geteuid().is_root() {
        bail!("brain-scoped-records requires OS root");
    }
    Ok(())
}
