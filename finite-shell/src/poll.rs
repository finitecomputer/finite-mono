//! Autonomous channel convergence (plan M4): the shell polls the signed
//! service directory for its channel's `payload_bundle` head and converges on
//! it unattended — staging through the exact stage path the socket verb uses
//! and flipping through the exact flip engine, serialized by the same
//! transition gate so the poller can never fight a manual `ctl` operation.
//!
//! Every tick's outcome is recorded atomically in `state.json` as
//! `last_poll` and surfaced in `/healthz`. A head that failed a flip's health
//! gate is bad-listed by the flip machinery and skipped here forever after
//! (until an operator force-stages it).

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use finite_service_directory::{FetchLimits, PAYLOAD_BUNDLE_KIND};

use crate::ShellError;
use crate::control::ShellRuntime;
use crate::generations::{self, StageRequest};
use crate::state::{PollAction, PollRecord, now_rfc3339};

impl ShellRuntime {
    /// Spawn the channel poll loop. Returns whether it was spawned: a zero
    /// `poll_interval` or a missing `FINITE_SERVICE_DIRECTORY_URL` disables
    /// autonomous convergence (manual `ctl` operation remains available).
    pub fn spawn_channel_poller(&self) -> bool {
        if self.settings.poll_interval.is_zero() {
            return false;
        }
        if self.settings.service_directory_url.is_none() {
            eprintln!(
                "finite-shell: no service directory URL is configured; the channel poller is off"
            );
            return false;
        }
        let runtime = self.clone();
        tokio::spawn(async move {
            loop {
                let record = runtime.poll_channel_once().await;
                if record.action == PollAction::Error {
                    eprintln!(
                        "finite-shell: channel poll failed: {}",
                        record.detail.as_deref().unwrap_or("unknown error")
                    );
                }
                tokio::time::sleep(jittered(runtime.settings.poll_interval)).await;
            }
        });
        true
    }

    /// One poll tick: fetch + verify the directory, resolve this shell's
    /// channel head, and stage/flip toward it if appropriate. Always records
    /// the outcome as `last_poll` (atomic state write).
    pub async fn poll_channel_once(&self) -> PollRecord {
        let channel = self.layout.channel_name();
        let (head_version, action, detail) = self.poll_tick(&channel).await;
        let record = PollRecord {
            at: now_rfc3339(),
            channel,
            head_version,
            action,
            detail,
        };
        let _ = self
            .state
            .update(|state| state.last_poll = Some(record.clone()));
        record
    }

    async fn poll_tick(&self, channel: &str) -> (Option<String>, PollAction, Option<String>) {
        let Some(url) = self.settings.service_directory_url.as_deref() else {
            return (
                None,
                PollAction::Error,
                Some("no service directory URL is configured".to_owned()),
            );
        };
        let Some(public_key_hex) = self.settings.release_public_key.as_deref() else {
            return (
                None,
                PollAction::Error,
                Some(ShellError::MissingReleaseKey.to_string()),
            );
        };
        let public_key = match finite_release::parse_verifying_key_hex(public_key_hex) {
            Ok(key) => key,
            Err(error) => {
                return (
                    None,
                    PollAction::Error,
                    Some(format!("release public key is invalid: {error}")),
                );
            }
        };
        let limits = FetchLimits {
            allow_insecure_url: self.settings.allow_insecure_bundle_url,
            ..FetchLimits::default()
        };
        let directory =
            match finite_service_directory::fetch_and_verify(url, &public_key, &limits).await {
                Ok(directory) => directory,
                Err(error) => {
                    return (
                        None,
                        PollAction::Error,
                        Some(format!("directory fetch failed: {error}")),
                    );
                }
            };
        let Some(head) = directory.channel_bundle(channel, PAYLOAD_BUNDLE_KIND) else {
            return (
                None,
                PollAction::None,
                Some(format!("channel {channel:?} has no payload head")),
            );
        };
        let head_version = head.version_label.clone();

        let current = generations::read_link_version(&self.layout.current_link());
        if current.as_deref() == Some(head_version.as_str()) {
            return (Some(head_version), PollAction::None, None);
        }
        let snapshot = self.state.snapshot();
        if snapshot.is_bad(&head_version) {
            return (
                Some(head_version),
                PollAction::SkippedBad,
                Some("the head failed a previous flip's health gate".to_owned()),
            );
        }

        // Serialize with the socket verbs through the shared transition
        // gate: if a stage/flip/rollback is mid-operation (manual or a
        // previous tick's), skip this tick rather than queue behind it.
        let Ok(gate) = Arc::clone(&self.transition_gate).try_lock_owned() else {
            return (
                Some(head_version),
                PollAction::None,
                Some("a transition is in progress; skipped this tick".to_owned()),
            );
        };

        let already_staged = snapshot.staged.as_ref().is_some_and(|staged| {
            staged.version_label == head_version
                && self.layout.generation_dir(&head_version).is_dir()
        });
        let mut staged_this_tick = false;
        if !already_staged {
            let request = StageRequest {
                tarball_url: Some(head.tarball_url.clone()),
                manifest_url: Some(head.manifest_url.clone()),
                tarball_sha256: Some(head.tarball_sha256.clone()),
                tarball_path: None,
                manifest_path: None,
                force: false,
            };
            match generations::stage_payload(&self.settings, &self.layout, &self.state, &request)
                .await
            {
                Ok(_) => staged_this_tick = true,
                Err(error @ ShellError::MinShellVersion { .. }) => {
                    return (
                        Some(head_version),
                        PollAction::SkippedMinShell,
                        Some(error.to_string()),
                    );
                }
                Err(error @ ShellError::VersionBad(_)) => {
                    return (
                        Some(head_version),
                        PollAction::SkippedBad,
                        Some(error.to_string()),
                    );
                }
                Err(ShellError::VersionIsCurrent(_)) => {
                    // A manual flip to the head raced this tick; converged.
                    return (Some(head_version), PollAction::None, None);
                }
                Err(error) => {
                    return (
                        Some(head_version),
                        PollAction::Error,
                        Some(format!("stage failed: {error}")),
                    );
                }
            }
        }

        match self.start_flip_locked(gate) {
            Ok((from, to)) => (
                Some(head_version),
                PollAction::FlipStarted,
                Some(format!(
                    "flip started: {} -> {to}{}",
                    from.as_deref().unwrap_or("<none>"),
                    if staged_this_tick {
                        ""
                    } else {
                        " (already staged)"
                    }
                )),
            ),
            Err(error) => (
                Some(head_version),
                if staged_this_tick {
                    PollAction::Staged
                } else {
                    PollAction::Error
                },
                Some(format!("flip did not start: {error}")),
            ),
        }
    }
}

/// `interval` jittered by ±20%, clock-seeded the same way as agentd's
/// directory refresher, so a fleet's polls de-align without a shared RNG.
fn jittered(interval: Duration) -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    let factor = 0.8 + 0.4 * (f64::from(nanos) / 1_000_000_000.0);
    interval.mul_f64(factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_poll_interval_stays_within_twenty_percent_jitter() {
        let interval = Duration::from_secs(900);
        for _ in 0..32 {
            let jittered = jittered(interval);
            assert!(jittered >= interval.mul_f64(0.8));
            assert!(jittered <= interval.mul_f64(1.2));
        }
    }
}
