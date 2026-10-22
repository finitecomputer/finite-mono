//! Host-local, preview-first scoped roster attribution. Never mounted as an API.
use super::*;
use finite_saas_core::brain_identity::scoped::{MAX_MANIFEST_BYTES, ScopedManifest};
use std::io::Read;

pub(crate) async fn brain_scoped_records_command(
    path: std::path::PathBuf,
    expected_digest: Option<String>,
    execute: bool,
) -> Result<()> {
    require_root()?;
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
    let store = postgres_store_from_env(ImportMode::from_dry_run(!execute)).await?;
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
