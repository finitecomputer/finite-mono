//! End-to-end proofs for `finitechat repair skip-entry`: the derived skip
//! list, the two-phase fail-closed apply, the max-skips bound, and the
//! durable audit trail. Every fixture is local: an in-process finitechat
//! server, fabricated devices, and fabricated store copies. Poison entries
//! are fabricated exactly like the rejected-entry classifier's tests: a
//! Commit entry relabeled as Application classifies as
//! `mls_application_ciphertext`, and bytes that no longer deserialize
//! classify as `protocol_envelope_parsing`.

use finitechat_client::rejected_entry_diagnostic::CapturedRoomLogFile;
use finitechat_client::{
    FiniteChatDevice, FiniteChatDeviceConfig, HttpRuntimeDelivery, ReqwestHttpRuntimeTransport,
    RuntimeDelivery, RuntimeSyncOptions, SqliteClientStore, SqliteClientStoreOptions,
    run_runtime_sync_tick,
};
use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
use finitechat_proto::{
    CreateRoomRequest, DurableAppEventKind, LogEntryKind, RoomLogEntry, RoomProtocol,
};
use finitechat_server::{HttpServerState, http_router};
use serde_json::Value;
use sha2::{Digest, Sha256};

const ALICE_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [61; NOSTR_SECRET_KEY_BYTES];
const HOSTED_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [63; NOSTR_SECRET_KEY_BYTES];
const CHARLIE_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [67; NOSTR_SECRET_KEY_BYTES];
const ROOM_ID: &str = "room_repair_skip_entry";
const MLS_GROUP_ID: &str = "mls_repair_skip_entry";
const HOSTED_DEVICE_ID: &str = "hosted_repair_device";
const INCIDENT_ALIAS: &str = "incident-repair-synthetic";

fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn test_config(
    account_secret_bytes: [u8; NOSTR_SECRET_KEY_BYTES],
    device_id: &str,
) -> FiniteChatDeviceConfig {
    let now = now_unix_seconds();
    FiniteChatDeviceConfig {
        account_secret_key: NostrSecretKey::from_bytes(account_secret_bytes).unwrap(),
        device_id: device_id.to_string(),
        now_unix_seconds: now,
        credential_not_before_unix_seconds: now - 60,
        credential_not_after_unix_seconds: now + 60,
    }
}

fn sqlite_client_store(
    path: impl AsRef<std::path::Path>,
    config: &FiniteChatDeviceConfig,
) -> SqliteClientStore {
    SqliteClientStore::open(
        path,
        SqliteClientStoreOptions::from_nostr_secret(&config.account_secret_key, &config.device_id)
            .unwrap(),
    )
    .unwrap()
}

fn spawn_live_http_server(path: &std::path::Path) -> String {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let app = http_router(HttpServerState::from_sqlite_path(path).unwrap());
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    let server_url = format!("http://{addr}");
    let client = reqwest::blocking::Client::new();
    for _ in 0..100 {
        if client
            .get(format!("{server_url}/health"))
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            return server_url;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("live HTTP server did not become healthy");
}

fn hex_lower(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(TABLE[(byte >> 4) as usize] as char);
        out.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

type TestDelivery = HttpRuntimeDelivery<ReqwestHttpRuntimeTransport>;

fn test_delivery(server_url: &str) -> TestDelivery {
    HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.to_owned()))
}

/// Bootstrap the room, add the hosted device as a member, and persist the
/// hosted device's durable state at the join (add-commit) cursor. Mirrors
/// the rejected-entry classifier's fixture.
fn create_room_with_hosted_member(
    delivery: &mut TestDelivery,
    hosted_store: &mut SqliteClientStore,
    alice: &mut FiniteChatDevice,
    hosted: &mut FiniteChatDevice,
) -> u64 {
    alice.create_group_state(ROOM_ID, MLS_GROUP_ID).unwrap();
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: ROOM_ID.to_string(),
            mls_group_id: MLS_GROUP_ID.to_string(),
            creator: alice.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();
    delivery
        .upload_key_package(
            hosted
                .upload_key_package_request("kp_hosted_repair")
                .unwrap(),
        )
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(hosted.device_ref())
        .unwrap()
        .expect("hosted key package");
    let prepared = alice
        .prepare_add_member_commit(
            ROOM_ID,
            &claimed_key_package,
            "welcome_hosted_repair",
            "add-hosted",
        )
        .unwrap();
    let accepted = delivery.submit_commit(prepared.request).unwrap();
    let alice_page = delivery
        .sync_events(ROOM_ID, alice.device_ref(), 0)
        .unwrap();
    alice
        .merge_pending_commit_from_log(ROOM_ID, &alice_page.entries, &prepared.message_id)
        .unwrap();
    let claimed_welcomes = delivery.claim_welcomes(hosted.device_ref()).unwrap();
    let welcome = claimed_welcomes
        .into_iter()
        .find(|welcome| welcome.welcome_id == "welcome_hosted_repair")
        .unwrap();
    hosted_store
        .activate_welcome_and_save(
            hosted,
            "welcome_hosted_repair",
            ROOM_ID,
            &welcome.welcome_payload,
            &welcome.ratchet_tree_payload,
            accepted.seq,
        )
        .unwrap();
    delivery.ack_welcome("welcome_hosted_repair").unwrap();
    accepted.seq
}

fn append_alice_message(
    delivery: &mut TestDelivery,
    alice: &mut FiniteChatDevice,
    plaintext: &[u8],
    idempotency_key: &str,
) -> u64 {
    let request = alice
        .create_application_request(ROOM_ID, plaintext, idempotency_key)
        .unwrap();
    delivery
        .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap()
        .seq
}

/// Alice adds charlie; returns the commit sequence. The hosted device never
/// processes this commit.
fn append_add_charlie_commit(
    delivery: &mut TestDelivery,
    alice: &mut FiniteChatDevice,
    charlie: &FiniteChatDevice,
    after_seq: u64,
) -> u64 {
    delivery
        .upload_key_package(
            charlie
                .upload_key_package_request("kp_charlie_repair")
                .unwrap(),
        )
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(charlie.device_ref())
        .unwrap()
        .expect("charlie key package");
    let prepared = alice
        .prepare_add_member_commit(
            ROOM_ID,
            &claimed_key_package,
            "welcome_charlie_repair",
            "add-charlie",
        )
        .unwrap();
    let commit_seq = delivery.submit_commit(prepared.request).unwrap().seq;
    assert!(commit_seq > after_seq);
    let alice_page = delivery
        .sync_events(ROOM_ID, alice.device_ref(), after_seq)
        .unwrap();
    alice
        .merge_pending_commit_from_log(ROOM_ID, &alice_page.entries, &prepared.message_id)
        .unwrap();
    commit_seq
}

/// Read-only capture of the room's ordered log from the fixture server.
fn capture_room_log(
    delivery: &mut TestDelivery,
    requester: &FiniteChatDevice,
) -> Vec<RoomLogEntry> {
    let mut entries: Vec<RoomLogEntry> = Vec::new();
    let mut after_seq = 0;
    loop {
        let page = delivery
            .sync_events(ROOM_ID, requester.device_ref(), after_seq)
            .unwrap();
        after_seq = page.next_after_seq;
        let has_more = page.has_more;
        entries.extend(page.entries);
        if !has_more {
            break;
        }
    }
    entries
}

/// Relabel a Commit entry as an Application entry: proven by the
/// classifier's tests to classify as kind=application,
/// error_class=mls_application_ciphertext.
fn relabel_commit_as_application(entry: &mut RoomLogEntry) {
    entry.kind = LogEntryKind::Application;
    entry.envelope.kind = LogEntryKind::Application;
    entry.message_id = entry.envelope.message_id().unwrap();
}

/// Corrupt an entry's payload into bytes that no longer deserialize:
/// error_class=protocol_envelope_parsing on the entry's own kind.
fn corrupt_payload(entry: &mut RoomLogEntry) {
    entry.envelope.payload = vec![0xff; 32];
    entry.message_id = entry.envelope.message_id().unwrap();
}

struct RepairFixture {
    server_url: String,
    hosted_config: FiniteChatDeviceConfig,
    hosted_secret_hex: String,
    hosted_store_path: std::path::PathBuf,
    capture_path: std::path::PathBuf,
    join_seq: u64,
    head_seq: u64,
    poison_seqs: Vec<u64>,
    poison_sha256: Option<String>,
}

enum FixtureShape {
    /// join, one valid message, then `count` poison application entries at
    /// the tail (a relabeled add-member commit plus clones of it).
    TailPoison { count: usize },
    /// join, one valid message, one corrupted Commit entry at the tail.
    PoisonCommit,
    /// join, one valid message, one corrupted Application entry.
    WrongErrorClass,
    /// join, two valid messages; the replay advances without any skip.
    Healthy,
    /// join, one valid message the hosted device applied, `poison` owner
    /// application entries it cannot decrypt, then the hosted device's own
    /// rekey Commit, already merged into its store while its cursor stayed
    /// frozen below the poison (the pre-fix rekey lever's store shape, or
    /// an older cursor restored from backup).
    MergedOwnCommit { poison: usize },
}

fn build_fixture(dir: &std::path::Path, shape: FixtureShape) -> RepairFixture {
    let server_url = spawn_live_http_server(&dir.join("fixture-server.sqlite3"));
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_repair_operator");
    let hosted_config = test_config(HOSTED_ACCOUNT_SECRET_BYTES, HOSTED_DEVICE_ID);
    let charlie_config = test_config(CHARLIE_ACCOUNT_SECRET_BYTES, "charlie_repair_device");
    let mut alice = FiniteChatDevice::new(alice_config).unwrap();
    let mut hosted = FiniteChatDevice::new(hosted_config.clone()).unwrap();
    let charlie = FiniteChatDevice::new(charlie_config).unwrap();
    let mut delivery = test_delivery(&server_url);

    let hosted_store_path = dir.join("hosted-store.sqlite3");
    let mut hosted_store = sqlite_client_store(&hosted_store_path, &hosted_config);
    let join_seq =
        create_room_with_hosted_member(&mut delivery, &mut hosted_store, &mut alice, &mut hosted);

    let mut entries;
    let mut poison_seqs = Vec::new();
    let mut poison_sha256 = None;
    match shape {
        FixtureShape::MergedOwnCommit { poison } => {
            let applied_seq = append_alice_message(
                &mut delivery,
                &mut alice,
                b"valid message the hosted device applied",
                "repair-merged-own-commit-valid",
            );
            assert_eq!(applied_seq, join_seq + 1);
            run_runtime_sync_tick(
                &mut hosted_store,
                &mut hosted,
                &mut delivery,
                &RuntimeSyncOptions {
                    key_package_target_available: 0,
                    max_sync_pages_per_room: 8,
                },
            )
            .unwrap();
            assert_eq!(hosted.last_applied_seq(ROOM_ID).unwrap(), applied_seq);
            // The owner's next sends are undecryptable for the hosted
            // device once it has moved to the next epoch (its previous
            // epoch secret tree is gone), which is the same class the
            // production rewound-sender poison rejects with.
            for index in 0..poison {
                poison_seqs.push(append_alice_message(
                    &mut delivery,
                    &mut alice,
                    format!("poison {index}").as_bytes(),
                    &format!("repair-merged-own-commit-poison-{index}"),
                ));
            }
            // The hosted device rekeys with the primitives the rekey lever
            // is built from, merging its own Commit while the cursor stays
            // frozen at `applied_seq`.
            let prepared = hosted
                .prepare_self_update_commit(ROOM_ID, "repair-merged-own-commit-rekey")
                .unwrap();
            hosted_store.save_device_state(&hosted).unwrap();
            let accepted = delivery.submit_commit(prepared.request).unwrap();
            let page = delivery
                .sync_events(ROOM_ID, hosted.device_ref(), accepted.seq - 1)
                .unwrap();
            hosted
                .merge_pending_commit_from_log(ROOM_ID, &page.entries, &prepared.message_id)
                .unwrap();
            hosted_store.save_device_state(&hosted).unwrap();
            assert_eq!(hosted.group_epoch(ROOM_ID).unwrap(), 2);
            assert_eq!(hosted.last_applied_seq(ROOM_ID).unwrap(), applied_seq);
            assert_eq!(accepted.seq, applied_seq + poison as u64 + 1);
            entries = capture_room_log(&mut delivery, &alice);
        }
        FixtureShape::TailPoison { count } => {
            let valid_seq = append_alice_message(
                &mut delivery,
                &mut alice,
                b"valid message before the poison tail",
                "repair-valid-before",
            );
            assert_eq!(valid_seq, join_seq + 1);
            let commit_seq =
                append_add_charlie_commit(&mut delivery, &mut alice, &charlie, join_seq);
            assert_eq!(commit_seq, join_seq + 2);
            entries = capture_room_log(&mut delivery, &alice);
            // Poison the tail: the commit relabeled as application, plus
            // clones of it at later sequences (identical bytes classify
            // identically).
            let poison = entries
                .iter_mut()
                .find(|entry| entry.seq == commit_seq)
                .expect("capture contains the commit");
            relabel_commit_as_application(poison);
            poison_sha256 = Some(sha256_hex(&poison.envelope.payload));
            let poison_template = poison.clone();
            poison_seqs.push(commit_seq);
            for index in 1..count {
                let mut clone = poison_template.clone();
                clone.seq = commit_seq + index as u64;
                poison_seqs.push(clone.seq);
                entries.push(clone);
            }
        }
        FixtureShape::PoisonCommit => {
            append_alice_message(
                &mut delivery,
                &mut alice,
                b"valid message before the commit",
                "repair-valid-before-commit",
            );
            let commit_seq =
                append_add_charlie_commit(&mut delivery, &mut alice, &charlie, join_seq);
            entries = capture_room_log(&mut delivery, &alice);
            let commit = entries
                .iter_mut()
                .find(|entry| entry.seq == commit_seq)
                .expect("capture contains the commit");
            corrupt_payload(commit);
            poison_seqs.push(commit_seq);
        }
        FixtureShape::WrongErrorClass => {
            append_alice_message(
                &mut delivery,
                &mut alice,
                b"valid message before the corrupted entry",
                "repair-valid-before-corrupted",
            );
            let corrupted_seq = append_alice_message(
                &mut delivery,
                &mut alice,
                b"plaintext never replayed",
                "repair-corrupted-entry",
            );
            entries = capture_room_log(&mut delivery, &alice);
            let corrupted = entries
                .iter_mut()
                .find(|entry| entry.seq == corrupted_seq)
                .expect("capture contains the entry");
            corrupt_payload(corrupted);
            poison_seqs.push(corrupted_seq);
        }
        FixtureShape::Healthy => {
            append_alice_message(
                &mut delivery,
                &mut alice,
                b"first healthy message",
                "repair-healthy-a",
            );
            append_alice_message(
                &mut delivery,
                &mut alice,
                b"second healthy message",
                "repair-healthy-b",
            );
            entries = capture_room_log(&mut delivery, &alice);
        }
    }
    drop(hosted_store);
    let head_seq = entries.iter().map(|entry| entry.seq).max().unwrap();

    let capture_path = dir.join("room-log-capture.json");
    let capture = CapturedRoomLogFile {
        target_room_id: ROOM_ID.to_owned(),
        rooms: vec![
            finitechat_client::rejected_entry_diagnostic::CapturedRoomLog {
                room_id: ROOM_ID.to_owned(),
                entries,
            },
        ],
    };
    std::fs::write(&capture_path, serde_json::to_vec_pretty(&capture).unwrap()).unwrap();

    RepairFixture {
        server_url,
        hosted_config,
        hosted_secret_hex: hex_lower(&HOSTED_ACCOUNT_SECRET_BYTES),
        hosted_store_path,
        capture_path,
        join_seq,
        head_seq,
        poison_seqs,
        poison_sha256,
    }
}

fn run_repair(
    fixture: &RepairFixture,
    work_dir: &std::path::Path,
    audit_path: &std::path::Path,
    extra_args: &[&str],
) -> Result<String, String> {
    let mut args = vec![
        "repair".to_owned(),
        "skip-entry".to_owned(),
        "--store".to_owned(),
        fixture.hosted_store_path.display().to_string(),
        "--work-dir".to_owned(),
        work_dir.display().to_string(),
        "--room-log".to_owned(),
        fixture.capture_path.display().to_string(),
        "--device-id".to_owned(),
        HOSTED_DEVICE_ID.to_owned(),
        "--account-secret-hex".to_owned(),
        fixture.hosted_secret_hex.clone(),
        "--incident-alias".to_owned(),
        INCIDENT_ALIAS.to_owned(),
        "--audit-log".to_owned(),
        audit_path.display().to_string(),
    ];
    args.extend(extra_args.iter().map(|arg| arg.to_string()));
    let mut output = Vec::new();
    finitechat_cli::run(args, &mut output)
        .map(|()| String::from_utf8(output).expect("utf8 output"))
        .map_err(|error| error.to_string())
}

fn read_audit_lines(audit_path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(audit_path)
        .expect("audit log exists")
        .lines()
        .map(|line| serde_json::from_str(line).expect("audit line is JSON"))
        .collect()
}

fn durable_cursor(fixture: &RepairFixture) -> u64 {
    let store = sqlite_client_store(&fixture.hosted_store_path, &fixture.hosted_config);
    let device = store.load_device(fixture.hosted_config.clone()).unwrap();
    device.last_applied_seq(ROOM_ID).unwrap()
}

#[test]
fn skips_poison_application_entries_and_advances_the_real_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = build_fixture(dir.path(), FixtureShape::TailPoison { count: 2 });
    assert_eq!(durable_cursor(&fixture), fixture.join_seq);
    let audit_path = dir.path().join("audit.jsonl");

    let stdout =
        run_repair(&fixture, &dir.path().join("work"), &audit_path, &[]).expect("repair runs");
    let record: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(record["schema_version"], 1);
    assert_eq!(record["incident_alias"], INCIDENT_ALIAS);
    assert_eq!(record["repair_disposition"], "applied");
    assert_eq!(record["rehearsal_outcome"], "advanced");
    assert_eq!(record["cursor_before"], fixture.join_seq);
    assert_eq!(
        record["cursor_after"], fixture.head_seq,
        "the durable cursor reaches the capture head"
    );
    assert_eq!(record["max_skips"], 16);
    let skipped = record["skipped"].as_array().expect("skipped list");
    assert_eq!(
        skipped
            .iter()
            .map(|entry| entry["seq"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        fixture.poison_seqs,
        "exactly the derived poison sequences, no operator input"
    );
    for entry in skipped {
        assert_eq!(entry["kind"], "application");
        assert_eq!(
            entry["sha256"],
            fixture.poison_sha256.clone().unwrap(),
            "the entry binding digest"
        );
    }
    assert!(record.get("refusal_reason").is_none());

    // Privacy lock: no identifiers, plaintext, or secrets leave stdout.
    let hosted_account_id = hex_lower(
        fixture
            .hosted_config
            .account_secret_key
            .public_key()
            .as_bytes(),
    );
    for forbidden in [
        ROOM_ID.to_owned(),
        MLS_GROUP_ID.to_owned(),
        hosted_account_id,
        HOSTED_DEVICE_ID.to_owned(),
        fixture.hosted_secret_hex.clone(),
        "valid message before the poison tail".to_owned(),
    ] {
        assert!(
            !stdout.contains(&forbidden),
            "repair output must not contain {forbidden:?}"
        );
    }

    // Durable audit: one line per skipped entry plus one apply summary.
    let audit = read_audit_lines(&audit_path);
    assert_eq!(audit.len(), 3);
    for (line, seq) in audit[..2].iter().zip(fixture.poison_seqs.iter()) {
        assert_eq!(line["schema_version"], 1);
        assert_eq!(line["incident_alias"], INCIDENT_ALIAS);
        assert_eq!(line["seq"], *seq);
        assert_eq!(line["kind"], "application");
        assert_eq!(line["entry_sha256"], fixture.poison_sha256.clone().unwrap());
        assert_eq!(line["error_class"], "mls_application_ciphertext");
        assert!(line["skipped_at_unix_seconds"].as_u64().unwrap() > 0);
    }
    let summary = &audit[2];
    assert_eq!(summary["phase"], "apply");
    assert_eq!(summary["cursor_before"], fixture.join_seq);
    assert_eq!(summary["cursor_after"], fixture.head_seq);
    assert_eq!(summary["skips"], 2);

    // The audit log is created private.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&audit_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    // The REAL store's durable cursor advanced past both poison entries.
    assert_eq!(durable_cursor(&fixture), fixture.head_seq);

    // Independent proof: the classifier now reports the repaired store as
    // healthy (nothing left to replay).
    let mut output = Vec::new();
    finitechat_cli::run(
        [
            "diagnose".to_owned(),
            "rejected-entry".to_owned(),
            "--store".to_owned(),
            fixture.hosted_store_path.display().to_string(),
            "--work-dir".to_owned(),
            dir.path().join("verify-work").display().to_string(),
            "--room-log".to_owned(),
            fixture.capture_path.display().to_string(),
            "--device-id".to_owned(),
            HOSTED_DEVICE_ID.to_owned(),
            "--account-secret-hex".to_owned(),
            fixture.hosted_secret_hex.clone(),
            "--incident-alias".to_owned(),
            INCIDENT_ALIAS.to_owned(),
        ],
        &mut output,
    )
    .expect("diagnose runs against the repaired store");
    let diagnostic: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(diagnostic["cursor_before"], fixture.head_seq);
    assert_eq!(diagnostic["replay_outcome"], "unchanged");
}

#[test]
fn refuses_a_non_application_rejected_entry_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = build_fixture(dir.path(), FixtureShape::PoisonCommit);
    let audit_path = dir.path().join("audit.jsonl");

    let stdout = run_repair(&fixture, &dir.path().join("work"), &audit_path, &[])
        .expect("repair runs and refuses");
    let record: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(record["repair_disposition"], "refused");
    assert_eq!(record["rehearsal_outcome"], "rejected");
    assert_eq!(record["refusal_reason"], "rejected_entry_not_skippable");
    assert_eq!(record["refused_entry"]["seq"], fixture.poison_seqs[0]);
    assert_eq!(record["refused_entry"]["kind"], "commit");
    assert_eq!(
        record["refused_entry"]["error_class"],
        "protocol_envelope_parsing"
    );
    assert_eq!(record["skipped"].as_array().unwrap().len(), 0);

    // Fail closed: the real store was never written.
    assert_eq!(durable_cursor(&fixture), fixture.join_seq);

    // The audit trail carries only the refusal summary.
    let audit = read_audit_lines(&audit_path);
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["phase"], "refused");
    assert_eq!(audit[0]["cursor_before"], fixture.join_seq);
    assert_eq!(audit[0]["cursor_after"], fixture.join_seq);
    assert_eq!(audit[0]["skips"], 0);
}

#[test]
fn refuses_a_non_ciphertext_error_class_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = build_fixture(dir.path(), FixtureShape::WrongErrorClass);
    let audit_path = dir.path().join("audit.jsonl");

    let stdout = run_repair(&fixture, &dir.path().join("work"), &audit_path, &[])
        .expect("repair runs and refuses");
    let record: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(record["repair_disposition"], "refused");
    assert_eq!(record["refusal_reason"], "rejected_entry_not_skippable");
    assert_eq!(record["refused_entry"]["seq"], fixture.poison_seqs[0]);
    assert_eq!(record["refused_entry"]["kind"], "application");
    assert_eq!(
        record["refused_entry"]["error_class"], "protocol_envelope_parsing",
        "an application entry that fails for any other class is not skippable"
    );

    assert_eq!(durable_cursor(&fixture), fixture.join_seq);
    let audit = read_audit_lines(&audit_path);
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["phase"], "refused");
}

#[test]
fn refuses_when_the_derived_skip_count_would_exceed_max_skips() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = build_fixture(dir.path(), FixtureShape::TailPoison { count: 3 });
    let audit_path = dir.path().join("audit.jsonl");

    let stdout = run_repair(
        &fixture,
        &dir.path().join("work"),
        &audit_path,
        &["--max-skips", "2"],
    )
    .expect("repair runs and refuses");
    let record: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(record["repair_disposition"], "refused");
    assert_eq!(record["refusal_reason"], "max_skips_exceeded");
    assert_eq!(record["max_skips"], 2);
    assert_eq!(record["skipped"].as_array().unwrap().len(), 2);
    assert_eq!(
        record["refused_entry"]["seq"], fixture.poison_seqs[2],
        "the third poison entry would have exceeded the bound"
    );

    // Fail closed: no skip is ever applied partially.
    assert_eq!(durable_cursor(&fixture), fixture.join_seq);
    let audit = read_audit_lines(&audit_path);
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["phase"], "refused");
    assert_eq!(audit[0]["skips"], 0);
}

#[test]
fn rejects_a_max_skips_value_above_the_hard_cap() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = build_fixture(dir.path(), FixtureShape::Healthy);
    let audit_path = dir.path().join("audit.jsonl");

    let error = run_repair(
        &fixture,
        &dir.path().join("work"),
        &audit_path,
        &["--max-skips", "65"],
    )
    .expect_err("--max-skips above the hard cap is a usage error");
    assert!(error.contains("--max-skips"), "unexpected error: {error}");
    assert!(!audit_path.exists());
}

#[test]
fn healthy_capture_applies_zero_skips_without_touching_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = build_fixture(dir.path(), FixtureShape::Healthy);
    let audit_path = dir.path().join("audit.jsonl");

    let stdout =
        run_repair(&fixture, &dir.path().join("work"), &audit_path, &[]).expect("repair runs");
    let record: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(record["repair_disposition"], "applied");
    assert_eq!(record["rehearsal_outcome"], "advanced");
    assert_eq!(record["skipped"].as_array().unwrap().len(), 0);
    assert_eq!(record["cursor_before"], fixture.join_seq);
    assert_eq!(record["cursor_after"], fixture.join_seq);

    // Zero derived skips means already healthy: the store is not written
    // and the device converges on its own next sync.
    assert_eq!(durable_cursor(&fixture), fixture.join_seq);
    let audit = read_audit_lines(&audit_path);
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["phase"], "apply");
    assert_eq!(audit[0]["skips"], 0);
}

/// A capture whose tail is the hosted device's own already-merged rekey
/// Commit (the store shape the pre-fix rekey lever left behind): the
/// rehearsal replay derives exactly the owner poison entries below the
/// Commit, the Commit itself is a no-op advance rather than a refusal, and
/// the durable cursor lands on the Commit seq.
#[test]
fn skips_owner_poison_below_an_already_merged_own_commit() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = build_fixture(dir.path(), FixtureShape::MergedOwnCommit { poison: 3 });
    let frozen_cursor = fixture.join_seq + 1;
    assert_eq!(durable_cursor(&fixture), frozen_cursor);
    assert_eq!(fixture.poison_seqs.len(), 3);
    assert_eq!(
        fixture.head_seq,
        frozen_cursor + 4,
        "the own Commit is the capture head"
    );
    let audit_path = dir.path().join("audit.jsonl");

    let stdout =
        run_repair(&fixture, &dir.path().join("work"), &audit_path, &[]).expect("repair runs");
    let record: Value = serde_json::from_str(&stdout).unwrap();

    // The rehearsal replay reached the capture head: the merged own Commit
    // was a no-op advance, not a refusal. Phase 2 writes only the derived
    // skips, so the durable cursor lands on the last poison entry and the
    // device crosses its own Commit on its next sync.
    assert_eq!(record["repair_disposition"], "applied");
    assert_eq!(record["rehearsal_outcome"], "advanced");
    assert_eq!(record["cursor_before"], frozen_cursor);
    assert_eq!(record["cursor_after"], fixture.head_seq - 1);
    assert!(record.get("refusal_reason").is_none());
    let skipped = record["skipped"].as_array().expect("skipped list");
    assert_eq!(
        skipped
            .iter()
            .map(|entry| entry["seq"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        fixture.poison_seqs,
        "only the owner poison entries below the Commit are skipped"
    );
    for entry in skipped {
        assert_eq!(entry["kind"], "application");
        assert_eq!(entry["error_class"], "mls_application_ciphertext");
    }

    let audit = read_audit_lines(&audit_path);
    assert_eq!(audit.len(), 4);
    let summary = &audit[3];
    assert_eq!(summary["phase"], "apply");
    assert_eq!(summary["cursor_before"], frozen_cursor);
    assert_eq!(summary["cursor_after"], fixture.head_seq - 1);
    assert_eq!(summary["skips"], 3);
    assert_eq!(durable_cursor(&fixture), fixture.head_seq - 1);

    // The restarted device's next sync crosses the merged own Commit
    // without error and lands on the capture head.
    let mut store = sqlite_client_store(&fixture.hosted_store_path, &fixture.hosted_config);
    let mut device = store.load_device(fixture.hosted_config.clone()).unwrap();
    let mut delivery = test_delivery(&fixture.server_url);
    let report = run_runtime_sync_tick(
        &mut store,
        &mut device,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 0,
            max_sync_pages_per_room: 8,
        },
    )
    .expect("the next sync crosses the merged own Commit");
    assert!(report.applied_entries.is_empty());
    assert_eq!(device.last_applied_seq(ROOM_ID).unwrap(), fixture.head_seq);
    assert_eq!(device.group_epoch(ROOM_ID).unwrap(), 2);
    assert!(!device.has_pending_commit(ROOM_ID).unwrap());
    drop(store);
    assert_eq!(durable_cursor(&fixture), fixture.head_seq);
}

#[test]
fn rejects_an_audit_log_inside_the_work_dir() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store.sqlite3");
    std::fs::write(&store, b"not a real store").unwrap();
    let work_dir = dir.path().join("work");
    let audit_path = work_dir.join("audit.jsonl");

    let mut output = Vec::new();
    let result = finitechat_cli::run(
        [
            "repair".to_owned(),
            "skip-entry".to_owned(),
            "--store".to_owned(),
            store.display().to_string(),
            "--work-dir".to_owned(),
            work_dir.display().to_string(),
            "--room-log".to_owned(),
            dir.path().join("capture.json").display().to_string(),
            "--device-id".to_owned(),
            HOSTED_DEVICE_ID.to_owned(),
            "--account-secret-hex".to_owned(),
            hex_lower(&HOSTED_ACCOUNT_SECRET_BYTES),
            "--incident-alias".to_owned(),
            INCIDENT_ALIAS.to_owned(),
            "--audit-log".to_owned(),
            audit_path.display().to_string(),
        ],
        &mut output,
    );
    let error = result.expect_err("an audit log inside the work dir is rejected");
    assert!(
        error.to_string().contains("--audit-log"),
        "unexpected error: {error}"
    );
    assert!(!audit_path.exists());
}
