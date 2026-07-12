use crate::{CoreError, CoreResult, HostingTier, generate_surrogate_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

pub const DEFAULT_LAUNCH_CODE_BATCH_HOURS: i64 = 7 * 24;
pub const MAX_LAUNCH_CODE_BATCH_HOURS: i64 = 30 * 24;
pub const MAX_LAUNCH_CODE_BATCH_SIZE: u32 = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchCodeBatch {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub hosting_tier: Option<HostingTier>,
    pub code_count: u32,
    pub expires_at: String,
    pub revoked_at: Option<String>,
    pub revoked_by_workos_user_id: Option<String>,
    pub created_by_workos_user_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchCodeStatus {
    pub id: String,
    pub redeemed_customer_org_id: Option<String>,
    pub redeemed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchCodeBatchDetails {
    pub batch: LaunchCodeBatch,
    pub codes: Vec<LaunchCodeStatus>,
}

/// Plaintext is deliberately returned only by batch issuance. This type does
/// not implement `Debug` or `Deserialize`, reducing the chance that ordinary
/// diagnostics or later reads accidentally expose the code material.
#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct IssuedLaunchCode {
    pub id: String,
    pub code: String,
}

/// One-time issuance response. Later list/revoke operations return
/// `LaunchCodeBatchDetails`, which contains metadata only.
#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct IssuedLaunchCodeBatch {
    pub batch: LaunchCodeBatch,
    pub codes: Vec<IssuedLaunchCode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueLaunchCodeBatchInput {
    pub name: String,
    pub code_count: u32,
    pub expires_in_hours: Option<i64>,
    #[serde(default)]
    pub hosting_tier: Option<HostingTier>,
    pub created_by_workos_user_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RevokeLaunchCodeBatchInput {
    pub batch_id: String,
    pub revoked_by_workos_user_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchCodeRecord {
    pub id: String,
    pub batch_id: String,
    pub code_hash: String,
    pub redeemed_customer_org_id: Option<String>,
    pub redemption_idempotency_key: Option<String>,
    pub redeemed_at: Option<String>,
    pub created_at: String,
}

impl LaunchCodeRecord {
    pub(crate) fn status(&self) -> LaunchCodeStatus {
        LaunchCodeStatus {
            id: self.id.clone(),
            redeemed_customer_org_id: self.redeemed_customer_org_id.clone(),
            redeemed_at: self.redeemed_at.clone(),
        }
    }
}

pub(crate) struct PreparedLaunchCodeBatch {
    pub batch: LaunchCodeBatch,
    pub records: Vec<LaunchCodeRecord>,
    pub issued_codes: Vec<IssuedLaunchCode>,
}

pub(crate) fn prepare_launch_code_batch(
    input: IssueLaunchCodeBatchInput,
) -> CoreResult<PreparedLaunchCodeBatch> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(CoreError::MissingLaunchCodeBatchName);
    }
    if name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err(CoreError::InvalidLaunchCodeBatchName);
    }
    if input.code_count == 0 || input.code_count > MAX_LAUNCH_CODE_BATCH_SIZE {
        return Err(CoreError::InvalidLaunchCodeBatchSize);
    }

    let actor = input.created_by_workos_user_id.trim();
    if actor.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let hours = input
        .expires_in_hours
        .unwrap_or(DEFAULT_LAUNCH_CODE_BATCH_HOURS);
    if !(1..=MAX_LAUNCH_CODE_BATCH_HOURS).contains(&hours) {
        return Err(CoreError::InvalidLaunchCodeBatchExpiry);
    }
    let created_at = match input.now {
        Some(value) => {
            OffsetDateTime::parse(&value, &Rfc3339).map_err(|_| CoreError::InvalidTimestamp)?
        }
        None => OffsetDateTime::now_utc(),
    };
    let expires_at = created_at
        .checked_add(Duration::hours(hours))
        .ok_or(CoreError::InvalidLaunchCodeBatchExpiry)?;
    let created_at = created_at.format(&Rfc3339)?;
    let expires_at = expires_at.format(&Rfc3339)?;
    let batch = LaunchCodeBatch {
        id: generate_surrogate_id("launch_batch")?,
        name: name.to_string(),
        hosting_tier: Some(input.hosting_tier.unwrap_or(HostingTier::Standard)),
        code_count: input.code_count,
        expires_at,
        revoked_at: None,
        revoked_by_workos_user_id: None,
        created_by_workos_user_id: actor.to_string(),
        created_at: created_at.clone(),
    };

    let mut records = Vec::with_capacity(input.code_count as usize);
    let mut issued_codes = Vec::with_capacity(input.code_count as usize);
    let mut seen_hashes = BTreeSet::new();
    while issued_codes.len() < input.code_count as usize {
        let code = generate_launch_code()?;
        let code_hash = hash_launch_code(&code)?;
        if !seen_hashes.insert(code_hash.clone()) {
            continue;
        }
        let id = generate_surrogate_id("launch_code")?;
        records.push(LaunchCodeRecord {
            id: id.clone(),
            batch_id: batch.id.clone(),
            code_hash,
            redeemed_customer_org_id: None,
            redemption_idempotency_key: None,
            redeemed_at: None,
            created_at: created_at.clone(),
        });
        issued_codes.push(IssuedLaunchCode { id, code });
    }

    Ok(PreparedLaunchCodeBatch {
        batch,
        records,
        issued_codes,
    })
}

pub(crate) fn hash_launch_code(value: &str) -> CoreResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(CoreError::MissingLaunchCode);
    }
    let digest = Sha256::digest(value.as_bytes());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn generate_launch_code() -> CoreResult<String> {
    let mut bytes = [0_u8; 18];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| CoreError::Store(format!("failed to generate Launch Code: {error}")))?;
    let mut code = String::with_capacity("finite_".len() + bytes.len() * 2);
    code.push_str("finite_");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut code, "{byte:02x}")
            .map_err(|error| CoreError::Store(format!("failed to render Launch Code: {error}")))?;
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issuance_defaults_to_seven_days_and_returns_distinct_codes() {
        let prepared = prepare_launch_code_batch(IssueLaunchCodeBatchInput {
            name: "Canary training".to_string(),
            code_count: 12,
            expires_in_hours: None,
            hosting_tier: None,
            created_by_workos_user_id: "user_operator".to_string(),
            now: Some("2026-07-10T12:00:00Z".to_string()),
        })
        .expect("prepare batch");

        assert_eq!(prepared.batch.code_count, 12);
        assert_eq!(prepared.batch.hosting_tier, Some(HostingTier::Standard));
        assert_eq!(prepared.batch.expires_at, "2026-07-17T12:00:00Z");
        assert_eq!(prepared.records.len(), 12);
        assert_eq!(prepared.issued_codes.len(), 12);
        let unique = prepared
            .issued_codes
            .iter()
            .map(|code| code.code.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), 12);
        for (record, issued) in prepared.records.iter().zip(&prepared.issued_codes) {
            assert_eq!(record.id, issued.id);
            assert_eq!(record.code_hash, hash_launch_code(&issued.code).unwrap());
            assert_ne!(record.code_hash, issued.code);
        }
    }

    #[test]
    fn issuance_persists_explicit_confidential_hosting_tier() {
        let prepared = prepare_launch_code_batch(IssueLaunchCodeBatchInput {
            name: "Confidential canary".to_string(),
            code_count: 1,
            expires_in_hours: Some(24),
            hosting_tier: Some(HostingTier::Confidential),
            created_by_workos_user_id: "user_operator".to_string(),
            now: Some("2026-07-10T12:00:00Z".to_string()),
        })
        .expect("prepare confidential batch");
        assert_eq!(prepared.batch.hosting_tier, Some(HostingTier::Confidential));
    }

    #[test]
    fn issuance_rejects_indefinite_or_overlong_batches() {
        for hours in [0, MAX_LAUNCH_CODE_BATCH_HOURS + 1] {
            let error = prepare_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "Invalid".to_string(),
                code_count: 1,
                expires_in_hours: Some(hours),
                hosting_tier: None,
                created_by_workos_user_id: "user_operator".to_string(),
                now: Some("2026-07-10T12:00:00Z".to_string()),
            })
            .err()
            .expect("invalid expiry");
            assert!(matches!(error, CoreError::InvalidLaunchCodeBatchExpiry));
        }
    }
}
