//! One process serves Brain-scoped demand; expensive discovery has one global gate.
use super::*;
use finite_brain_server::principal_labels::LABEL_REFRESH_INTERVAL_SECONDS;

const DISCOVERY_INTERVAL: Duration = Duration::from_secs(15 * 60);
const REFRESH_INTERVAL: Duration = Duration::from_secs(LABEL_REFRESH_INTERVAL_SECONDS);

/// One complete, temporary public-key index serves every Brain. Unknown keys
/// share the discovery cooldown: adding arbitrary npubs cannot force new scans.
#[derive(Default)]
pub(super) struct HostedLocations {
    discovered_at: Option<Instant>,
    discovery_failed: bool,
    index: Option<HostedIndex>,
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
        let started = Instant::now();
        if self
            .discovered_at
            .is_none_or(|last| now.duration_since(last) >= DISCOVERY_INTERVAL)
        {
            // Failed discovery shares the same cooldown. Keep the old output
            // to expire; never republish it as newly verified after a scan error.
            self.discovered_at = Some(now);
            self.discovery_failed = true;
            self.index = Some(discover_hosted(root, started)?);
            self.discovery_failed = false;
        }
        if self.discovery_failed {
            return Err(RefreshError::SourceUnavailable);
        }

        let mut current = BTreeMap::new();
        let mut examined = 0;
        let index = self.index.as_ref().ok_or(RefreshError::SourceUnavailable)?;
        let mut statement = index
            .connection
            .prepare("SELECT storage_id FROM locations WHERE npub=?1 LIMIT ?2")
            .map_err(|_| RefreshError::InvalidSource)?;
        for npub in principals {
            let rows = statement
                .query_map(rusqlite::params![npub, MAX_LABELS as u64 * 2 + 1], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|_| RefreshError::InvalidSource)?;
            for row in rows {
                examined += 1;
                // Count every indexed claim, including changed keys. Truncating
                // before revalidation could hide a conflicting owner.
                if examined > MAX_LABELS * 2 {
                    return Err(RefreshError::Limit);
                }
                if started.elapsed() > Duration::from_secs(25) {
                    return Err(RefreshError::Limit);
                }
                let storage_id = row.map_err(|_| RefreshError::InvalidSource)?;
                if hosted_public_key(root, &storage_id)?.as_ref() == Some(npub) {
                    current.insert(storage_id, npub.clone());
                }
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
            Duration::ZERO
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
    let directory = PathBuf::from(
        std::env::var("FINITE_BRAIN_PRINCIPAL_LABELS_DIR")
            .map_err(|_| RefreshError::Configuration)?,
    );
    let mut bytes = [0_u8; 129];
    loop {
        let len = socket
            .recv(&mut bytes)
            .await
            .map_err(|_| RefreshError::Configuration)?;
        let Ok(value) = std::str::from_utf8(&bytes[..len]) else {
            continue;
        };
        let Ok(brain_id) = BrainId::new(value) else {
            continue;
        };
        // Per-Brain success cooldown lives in its disposable file; no growing
        // in-memory roster/gate map and no one Brain's success starving another.
        let fresh = fs::metadata(projection_path(&directory, &brain_id))
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age < REFRESH_INTERVAL);
        if fresh || !gate.ready(Instant::now()) {
            continue;
        }
        let result =
            tokio::time::timeout(Duration::from_secs(30), refresh(&brain_id, &mut locations)).await;
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
        assert!(gate.ready(now), "another Brain can run after success");
        let retry = now;
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

    #[test]
    fn one_discovery_serves_different_brains_without_a_global_roster() {
        let dir = tempfile::tempdir().unwrap();
        let (first_hex, first) = tests_fixture_principal(1);
        let (second_hex, second) = tests_fixture_principal(2);
        super::super::tests::hosted_fixture(dir.path(), "first", &first_hex);
        super::super::tests::hosted_fixture(dir.path(), "second", &second_hex);
        let mut locations = HostedLocations::default();
        let now = Instant::now();
        let a = locations
            .candidates(dir.path(), &BTreeSet::from([first.clone()]), now)
            .unwrap();
        assert_eq!(a.values().collect::<Vec<_>>(), vec![&first]);
        let broken = super::super::tests::hosted_fixture(
            dir.path(),
            "unrelated",
            &tests_fixture_principal(3).0,
        );
        fs::write(broken, b"broken").unwrap();
        let b = locations
            .candidates(
                dir.path(),
                &BTreeSet::from([second.clone()]),
                now + Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(b.values().collect::<Vec<_>>(), vec![&second]);
        assert_eq!(
            locations.discovered_at,
            Some(now),
            "a second Brain must reuse complete discovery"
        );
    }

    fn tests_fixture_principal(byte: u8) -> (String, String) {
        super::super::tests::principal(byte)
    }
}
