//! Service directory refresh: fetch, verify, and cache Core's signed
//! `finite_service_directory.v1` document.
//!
//! The daemon refreshes `<agent home>/service-directory.json` on start and
//! every 15 minutes (jittered ±20%). Refresh failures are logged and keep the
//! previous cache — a stale verified directory beats no directory. Readers of
//! the cache (e.g. the Hermes plugin's Python accessor) get base URLs only;
//! anything that acts on channel heads re-fetches and re-verifies.

use std::path::PathBuf;
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
    let public_key = finite_release::parse_verifying_key_hex(public_key_hex).map_err(|error| {
        AgentdError::Config(format!("{RELEASE_PUBLIC_KEY_ENV} is invalid: {error}"))
    })?;
    let limits = FetchLimits {
        allow_insecure_url,
        ..FetchLimits::default()
    };
    finite_service_directory::fetch_and_verify(url, &public_key, &limits)
        .await
        .map_err(|error| AgentdError::ServiceDirectory(error.to_string()))
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
            match fetch_verified_directory(&url, &public_key_hex, allow_insecure_url).await {
                Ok(directory) => {
                    if let Err(error) =
                        finite_service_directory::write_cache(&directory, &cache_path)
                    {
                        eprintln!(
                            "finite-agentd: service directory cache write failed; the previous cache remains: {error}"
                        );
                    }
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
