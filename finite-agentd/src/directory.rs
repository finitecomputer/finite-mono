//! Service directory refresh: fetch, verify, and cache Core's signed
//! `finite_service_directory.v1` document.
//!
//! The daemon refreshes `<agent home>/service-directory.json` on start and
//! every 15 minutes (jittered ±20%). Refresh failures are logged and keep the
//! previous cache — a stale verified directory beats no directory. Readers of
//! the cache (e.g. the Hermes plugin's Python accessor) get base URLs only;
//! anything that acts on channel heads re-fetches and re-verifies.
//!
//! Anti-replay: the verified cache doubles as the daemon's durable
//! `generated_at` floor. A fetched document strictly older than the cached
//! one is refused (an old signed capture must never downgrade channel heads),
//! and every accepted document advances the floor by replacing the cache.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use finite_service_directory::{FetchLimits, SKILLS_BUNDLE_KIND, ServiceDirectoryV1};

use crate::AgentdError;
use crate::daemon::DaemonConfig;
use crate::skills::{RELEASE_PUBLIC_KEY_ENV, SkillsSyncRequest, run_skills_sync_cli};

pub(crate) const SERVICE_DIRECTORY_URL_ENV: &str = "FINITE_SERVICE_DIRECTORY_URL";
pub(crate) const SERVICE_DIRECTORY_CACHE_FILE: &str = "service-directory.json";

/// Toggle for skills auto-convergence (on by default). Disabled by a clearly
/// falsy value, matching the brain-sync supervisor toggle.
pub(crate) const SKILLS_AUTOCONVERGE_ENV: &str = "FINITE_SKILLS_AUTOCONVERGE";
/// Durable marker of the last skills head applied by auto-convergence, so a
/// tick whose head is unchanged does no work. Lives in the agentd state dir.
const APPLIED_SKILLS_HEAD_FILE: &str = "applied-skills-head";
/// Where auto-convergence reads its channel when the shell has not written one.
const DEFAULT_CHANNEL: &str = "stable";

const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Fetch the directory from `url` and verify it against the release public
/// key. This is always a fresh fetch — never a cache read.
pub(crate) async fn fetch_verified_directory(
    url: &str,
    public_key_hex: &str,
    allow_insecure_url: bool,
) -> Result<ServiceDirectoryV1, AgentdError> {
    let public_key = parse_public_key(public_key_hex)?;
    let limits = FetchLimits {
        allow_insecure_url,
        ..FetchLimits::default()
    };
    finite_service_directory::fetch_and_verify(url, &public_key, &limits)
        .await
        .map_err(|error| AgentdError::ServiceDirectory(error.to_string()))
}

fn parse_public_key(public_key_hex: &str) -> Result<ed25519_dalek::VerifyingKey, AgentdError> {
    finite_release::parse_verifying_key_hex(public_key_hex).map_err(|error| {
        AgentdError::Config(format!("{RELEASE_PUBLIC_KEY_ENV} is invalid: {error}"))
    })
}

/// [`fetch_verified_directory`] plus the persisted monotonic floor: when a
/// cache path is given, a fetched document older than the cached (verified)
/// one is refused with a distinct replay error, and an accepted document
/// replaces the cache so the floor survives restarts. An equal `generated_at`
/// is accepted — Core idempotently re-serving the same document is not a
/// replay.
pub(crate) async fn fetch_verified_directory_with_floor(
    url: &str,
    public_key_hex: &str,
    allow_insecure_url: bool,
    cache_path: Option<&Path>,
) -> Result<ServiceDirectoryV1, AgentdError> {
    let directory = fetch_verified_directory(url, public_key_hex, allow_insecure_url).await?;
    let Some(cache_path) = cache_path else {
        return Ok(directory);
    };
    let public_key = parse_public_key(public_key_hex)?;
    // An unreadable/stale/tampered cache yields no floor rather than wedging
    // convergence; the max-age check bounds what a wiped floor can admit.
    if let Ok(cached) = finite_service_directory::read_cache(cache_path, &public_key) {
        directory
            .check_not_older_than(&cached.generated_at)
            .map_err(|error| AgentdError::ServiceDirectoryReplayed(error.to_string()))?;
    }
    if let Err(error) = finite_service_directory::write_cache(&directory, cache_path) {
        eprintln!(
            "finite-agentd: service directory cache write failed; the previous cache remains: {error}"
        );
    }
    Ok(directory)
}

/// Start the daemon's periodic directory refresh when
/// `FINITE_SERVICE_DIRECTORY_URL` is configured. The first refresh runs
/// immediately on daemon start.
pub(crate) fn spawn_directory_refresher(config: &DaemonConfig) {
    let Some(url) = config.service_directory_url.clone() else {
        return;
    };
    let Some(public_key_hex) = config.release_public_key.clone() else {
        // Non-fatal, same reporting channel as other daemon issues: without
        // the key nothing could verify the fetched document.
        eprintln!(
            "finite-agentd: {SERVICE_DIRECTORY_URL_ENV} is set but {RELEASE_PUBLIC_KEY_ENV} is not; the service directory will not be fetched"
        );
        return;
    };
    let cache_path: PathBuf = config.agent_home.join(SERVICE_DIRECTORY_CACHE_FILE);
    let allow_insecure_url = config.allow_insecure_bundle_url;
    // The converge step reuses the full skills-sync path (verify + apply +
    // ledger), so it needs the whole config.
    let config = config.clone();
    tokio::spawn(async move {
        loop {
            // The floor-checked fetch also advances the cache on acceptance.
            match fetch_verified_directory_with_floor(
                &url,
                &public_key_hex,
                allow_insecure_url,
                Some(&cache_path),
            )
            .await
            {
                Ok(directory) => {
                    // Same tick: auto-converge the channel's skills head, using
                    // the verified skills-sync path (no verification bypass).
                    converge_channel_skills(&config, &directory).await;
                }
                Err(error) => {
                    eprintln!(
                        "finite-agentd: service directory refresh failed; the previous cache remains: {}",
                        error.public_message()
                    );
                }
            }
            tokio::time::sleep(jittered_refresh_interval()).await;
        }
    });
}

/// The channel auto-convergence acts on: whatever the shell recorded beside its
/// control socket (the same file `agent.payload.set-channel` writes), defaulting
/// to `stable`.
fn resolve_channel(config: &DaemonConfig) -> String {
    config
        .shell_socket
        .parent()
        .map(|dir| dir.join("channel"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CHANNEL.to_owned())
}

/// Auto-convergence is on unless explicitly disabled.
fn skills_autoconverge_enabled() -> bool {
    !std::env::var(SKILLS_AUTOCONVERGE_ENV)
        .is_ok_and(|value| matches!(value.trim(), "0" | "false" | "no"))
}

/// Resolve the channel's `skills_bundle` head from the freshly-verified
/// directory and, if it differs from the last-applied head, apply it through
/// the same verified `agent.skills.sync` path (fetch, verify signature+digest,
/// atomic swap) the manual command uses. The outcome is recorded in the agentd
/// ledger. A durable head marker keeps an unchanged head from re-applying every
/// tick; a failed attempt leaves the marker untouched so the next tick retries.
pub(crate) async fn converge_channel_skills(config: &DaemonConfig, directory: &ServiceDirectoryV1) {
    if !skills_autoconverge_enabled() {
        return;
    }
    let channel = resolve_channel(config);
    let Some(bundle) = directory.channel_bundle(&channel, SKILLS_BUNDLE_KIND) else {
        // Not every channel advertises a skills head; nothing to converge.
        return;
    };
    // The head identity: artifact + version + content hash. Content hash alone
    // would suffice, but the version/artifact make the marker legible.
    let head = format!(
        "{}\t{}\t{}",
        bundle.artifact_id, bundle.version_label, bundle.tarball_sha256
    );
    let marker_path = config.state_dir().join(APPLIED_SKILLS_HEAD_FILE);
    if std::fs::read_to_string(&marker_path).ok().as_deref() == Some(head.as_str()) {
        // Already applied this exact head; do no work.
        return;
    }
    // A fresh request id per attempt so a transient failure does not become a
    // permanently-replayed ledger verdict — the marker, not the ledger, is the
    // dedup for a successfully-applied head.
    let request_id = format!(
        "auto-skills-sync-{channel}-{}-{}",
        bundle.tarball_sha256,
        unix_millis_now()
    );
    let request = SkillsSyncRequest {
        tarball_url: None,
        manifest_url: None,
        tarball_sha256: None,
        channel: Some(channel.clone()),
    };
    match run_skills_sync_cli(config, &request_id, request).await {
        Ok(_) => {
            if let Err(error) = write_marker(&marker_path, &head) {
                eprintln!(
                    "finite-agentd: skills auto-converge applied {channel} head {} but the marker was not persisted ({error}); it may re-apply next tick",
                    bundle.version_label
                );
            }
            eprintln!(
                "finite-agentd: auto-converged skills for channel {channel} to {} ({})",
                bundle.version_label, bundle.artifact_id
            );
        }
        Err(error) => {
            eprintln!(
                "finite-agentd: skills auto-converge for channel {channel} failed; will retry: {}",
                error.public_message()
            );
        }
    }
}

/// Persist the applied-head marker atomically (tmp + rename).
fn write_marker(marker_path: &Path, head: &str) -> std::io::Result<()> {
    if let Some(parent) = marker_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = marker_path.with_extension("tmp");
    std::fs::write(&temporary, head)?;
    std::fs::rename(&temporary, marker_path)
}

fn unix_millis_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// The 15-minute refresh interval jittered by ±20% so a fleet's refreshes
/// de-align without needing a shared RNG.
fn jittered_refresh_interval() -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    let factor = 0.8 + 0.4 * (f64::from(nanos) / 1_000_000_000.0);
    REFRESH_INTERVAL.mul_f64(factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refresh_interval_stays_within_twenty_percent_jitter() {
        for _ in 0..32 {
            let interval = jittered_refresh_interval();
            assert!(interval >= REFRESH_INTERVAL.mul_f64(0.8));
            assert!(interval <= REFRESH_INTERVAL.mul_f64(1.2));
        }
    }

    #[test]
    fn skills_auto_converge_is_on_by_default() {
        // The gate defaults on when the toggle is unset (the runtime image sets
        // no override). Env mutation is avoided to keep parallel tests sound.
        assert!(std::env::var(SKILLS_AUTOCONVERGE_ENV).is_err());
        assert!(skills_autoconverge_enabled());
    }
}
