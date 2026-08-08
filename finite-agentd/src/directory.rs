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

use finite_service_directory::{FetchLimits, ServiceDirectoryV1};

use crate::AgentdError;
use crate::daemon::DaemonConfig;
use crate::skills::RELEASE_PUBLIC_KEY_ENV;

pub(crate) const SERVICE_DIRECTORY_URL_ENV: &str = "FINITE_SERVICE_DIRECTORY_URL";
pub(crate) const SERVICE_DIRECTORY_CACHE_FILE: &str = "service-directory.json";

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
    tokio::spawn(async move {
        loop {
            // The floor-checked fetch also advances the cache on acceptance.
            if let Err(error) = fetch_verified_directory_with_floor(
                &url,
                &public_key_hex,
                allow_insecure_url,
                Some(&cache_path),
            )
            .await
            {
                eprintln!(
                    "finite-agentd: service directory refresh failed; the previous cache remains: {}",
                    error.public_message()
                );
            }
            tokio::time::sleep(jittered_refresh_interval()).await;
        }
    });
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
}
