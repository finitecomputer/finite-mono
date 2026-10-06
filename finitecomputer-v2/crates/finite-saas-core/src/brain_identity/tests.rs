use super::*;

const FIXTURE: &str =
    include_str!("../../../../docs/contracts/brain-identity-descriptions-v1.example.json");

#[test]
fn npub_encoding_matches_the_nip19_vector() {
    assert_eq!(
        npub_for_hex("3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d").as_deref(),
        Some("npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6")
    );
    assert_eq!(
        npub_for_hex("3BF0C63FCB93463407AF97A5E5EE64FA883D107EF9E558472C4EB9AAAEFA459D"),
        None
    );
    assert_eq!(npub_for_hex("npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7"), None);
}

#[test]
fn brain_server_identity_has_one_accepted_form() {
    assert_eq!(
        canonical_brain_server("https://brain.finite.computer").as_deref(),
        Some("https://brain.finite.computer")
    );
    assert!(canonical_brain_server("https://brain.local:8443").is_some());
    assert!(canonical_brain_server("http://127.0.0.1:3015").is_some());
    assert!(canonical_brain_server("http://localhost:3015").is_some());
    for rejected in [
        "https://brain.finite.computer/",
        "https://Brain.finite.computer",
        "HTTPS://brain.finite.computer",
        "http://brain.finite.computer",
        "http://127.0.0.1",
        "http://127.0.0.1:080",
        "http://10.0.0.1:8080",
        "http://localhost:3015/",
        "https://brain.finite.computer:443",
        "https://brain.finite.computer:0",
        "https://brain.finite.computer:08443",
        "https://user@brain.finite.computer",
        "https://brain.finite.computer/v1",
        "https://brain.finite.computer?x",
        "https://",
        "https://.brain",
    ] {
        assert_eq!(canonical_brain_server(rejected), None, "{rejected}");
    }
}

#[test]
fn shared_fixture_round_trips_through_the_core_serializer() {
    let parsed: BrainIdentityDescriptionsResponse =
        serde_json::from_str(FIXTURE).expect("fixture parses");
    assert_eq!(parsed.version, DESCRIPTIONS_VERSION);
    let reserialized = serde_json::to_value(&parsed).unwrap();
    let original: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(reserialized, original);
    let states = parsed
        .results
        .iter()
        .map(|row| row.state)
        .collect::<std::collections::BTreeSet<_>>();
    for state in [
        DescriptionState::Resolved,
        DescriptionState::Unknown,
        DescriptionState::Ambiguous,
        DescriptionState::NotShared,
    ] {
        assert!(states.contains(&state), "fixture lacks {state:?}");
    }
}

#[test]
fn description_requests_are_bounded_exact_and_canonical() {
    let key = "a".repeat(64);
    let mut request = BrainIdentityDescriptionsRequest {
        version: DESCRIPTIONS_VERSION.to_string(),
        brain_server: "https://brain.test".to_string(),
        brain_id: "brain_1".to_string(),
        requested_by_public_key_hex: key.clone(),
        public_keys_hex: vec![key.clone()],
    };
    assert_eq!(request.validate("https://brain.test"), Ok(()));
    assert!(request.requires_sharing_scope());
    request.version = DESCRIPTIONS_VERSION_V2.to_string();
    assert_eq!(request.validate("https://brain.test"), Ok(()));
    assert!(!request.requires_sharing_scope());
    // Brain falls back to v1 only on exactly this error's message.
    request.version = "finite-core-brain-identity-descriptions-v3".to_string();
    assert_eq!(
        request.validate("https://brain.test"),
        Err(DescriptionsInputError::Version)
    );
    assert_eq!(
        DescriptionsInputError::Version.to_string(),
        "unsupported descriptions version"
    );
    request.version = DESCRIPTIONS_VERSION.to_string();
    assert_eq!(
        request.validate("https://other.test"),
        Err(DescriptionsInputError::BrainServer)
    );
    request.public_keys_hex = vec![key.to_uppercase()];
    assert_eq!(
        request.validate("https://brain.test"),
        Err(DescriptionsInputError::PublicKey)
    );
    request.public_keys_hex = vec![key.clone(), key.clone()];
    assert_eq!(
        request.validate("https://brain.test"),
        Err(DescriptionsInputError::KeyCount)
    );
    request.public_keys_hex = (0..=MAX_DESCRIPTION_KEYS)
        .map(|index| format!("{index:064x}"))
        .collect();
    assert_eq!(
        request.validate("https://brain.test"),
        Err(DescriptionsInputError::KeyCount)
    );
    request.public_keys_hex = Vec::new();
    assert_eq!(
        request.validate("https://brain.test"),
        Err(DescriptionsInputError::KeyCount)
    );
    let unknown_field = serde_json::json!({
        "version": DESCRIPTIONS_VERSION, "brainServer": "https://brain.test",
        "brainId": "b", "requestedByPublicKeyHex": key, "publicKeysHex": [key],
        "email": "x@example.org"
    });
    assert!(serde_json::from_value::<BrainIdentityDescriptionsRequest>(unknown_field).is_err());
}

/// v2 releases a current owner on the strength of an Agent key's past Brain
/// participation. That is sound only while a project's owner never changes:
/// a transfer writer must first stop pre-transfer participation releasing the
/// new owner (see "Ownership transfer" in brain-identity-descriptions-v1.md).
#[test]
fn no_core_writer_transfers_project_ownership() {
    fn sources(dir: &std::path::Path, found: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "tests") {
                    sources(&path, found);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && path.file_name().is_some_and(|name| name != "tests.rs")
            {
                let text = std::fs::read_to_string(&path).unwrap();
                found.push((path.display().to_string(), text));
            }
        }
    }
    let mut found = Vec::new();
    sources(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    assert!(found.len() > 20, "source scan found too few files");
    for (path, text) in found {
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        for statement in flat.split("UPDATE projects").skip(1) {
            let statement = statement.split(';').next().unwrap_or_default();
            let set_clause = statement.split(" WHERE ").next().unwrap_or_default();
            assert!(
                !set_clause.contains("owner_user_id"),
                "{path} changes projects.owner_user_id; handle ownership transfer for v2 Brain descriptions first"
            );
        }
    }
}

#[test]
fn human_actions_must_observe_the_participating_key() {
    let key = "b".repeat(64);
    let mut request = BrainAccountObservationRequest {
        version: OBSERVATION_VERSION.to_string(),
        operation_id: "op_0123456789abcdef".to_string(),
        brain_server: "https://brain.test".to_string(),
        brain_id: "brain_1".to_string(),
        observed_at: "2026-10-04T00:00:00Z".to_string(),
        action_kind: ObservationActionKind::HumanHostedAction,
        observed_human_public_key_hex: Some(key.clone()),
        participating_public_key_hex: key.clone(),
    };
    assert_eq!(request.validate("https://brain.test"), Ok(()));
    request.observed_human_public_key_hex = Some("c".repeat(64));
    assert_eq!(
        request.validate("https://brain.test"),
        Err(ObservationInputError::HumanKeyMismatch)
    );
    request.observed_human_public_key_hex = None;
    assert_eq!(
        request.validate("https://brain.test"),
        Err(ObservationInputError::HumanKeyMismatch)
    );
    request.action_kind = ObservationActionKind::OwnedAgentHostedAction;
    assert_eq!(request.validate("https://brain.test"), Ok(()));
    request.operation_id = "short".to_string();
    assert_eq!(
        request.validate("https://brain.test"),
        Err(ObservationInputError::OperationId)
    );
}

#[test]
fn the_private_listener_refuses_wildcard_and_public_binds() {
    for accepted in [
        "127.0.0.1:4202",
        "10.0.0.2:4202",
        "192.168.1.2:4202",
        "[::1]:4202",
        "[fd00::2]:4202",
    ] {
        assert!(private_bind(accepted), "{accepted}");
    }
    for refused in [
        "0.0.0.0:4202",
        "[::]:4202",
        "64.34.80.19:4202",
        "[2001:db8::1]:4202",
        "localhost:4202",
        "127.0.0.1",
    ] {
        assert!(!private_bind(refused), "{refused}");
    }
}
