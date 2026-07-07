use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::CliEnvironment;

pub(crate) fn timestamp(env: &CliEnvironment) -> String {
    env.now.clone().unwrap_or_else(|| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0));
        timestamp_from_unix(now.as_secs())
    })
}

pub(crate) fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs()
}

pub(crate) fn timestamp_from_unix(seconds: u64) -> String {
    let datetime = i64::try_from(seconds)
        .ok()
        .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok())
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    datetime
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

pub(crate) fn auth_nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_nanos();
    deterministic_id("nonce", &[&nanos.to_string()])
}

pub(crate) fn deterministic_id(prefix: &str, parts: &[&str]) -> String {
    let digest = Sha256::digest(parts.join("\n").as_bytes());
    format!(
        "{prefix}-{}",
        digest
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
