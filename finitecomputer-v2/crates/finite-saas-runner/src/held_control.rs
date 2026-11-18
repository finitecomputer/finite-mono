//! Runtime controls a Runner holds across runs.
//!
//! A retirement's Borg transfer runs in its own systemd unit and can take
//! hours, longer than any one run (systemd ends a run after an hour). The
//! Runner keeps that retirement's Core lease: the lease is recorded here the
//! moment Core grants it, before any work, and every later run renews it and
//! checks on the transfer. So a run that is killed leaves a lease the next run
//! continues within seconds, never one held with nothing working on it.
use crate::{RunnerError, sanitize_sandbox_name, write_secret_file};
use finite_saas_core::RuntimeControlLease;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const HELD_RUNTIME_CONTROL_SCHEMA: u8 = 1;
/// Masking retirement leases while one is held keeps this at one in practice.
const MAX_HELD_RUNTIME_CONTROLS: usize = 8;
const MAX_HELD_RUNTIME_CONTROL_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeldRuntimeControl {
    schema_version: u8,
    pub(crate) lease_token: String,
    pub(crate) lease: RuntimeControlLease,
}

#[derive(Debug, Clone)]
pub(crate) struct HeldControlStore {
    root: PathBuf,
}

fn store_error(context: &str, error: impl std::fmt::Display) -> RunnerError {
    RunnerError::RuntimeLaunch(format!("held runtime control store: {context}: {error}"))
}

impl HeldControlStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, request_id: &str) -> PathBuf {
        self.root
            .join(format!("{}.json", sanitize_sandbox_name(request_id)))
    }

    fn sync_root(&self) -> Result<(), RunnerError> {
        #[cfg(unix)]
        std::fs::File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| store_error("sync", error))?;
        Ok(())
    }

    /// Records a lease before any work runs under it.
    pub(crate) fn hold(
        &self,
        lease: &RuntimeControlLease,
        lease_token: &str,
    ) -> Result<(), RunnerError> {
        std::fs::create_dir_all(&self.root).map_err(|error| store_error("create", error))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| store_error("secure", error))?;
        }
        let record = HeldRuntimeControl {
            schema_version: HELD_RUNTIME_CONTROL_SCHEMA,
            lease_token: lease_token.to_string(),
            lease: lease.clone(),
        };
        let bytes = serde_json::to_vec(&record).map_err(|error| store_error("encode", error))?;
        if bytes.len() as u64 > MAX_HELD_RUNTIME_CONTROL_BYTES {
            return Err(store_error("encode", "record exceeds its bounded size"));
        }
        let path = self.path(&lease.request.id);
        let temporary = path.with_extension("json.tmp");
        write_secret_file(&temporary, &bytes).map_err(|error| store_error("write", error))?;
        std::fs::rename(&temporary, &path).map_err(|error| store_error("commit", error))?;
        self.sync_root()
    }

    /// Forgets a lease once Core has its outcome or no longer grants it.
    pub(crate) fn release(&self, request_id: &str) -> Result<(), RunnerError> {
        match std::fs::remove_file(self.path(request_id)) {
            Ok(()) => self.sync_root(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(store_error("release", error)),
        }
    }

    /// Every held lease, oldest request first, and every record this
    /// Runner cannot use. Those are kept as evidence for an operator; while
    /// any exists the Runner leases no Borg transfer, but its other controls
    /// still run.
    pub(crate) fn scan(&self) -> HeldScan {
        let mut scan = HeldScan::default();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return scan,
            Err(error) => {
                scan.unreadable
                    .push(format!("{}: {error}", self.root.display()));
                return scan;
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            match entry {
                // A temporary file is a hold that never committed: no work
                // ran under it.
                Ok(entry) if entry.path().extension().is_some_and(|ext| ext == "json") => {
                    paths.push(entry.path());
                }
                Ok(_) => {}
                Err(error) => scan
                    .unreadable
                    .push(format!("{}: {error}", self.root.display())),
            }
        }
        if paths.len() > MAX_HELD_RUNTIME_CONTROLS {
            scan.unreadable.push(format!(
                "{}: {} held leases, more than its bound of {MAX_HELD_RUNTIME_CONTROLS}",
                self.root.display(),
                paths.len()
            ));
            return scan;
        }
        for path in paths {
            match read_record(&path) {
                Ok(record)
                    if record.schema_version == HELD_RUNTIME_CONTROL_SCHEMA
                        && path == self.path(&record.lease.request.id)
                        && !record.lease_token.trim().is_empty() =>
                {
                    scan.held.push(record);
                }
                Ok(_) => scan.unreadable.push(format!(
                    "{}: the record does not match its lease",
                    path.display()
                )),
                Err(error) => scan.unreadable.push(format!("{}: {error}", path.display())),
            }
        }
        scan.held.sort_by(|left, right| {
            (&left.lease.request.created_at, &left.lease.request.id)
                .cmp(&(&right.lease.request.created_at, &right.lease.request.id))
        });
        scan
    }
}

#[derive(Debug, Default)]
pub(crate) struct HeldScan {
    pub(crate) held: Vec<HeldRuntimeControl>,
    pub(crate) unreadable: Vec<String>,
}

fn read_record(path: &Path) -> Result<HeldRuntimeControl, RunnerError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(MAX_HELD_RUNTIME_CONTROL_BYTES + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|error| store_error("read", error))?;
    if bytes.len() as u64 > MAX_HELD_RUNTIME_CONTROL_BYTES {
        return Err(store_error("read", "record exceeds its bounded size"));
    }
    serde_json::from_slice(&bytes).map_err(|error| store_error("decode", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(request_id: &str, created_at: &str) -> RuntimeControlLease {
        let mut lease = crate::tests::sample_runtime_control_lease(request_id);
        lease.request.created_at = created_at.to_string();
        lease
    }

    #[test]
    fn holds_survive_a_restart_and_release_forgets_them() {
        let temp = tempfile::tempdir().unwrap();
        let store = HeldControlStore::new(temp.path().join("held"));
        assert!(store.scan().held.is_empty());
        store
            .hold(&lease("runtime_ctl_b", "2026-11-18T16:00:01Z"), "token-b")
            .unwrap();
        store
            .hold(&lease("runtime_ctl_a", "2026-11-18T16:00:00Z"), "token-a")
            .unwrap();
        // A hold that never committed ran no work.
        std::fs::write(temp.path().join("held/runtime_ctl_c.json.tmp"), b"{").unwrap();

        let reopened = HeldControlStore::new(temp.path().join("held"));
        let scan = reopened.scan();
        assert!(scan.unreadable.is_empty(), "{:?}", scan.unreadable);
        let held = scan.held;
        assert_eq!(
            held.iter()
                .map(|record| (
                    record.lease.request.id.as_str(),
                    record.lease_token.as_str()
                ))
                .collect::<Vec<_>>(),
            [("runtime_ctl_a", "token-a"), ("runtime_ctl_b", "token-b")]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&temp.path().join("held")), 0o700);
            assert_eq!(mode(&temp.path().join("held/runtime_ctl_a.json")), 0o600);
        }

        reopened.release("runtime_ctl_a").unwrap();
        reopened.release("runtime_ctl_a").unwrap();
        assert_eq!(reopened.scan().held.len(), 1);
    }

    #[test]
    fn a_record_that_does_not_match_its_lease_is_reported_and_kept() {
        let temp = tempfile::tempdir().unwrap();
        let store = HeldControlStore::new(temp.path().join("held"));
        store
            .hold(&lease("runtime_ctl_a", "2026-11-18T16:00:00Z"), "token-a")
            .unwrap();
        store
            .hold(&lease("runtime_ctl_b", "2026-11-18T16:00:01Z"), "token-b")
            .unwrap();
        std::fs::rename(
            temp.path().join("held/runtime_ctl_a.json"),
            temp.path().join("held/runtime_ctl_other.json"),
        )
        .unwrap();
        let scan = store.scan();
        assert_eq!(scan.held.len(), 1);
        assert_eq!(scan.unreadable.len(), 1);
        assert!(scan.unreadable[0].contains("does not match its lease"));

        std::fs::write(temp.path().join("held/runtime_ctl_other.json"), b"not json").unwrap();
        let scan = store.scan();
        assert_eq!(scan.held.len(), 1);
        assert!(
            scan.unreadable[0].contains("decode"),
            "{:?}",
            scan.unreadable
        );
        // Evidence is never removed by a scan.
        assert!(temp.path().join("held/runtime_ctl_other.json").is_file());
    }
}
