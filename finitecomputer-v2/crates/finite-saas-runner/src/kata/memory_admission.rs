//! Admission headroom for the fleet's 8-GiB Kata guests. These observations
//! prevent starting another guest under pressure; they do not reserve memory
//! against subsequent growth of existing guests or guarantee 32 full guests.
use super::{KataConfig, KataRuntimeOperationLock, RunnerError};
use std::fs::{File, OpenOptions};
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const GIB: u64 = 1024 * 1024 * 1024;
// One complete 8-GiB guest plus 16 GiB of host headroom and 8 GiB of
// shared-memory headroom. /dev/shm can fill while MemAvailable remains high.
const MIN_AVAILABLE_MEMORY: u64 = 24 * GIB;
const MIN_AVAILABLE_SHARED_MEMORY: u64 = 16 * GIB;

#[derive(Debug)]
pub(super) struct MemoryAdmission {
    pub available_memory_bytes: Option<u64>,
    available_shared_memory_bytes: Option<u64>,
}

impl MemoryAdmission {
    pub fn read(host_root: &Path) -> Self {
        let available_memory_bytes = (|| {
            let mut text = String::new();
            File::open(host_root.join("proc/meminfo"))
                .ok()?
                .take(65537)
                .read_to_string(&mut text)
                .ok()?;
            (text.len() <= 65536).then_some(())?;
            parse_available_memory(&text)
        })();
        Self {
            available_memory_bytes,
            available_shared_memory_bytes: fs4::available_space(host_root.join("dev/shm")).ok(),
        }
    }

    pub fn rejection_reason(&self) -> Option<&'static str> {
        match (
            self.available_memory_bytes,
            self.available_shared_memory_bytes,
        ) {
            (None, _) | (_, None) => {
                Some("Kata memory admission: host memory observation unavailable")
            }
            (Some(memory), _) if memory < MIN_AVAILABLE_MEMORY => {
                Some("Kata memory admission: less than 24 GiB of available RAM")
            }
            (_, Some(shared)) if shared < MIN_AVAILABLE_SHARED_MEMORY => {
                Some("Kata memory admission: less than 16 GiB free in /dev/shm")
            }
            _ => None,
        }
    }

    pub fn require_capacity(&self) -> Result<(), RunnerError> {
        self.rejection_reason().map_or(Ok(()), |reason| {
            Err(RunnerError::RuntimeLaunch(reason.into()))
        })
    }
}

fn parse_available_memory(text: &str) -> Option<u64> {
    let mut matches = text
        .lines()
        .filter_map(|line| line.strip_prefix("MemAvailable:"));
    let mut fields = matches.next()?.split_whitespace();
    let bytes = fields.next()?.parse::<u64>().ok()?.checked_mul(1024)?;
    (fields.next() == Some("kB") && fields.next().is_none() && matches.next().is_none())
        .then_some(bytes)
}

/// Serialize creation checks through readiness so concurrent workers cannot
/// admit guests from the same pre-launch observation. Refuse contention instead
/// of waiting past a creation lease. Existing lifecycle operations do not wait
/// on this lock and can still free resources.
pub(super) fn lock_creation(config: &KataConfig) -> Result<KataRuntimeOperationLock, RunnerError> {
    std::fs::create_dir_all(&config.work_root)
        .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(config.work_root.join("kata-creation.lock"))
        .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;
    file.try_lock().map_err(|_| {
        RunnerError::RuntimeLaunch("Kata memory admission: another creation is in progress".into())
    })?;
    Ok(KataRuntimeOperationLock { file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_admission_requires_both_independent_headrooms() {
        for (memory, shared, allowed) in [
            (Some(24 * GIB), Some(16 * GIB), true),
            (Some(24 * GIB - 1), Some(80 * GIB), false),
            (Some(100 * GIB), Some(16 * GIB - 1), false),
            (None, Some(80 * GIB), false),
            (Some(100 * GIB), None, false),
        ] {
            let observed = MemoryAdmission {
                available_memory_bytes: memory,
                available_shared_memory_bytes: shared,
            };
            assert_eq!(observed.require_capacity().is_ok(), allowed, "{observed:?}");
        }
    }

    #[test]
    fn memory_admission_rejects_missing_malformed_or_overflowing_meminfo() {
        assert_eq!(
            parse_available_memory("MemTotal: 1 kB\nMemAvailable: 42 kB\n"),
            Some(43008)
        );
        for text in [
            "",
            "MemFree: 42 kB",
            "MemAvailable: -1 kB",
            "MemAvailable: 42 MB",
            "MemAvailable: 18446744073709551615 kB",
            "MemAvailable: 42 kB extra",
            "MemAvailable: 42 kB\nMemAvailable: 45 kB",
        ] {
            assert_eq!(parse_available_memory(text), None, "{text}");
        }
    }

    #[test]
    fn memory_admission_creation_lock_refuses_concurrent_launches_and_releases() {
        let temp = tempfile::tempdir().unwrap();
        let config = KataConfig {
            work_root: temp.path().into(),
            ..KataConfig::default()
        };
        let lock = lock_creation(&config).unwrap();
        assert!(lock_creation(&config).is_err());
        drop(lock);
        assert!(lock_creation(&config).is_ok());
    }
}
