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
