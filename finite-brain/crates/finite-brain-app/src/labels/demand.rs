//! One process coalesces demand across every Brain. No timer performs discovery.
use super::*;
use finite_brain_server::principal_labels::LABEL_REFRESH_INTERVAL_SECONDS;

const DISCOVERY_INTERVAL: Duration = Duration::from_secs(15 * 60);
const REFRESH_INTERVAL: Duration = Duration::from_secs(LABEL_REFRESH_INTERVAL_SECONDS);

/// Only locations matched by a complete discovery are retained. Unknown keys
/// share the discovery cooldown: adding arbitrary npubs cannot force new scans.
#[derive(Default)]
pub(super) struct HostedLocations {
    discovered_at: Option<Instant>,
    discovery_failed: bool,
    candidates: BTreeMap<String, String>,
}

impl HostedLocations {
    pub(super) fn candidates(
        &mut self,
        root: &Path,
        principals: &BTreeSet<String>,
        now: Instant,
    ) -> Result<BTreeMap<String, String>, RefreshError> {
        if principals.is_empty() {
            return Ok(BTreeMap::new());
        }
        if self
            .discovered_at
            .is_none_or(|last| now.duration_since(last) >= DISCOVERY_INTERVAL)
        {
            // Failed discovery shares the same cooldown. Keep the old output
            // to expire; never republish it as newly verified after a scan error.
            self.discovered_at = Some(now);
            self.discovery_failed = true;
            self.candidates = hosted_candidates(root, principals, Instant::now())?;
            self.discovery_failed = false;
            return Ok(self.candidates.clone());
        }
        if self.discovery_failed {
            return Err(RefreshError::SourceUnavailable);
        }

        let mut current = BTreeMap::new();
        let started = Instant::now();
        for (storage_id, prior_npub) in &self.candidates {
            if started.elapsed() > Duration::from_secs(25) {
                return Err(RefreshError::Limit);
            }
            if principals.contains(prior_npub)
                && hosted_public_key(root, storage_id)?.as_ref() == Some(prior_npub)
            {
                current.insert(storage_id.clone(), prior_npub.clone());
            }
        }
        Ok(current)
    }
}

#[derive(Default)]
struct RefreshGate {
    next_attempt: Option<Instant>,
    failures: u32,
}

impl RefreshGate {
    fn ready(&self, now: Instant) -> bool {
        self.next_attempt.is_none_or(|next| now >= next)
    }

    fn finished(&mut self, now: Instant, success: bool) {
        let delay = if success {
            self.failures = 0;
            REFRESH_INTERVAL
        } else {
            self.failures = (self.failures + 1).min(5);
            Duration::from_secs((30_u64 << (self.failures - 1)).min(300))
        };
        self.next_attempt = Some(now + delay);
    }
}

#[cfg(unix)]
pub(super) async fn serve(path: &Path) -> Result<(), RefreshError> {
    use std::os::unix::fs::PermissionsExt;
    // The directory is worker-owned (0750); callers have socket write access,
    // not directory write access. A restarted worker may remove its old socket.
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(RefreshError::Configuration),
    }
    let socket = tokio::net::UnixDatagram::bind(path).map_err(|_| RefreshError::Configuration)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))
        .map_err(|_| RefreshError::Configuration)?;
    let mut locations = HostedLocations::default();
    let mut gate = RefreshGate::default();
    let mut bytes = [0_u8; 16];
    loop {
        let len = socket
            .recv(&mut bytes)
            .await
            .map_err(|_| RefreshError::Configuration)?;
        if &bytes[..len] != b"refresh" || !gate.ready(Instant::now()) {
            continue;
        }
        let result = tokio::time::timeout(Duration::from_secs(30), refresh(&mut locations)).await;
        let success = matches!(result, Ok(Ok(())));
        gate.finished(Instant::now(), success);
        if !success {
            // No connection strings or identities in diagnostics.
            eprintln!(
                "Brain label refresh failed: {:?}",
                result.map(|result| result.err())
            );
        }
    }
}

#[cfg(not(unix))]
pub(super) async fn serve(_path: &Path) -> Result<(), RefreshError> {
    Err(RefreshError::Configuration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_demand_is_coalesced_and_failures_back_off() {
        let now = Instant::now();
        let mut gate = RefreshGate::default();
        assert!(gate.ready(now));
        gate.finished(now, true);
        for seconds in 0..LABEL_REFRESH_INTERVAL_SECONDS {
            assert!(!gate.ready(now + Duration::from_secs(seconds)));
        }
        let retry = now + REFRESH_INTERVAL;
        assert!(gate.ready(retry));
        gate.finished(retry, false);
        assert!(!gate.ready(retry + Duration::from_secs(29)));
        assert!(gate.ready(retry + Duration::from_secs(30)));
        gate.finished(retry + Duration::from_secs(30), false);
        assert!(!gate.ready(retry + Duration::from_secs(89)));
        assert!(gate.ready(retry + Duration::from_secs(90)));
        for _ in 0..20 {
            gate.finished(retry, false);
        }
        assert!(gate.ready(retry + REFRESH_INTERVAL));
        gate.finished(retry, true);
        assert_eq!(gate.failures, 0);
    }

    #[test]
    fn cached_locations_revalidate_keys_and_bound_unknown_discovery() {
        let dir = tempfile::tempdir().unwrap();
        let (hex, npub) = tests_fixture_principal(1);
        super::super::tests::hosted_fixture(dir.path(), "known", &hex);
        let (_, unknown) = tests_fixture_principal(2);
        let targets = BTreeSet::from([npub.clone(), unknown.clone()]);
        let now = Instant::now();
        let mut locations = HostedLocations::default();
        assert_eq!(
            locations
                .candidates(dir.path(), &targets, now)
                .unwrap()
                .len(),
            1
        );
        // A corrupt unrelated source would fail a complete scan. Cached refresh
        // touches only known locations until discovery is due.
        let unrelated = super::super::tests::hosted_fixture(
            dir.path(),
            "unrelated",
            &tests_fixture_principal(3).0,
        );
        fs::write(&unrelated, b"broken").unwrap();
        assert_eq!(
            locations
                .candidates(dir.path(), &targets, now + REFRESH_INTERVAL)
                .unwrap()
                .len(),
            1
        );
        assert!(
            locations
                .candidates(dir.path(), &targets, now + DISCOVERY_INTERVAL)
                .is_err()
        );
        fs::remove_file(unrelated).unwrap();
        assert!(
            locations
                .candidates(
                    dir.path(),
                    &targets,
                    now + DISCOVERY_INTERVAL + REFRESH_INTERVAL
                )
                .is_err(),
            "failed discovery cannot be retried by a burst of new requests"
        );
        super::super::tests::hosted_fixture(dir.path(), "new", &tests_fixture_principal(2).0);
        let discovered = locations
            .candidates(dir.path(), &targets, now + DISCOVERY_INTERVAL * 2)
            .unwrap();
        assert_eq!(discovered.len(), 2);
        let known = dir
            .path()
            .join("users")
            .join(super::super::tests::subject_hash("known"))
            .join("chat/client.sqlite3");
        let conn = Connection::open(known).unwrap();
        conn.execute(
            "UPDATE client_device_states SET account_id=?1",
            [tests_fixture_principal(4).0],
        )
        .unwrap();
        let refreshed = locations
            .candidates(
                dir.path(),
                &targets,
                now + DISCOVERY_INTERVAL * 2 + REFRESH_INTERVAL,
            )
            .unwrap();
        assert!(!refreshed.values().any(|key| key == &npub));
        assert!(refreshed.values().any(|key| key == &unknown));
        assert!(
            locations
                .candidates(
                    dir.path(),
                    &BTreeSet::new(),
                    now + DISCOVERY_INTERVAL * 2 + REFRESH_INTERVAL
                )
                .unwrap()
                .is_empty()
        );
    }

    fn tests_fixture_principal(byte: u8) -> (String, String) {
        super::super::tests::principal(byte)
    }
}
