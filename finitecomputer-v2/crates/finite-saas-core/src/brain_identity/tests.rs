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

/// True when `sql` assigns `projects.owner_user_id` in an UPDATE or in an
/// INSERT's `ON CONFLICT ... DO UPDATE SET`. A tripwire for ordinary SQL
/// spellings, not a parser.
fn transfers_project_owner(sql: &str) -> bool {
    let flat = sql
        .to_ascii_lowercase()
        .replace('"', "")
        .replace('=', " = ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let projects = |text: &str| {
        let text = text.strip_prefix("only ").unwrap_or(text);
        let text = text.strip_prefix("public.").unwrap_or(text);
        text.starts_with("projects ") || text.starts_with("projects(")
    };
    let assigns_owner = |set_list: &str| {
        let set_list = set_list.split(';').next().unwrap_or_default();
        let set_list = set_list.split(" where ").next().unwrap_or_default();
        let set_list = set_list.split(" from ").next().unwrap_or_default();
        set_list.contains("owner_user_id =")
    };
    let updates = flat
        .split("update ")
        .skip(1)
        .any(|statement| projects(statement) && assigns_owner(statement));
    let upserts = flat.split("insert into ").skip(1).any(|statement| {
        projects(statement)
            && statement
                .split(';')
                .next()
                .unwrap_or_default()
                .split("do update set ")
                .nth(1)
                .is_some_and(assigns_owner)
    });
    updates || upserts
}

#[test]
fn transfer_detector_recognizes_ordinary_spellings() {
    for transfer in [
        "UPDATE projects SET owner_user_id = $1 WHERE id = $2",
        "update public.projects\n  set display_name = $1, owner_user_id=$2 where id = $3",
        "UPDATE ONLY \"projects\" AS p SET owner_user_id = $1",
        "INSERT INTO projects (id, owner_user_id) VALUES ($1, $2)
         ON CONFLICT (id) DO UPDATE SET owner_user_id = EXCLUDED.owner_user_id",
    ] {
        assert!(transfers_project_owner(transfer), "{transfer}");
    }
    for other in [
        "UPDATE projects SET display_name = $1 WHERE owner_user_id = $2",
        "UPDATE projects_archive SET owner_user_id = $1",
        "UPDATE project_runtime_links SET active = false",
        "UPDATE projects AS p SET hosting_tier = r.tier FROM requests r WHERE r.owner_user_id = p.owner_user_id",
        // Today's upsert: the owner is inserted, never reassigned.
        "INSERT INTO projects (id, owner_user_id) VALUES ($1, $2)
         ON CONFLICT (id) DO UPDATE SET display_name = EXCLUDED.display_name\", &[&id, &owner_user_id]);",
    ] {
        assert!(!transfers_project_owner(other), "{other}");
    }
}

/// v2 releases a current owner on the strength of an Agent key's past Brain
/// participation. That is sound only while a project's owner never changes:
/// a transfer writer must first stop pre-transfer participation releasing the
/// new owner (see "Ownership transfer" in brain-identity-descriptions-v1.md).
/// Scans non-test Core sources and migrations.
#[test]
fn no_core_writer_transfers_project_ownership() {
    fn sources(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap_or_default();
            if path.is_dir() {
                if name != "tests" {
                    sources(&path, found);
                }
            } else if path
                .extension()
                .is_some_and(|ext| ext == "rs" || ext == "sql")
                && name != "tests.rs"
            {
                found.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    sources(&root.join("src"), &mut found);
    sources(&root.join("migrations"), &mut found);
    assert!(found.len() > 40, "source scan found too few files");
    for path in found {
        assert!(
            !transfers_project_owner(&std::fs::read_to_string(&path).unwrap()),
            "{} changes projects.owner_user_id; handle ownership transfer for v2 Brain descriptions first",
            path.display()
        );
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
