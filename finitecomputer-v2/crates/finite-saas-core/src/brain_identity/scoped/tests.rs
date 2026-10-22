use super::*;

#[test]
fn annotation_bounds_are_ascii_and_real_utc_dates() {
    for label in ["Historical Agent: Ada", "Invitation key", &"x".repeat(120)] {
        assert!(valid_annotation_label(label));
    }
    for label in [
        "",
        " leading",
        "trailing ",
        "line\nbreak",
        "A — B",
        &"x".repeat(121),
    ] {
        assert!(!valid_annotation_label(label));
    }
    for value in ["2026-01-01T00:00:00Z", "2024-02-29T00:00:00.123456789Z"] {
        assert!(valid_evidence_time(value));
    }
    for value in [
        "2026-02-30T00:00:00Z",
        "0000-01-01T00:00:00Z",
        "2026-01-01T00:00:00+00:00",
        "2026-01-01T23:59:60Z",
        "yesterday",
    ] {
        assert!(!valid_evidence_time(value), "{value}");
    }
}

#[test]
fn strict_v3_dtos_do_not_relax_the_legacy_decoder() {
    let value = serde_json::json!({ "version": DESCRIPTIONS_VERSION_V3, "brainServer": "https://brain.test", "brainId": "example",
        "requestedByPublicKeyHex": "ee".repeat(32), "publicKeysHex": ["01".repeat(32)], "participatingPublicKeysHex": [] });
    let request: ScopedDescriptionsRequest = serde_json::from_value(value.clone()).unwrap();
    assert!(request.validate("https://brain.test").is_ok());
    assert!(serde_json::from_value::<BrainIdentityDescriptionsRequest>(value.clone()).is_err());
    let mut duplicate = request.clone();
    duplicate.participating_public_keys_hex = vec!["01".repeat(32); 2];
    assert!(duplicate.validate("https://brain.test").is_err());
    let mut extra = value;
    extra["unknown"] = true.into();
    assert!(serde_json::from_value::<ScopedDescriptionsRequest>(extra).is_err());
}

#[test]
fn shared_v3_fixture_round_trips_all_28_synthetic_rows_without_private_evidence() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../docs/contracts/brain-identity-descriptions-v3.example.json"
    ))
    .unwrap();
    let response: ScopedDescriptionsResponse = serde_json::from_value(fixture.clone()).unwrap();
    assert_eq!(response.results.len(), 28);
    assert_eq!(serde_json::to_value(response).unwrap(), fixture);
    assert!(serde_json::from_value::<BrainIdentityDescriptionsResponse>(fixture).is_err());
}
