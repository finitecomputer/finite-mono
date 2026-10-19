//! Folder Key rotation through `BrainStore::rotate_folder_key`: standalone
//! rotation, revocation of stale current-wrap holders, and the refusals.

use super::*;

const ADMIN: &str = "npub-admin";
const DEMOTED: &str = "npub-demoted";
const NOW: &str = "2026-06-23T00:00:00.000Z";

fn acme() -> BrainId {
    BrainId::new("acme").unwrap()
}

fn user(npub: &str) -> UserId {
    UserId::new(npub).unwrap()
}

/// `acme` with a restricted `strategy` Folder holding one live object.
/// `npub-demoted` received the current wrap as an admin and was then demoted,
/// so it holds the key without entitlement: the production `revocationIncomplete`.
fn demoted_admin_store() -> BrainStore {
    let mut store = store_with_strategy_folder();
    admin_with_current_wrap(&mut store);
    store.remove_admin(&acme(), &user(DEMOTED)).unwrap();
    store
}

/// Make `npub-demoted` an admin holding the current `strategy` wrap by role
/// (no explicit row) and add one live object to `strategy`.
fn admin_with_current_wrap(store: &mut BrainStore) {
    let brain_id = acme();
    let folder_id = FolderId::new("strategy").unwrap();
    let demoted = user(DEMOTED);
    store.add_member(&brain_id, &demoted).unwrap();
    store.add_admin(&brain_id, &demoted).unwrap();
    store
        .grant_folder_access(
            &brain_id,
            &folder_id,
            &demoted,
            &grant("strategy-demoted-v1", "strategy", 1, ADMIN, DEMOTED),
        )
        .unwrap();
    store
        .submit_sync_record(
            &brain_id,
            &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
        )
        .unwrap();
}

/// Hold the database write lock on a second connection, apply `change`,
/// start `work` on the store in another thread, then commit the change after
/// the worker has had time to begin. Returns the worker's result, the store
/// and how long the worker took.
fn interleave<T: Send + 'static>(
    database: &std::path::Path,
    store: BrainStore,
    change: &str,
    work: impl FnOnce(&mut BrainStore) -> T + Send + 'static,
) -> (T, BrainStore, std::time::Duration) {
    let mut other = rusqlite::Connection::open(database).unwrap();
    other.pragma_update(None, "foreign_keys", true).unwrap();
    let tx = other
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    tx.execute_batch(change).unwrap();
    let started = std::time::Instant::now();
    let worker = std::thread::spawn(move || {
        let mut store = store;
        let result = work(&mut store);
        (result, store)
    });
    std::thread::sleep(std::time::Duration::from_millis(400));
    tx.commit().unwrap();
    let (result, store) = worker.join().unwrap();
    (result, store, started.elapsed())
}

fn folder(store: &BrainStore, brain_id: &BrainId, folder_id: &str) -> (StoredBrain, Folder) {
    let stored = store.load_brain(brain_id).unwrap();
    let folder = stored
        .brain
        .folders
        .iter()
        .find(|folder| folder.id.as_str() == folder_id)
        .cloned()
        .unwrap();
    (stored, folder)
}

fn reencrypted_strategy_object(event_id: &str, key_version: u32) -> FolderObjectRevisionSyncRecord {
    revision_record_struct(
        event_id,
        "strategy",
        "obj_000000000001",
        u64::from(key_version),
        Some(u64::from(key_version) - 1),
        "reencrypted",
    )
}

fn rotate_strategy(
    store: &mut BrainStore,
    target: FolderKeyRotationTarget<'_>,
    new_key_version: u32,
    grants: &[FolderKeyGrantMetadata],
    records: &[FolderObjectRevisionSyncRecord],
) -> Result<FolderKeyRotationOutcome, StoreError> {
    let folder_id = FolderId::new("strategy").unwrap();
    store.rotate_folder_key(
        &acme(),
        target,
        FolderKeyRotationRequest {
            folder_id: &folder_id,
            new_key_version,
            grants,
            reencrypted_records: records,
            updated_at: NOW,
            control_records: &[],
        },
    )
}

fn current_grant_recipients(store: &BrainStore, folder_id: &str) -> BTreeSet<UserId> {
    let (stored, folder) = folder(store, &acme(), folder_id);
    stored
        .grants
        .iter()
        .filter(|grant| {
            grant.folder_id == folder.id && grant.key_version == folder.current_key_version
        })
        .map(|grant| grant.recipient_npub.clone())
        .collect()
}

#[test]
fn demoted_admin_is_a_stale_holder_and_not_a_required_recipient() {
    let store = demoted_admin_store();
    let (stored, strategy) = folder(&store, &acme(), "strategy");
    assert_eq!(
        stale_folder_key_holders(&stored, &strategy).unwrap(),
        BTreeSet::from([user(DEMOTED)])
    );
    assert_eq!(
        folder_key_required_recipients(&stored, &strategy).unwrap(),
        BTreeSet::from([user(ADMIN)])
    );
    // The all_members Folder still entitles the demoted Member.
    let (_, team_notes) = folder(&store, &acme(), "team-notes");
    assert!(
        stale_folder_key_holders(&stored, &team_notes)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn standalone_rotation_rewraps_required_recipients_and_cuts_off_stale_holders() {
    let mut store = demoted_admin_store();
    let sequence_before = store.latest_sequence(&acme()).unwrap();
    let control_grant = grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN);
    let folder_id = FolderId::new("strategy").unwrap();
    let outcome = store
        .rotate_folder_key(
            &acme(),
            FolderKeyRotationTarget::RequiredRecipients,
            FolderKeyRotationRequest {
                folder_id: &folder_id,
                new_key_version: 2,
                grants: std::slice::from_ref(&control_grant),
                reencrypted_records: &[reencrypted_strategy_object("event-reencrypt-2", 2)],
                updated_at: NOW,
                control_records: &[
                    folder_key_grant_control_record(&control_grant, "rotation-grant-record"),
                    folder_access_control_record(
                        "rotation-access-change",
                        SyncRecordType::BrainAdminAccessChange,
                        "strategy",
                        ADMIN,
                    ),
                ],
            },
        )
        .unwrap();

    assert_eq!(
        outcome,
        FolderKeyRotationOutcome {
            folder_id: folder_id.clone(),
            previous_key_version: 1,
            new_key_version: 2,
            recipients: BTreeSet::from([user(ADMIN)]),
            cut_off_holders: BTreeSet::from([user(DEMOTED)]),
        }
    );
    let (stored, strategy) = folder(&store, &acme(), "strategy");
    assert_eq!(strategy.current_key_version, 2);
    assert!(
        stored
            .folder_access
            .get(&folder_id)
            .is_none_or(BTreeSet::is_empty)
    );
    assert!(
        stale_folder_key_holders(&stored, &strategy)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        current_grant_recipients(&store, "strategy"),
        BTreeSet::from([user(ADMIN)])
    );
    // One re-encrypted object revision plus the two signed control records.
    assert_eq!(store.latest_sequence(&acme()).unwrap(), sequence_before + 3);
    let bootstrap = store.sync_bootstrap(&acme()).unwrap();
    let object = bootstrap
        .objects
        .iter()
        .find(|object| object.object_id.as_str() == "obj_000000000001")
        .unwrap();
    assert_eq!(object.revision, 2);
    assert_eq!(object.payload_json, "{\"body\":\"reencrypted\"}");
}

#[test]
fn standalone_rotation_requires_exactly_the_required_recipients() {
    let mut store = demoted_admin_store();
    let sequence_before = store.latest_sequence(&acme()).unwrap();
    let record = [reencrypted_strategy_object("event-reencrypt-2", 2)];
    let target = FolderKeyRotationTarget::RequiredRecipients;

    assert_eq!(
        rotate_strategy(&mut store, target, 2, &[], &record).unwrap_err(),
        StoreError::MissingRequiredGrant {
            recipient_user_id: ADMIN.to_owned()
        }
    );
    // Re-wrapping for the stale holder would hand it the new key.
    assert_eq!(
        rotate_strategy(
            &mut store,
            target,
            2,
            &[
                grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN),
                grant("strategy-demoted-v2", "strategy", 2, ADMIN, DEMOTED),
            ],
            &record,
        )
        .unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "grant recipients must exactly match required recipients".to_owned()
        }
    );
    assert_eq!(
        rotate_strategy(
            &mut store,
            target,
            2,
            &[grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN)],
            &[],
        )
        .unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "folder key rotation must re-encrypt every live object exactly once".to_owned()
        }
    );
    // A demoted key cannot issue the new wrap.
    assert_eq!(
        rotate_strategy(
            &mut store,
            target,
            2,
            &[grant("strategy-admin-v2", "strategy", 2, DEMOTED, ADMIN)],
            &record,
        )
        .unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "organization folder grants must be issued by a brain admin".to_owned()
        }
    );

    let (_, strategy) = folder(&store, &acme(), "strategy");
    assert_eq!(strategy.current_key_version, 1);
    assert_eq!(store.latest_sequence(&acme()).unwrap(), sequence_before);
    assert_eq!(
        current_grant_recipients(&store, "strategy"),
        BTreeSet::from([user(ADMIN), user(DEMOTED)])
    );
}

#[test]
fn standalone_rotation_enforces_fanout_before_reading_state() {
    let mut store = demoted_admin_store();
    let grants = (0..=MAX_FOLDER_ACCESS_REMOVAL_GRANTS)
        .map(|index| grant(&format!("fanout-{index}"), "strategy", 2, ADMIN, ADMIN))
        .collect::<Vec<_>>();
    let error = rotate_strategy(
        &mut store,
        FolderKeyRotationTarget::RequiredRecipients,
        2,
        &grants,
        &[],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        StoreError::Core(CoreError::RotationFanoutLimitExceeded {
            operation: "Folder Key rotation",
            ..
        })
    ));
}

#[test]
fn concurrent_rotations_leave_exactly_one_winner() {
    let mut store = demoted_admin_store();
    let target = FolderKeyRotationTarget::RequiredRecipients;
    rotate_strategy(
        &mut store,
        target,
        2,
        &[grant("winner-admin-v2", "strategy", 2, ADMIN, ADMIN)],
        &[reencrypted_strategy_object("winner-reencrypt", 2)],
    )
    .unwrap();
    let sequence_after_winner = store.latest_sequence(&acme()).unwrap();

    // The loser prepared against version 1 as well.
    assert_eq!(
        rotate_strategy(
            &mut store,
            target,
            2,
            &[grant("loser-admin-v2", "strategy", 2, ADMIN, ADMIN)],
            &[reencrypted_strategy_object("loser-reencrypt", 2)],
        )
        .unwrap_err(),
        StoreError::StaleFolderKeyVersion {
            folder_id: "strategy".to_owned(),
            current_key_version: 2,
            requested_key_version: 2,
        }
    );
    assert_eq!(
        rotate_strategy(
            &mut store,
            FolderKeyRotationTarget::RemoveAccess(&user(DEMOTED)),
            4,
            &[grant("skip-admin-v4", "strategy", 4, ADMIN, ADMIN)],
            &[],
        )
        .unwrap_err(),
        StoreError::StaleFolderKeyVersion {
            folder_id: "strategy".to_owned(),
            current_key_version: 2,
            requested_key_version: 4,
        }
    );
    assert_eq!(
        store.latest_sequence(&acme()).unwrap(),
        sequence_after_winner
    );

    // The version bump itself is a compare-and-swap inside the transaction,
    // so a loser that passed validation before the winner committed still
    // fails without relying on the server's store lock.
    let tx = store.conn.transaction().unwrap();
    assert_eq!(
        crate::folder_key_rotation::apply_folder_key_rotation(
            &tx,
            &acme(),
            &FolderId::new("strategy").unwrap(),
            1,
            2,
            &[],
            &[],
            NOW,
        )
        .unwrap_err(),
        StoreError::StaleFolderKeyVersion {
            folder_id: "strategy".to_owned(),
            current_key_version: 2,
            requested_key_version: 2,
        }
    );
}

#[test]
fn revoking_a_stale_holder_without_an_access_row_rotates_the_folder() {
    let mut store = demoted_admin_store();
    let outcome = rotate_strategy(
        &mut store,
        FolderKeyRotationTarget::RemoveAccess(&user(DEMOTED)),
        2,
        &[grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN)],
        &[reencrypted_strategy_object("event-reencrypt-2", 2)],
    )
    .unwrap();
    assert_eq!(outcome.cut_off_holders, BTreeSet::from([user(DEMOTED)]));
    assert_eq!(outcome.recipients, BTreeSet::from([user(ADMIN)]));
    let (stored, strategy) = folder(&store, &acme(), "strategy");
    assert_eq!(strategy.current_key_version, 2);
    assert!(
        stale_folder_key_holders(&stored, &strategy)
            .unwrap()
            .is_empty()
    );
    // The demoted key keeps Membership and its all_members Folder.
    assert!(
        stored
            .brain
            .members
            .iter()
            .any(|member| member.user_id.as_str() == DEMOTED)
    );

    // Once cut off, the same key no longer has anything to revoke.
    assert_eq!(
        rotate_strategy(
            &mut store,
            FolderKeyRotationTarget::RemoveAccess(&user(DEMOTED)),
            3,
            &[grant("strategy-admin-v3", "strategy", 3, ADMIN, ADMIN)],
            &[reencrypted_strategy_object("event-reencrypt-3", 3)],
        )
        .unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "folder access target does not currently have access".to_owned()
        }
    );
}

#[test]
fn revoking_an_entitled_admin_or_an_unknown_key_is_refused() {
    let mut store = demoted_admin_store();
    let sequence_before = store.latest_sequence(&acme()).unwrap();
    let grants = [grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN)];
    let records = [reencrypted_strategy_object("event-reencrypt-2", 2)];
    assert_eq!(
        rotate_strategy(
            &mut store,
            FolderKeyRotationTarget::RemoveAccess(&user(ADMIN)),
            2,
            &grants,
            &records,
        )
        .unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "folder access target holds this Folder through its Brain role, not Folder access; change the role (for example revoke admin) instead of revoking Folder access".to_owned()
        }
    );
    assert_eq!(
        rotate_strategy(
            &mut store,
            FolderKeyRotationTarget::RemoveAccess(&user("npub-stranger")),
            2,
            &grants,
            &records,
        )
        .unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "folder access target does not currently have access".to_owned()
        }
    );
    let (_, strategy) = folder(&store, &acme(), "strategy");
    assert_eq!(strategy.current_key_version, 1);
    assert_eq!(store.latest_sequence(&acme()).unwrap(), sequence_before);
}

#[test]
fn rotation_drops_pending_wrap_markers_below_the_new_version() {
    let mut store = demoted_admin_store();
    store
        .conn
        .execute(
            "INSERT INTO brain_pending_grant_wraps (brain_id, folder_id, recipient_npub, key_version, reason, created_at) VALUES ('acme', 'strategy', 'npub-departed', 1, 'invitation', ?1)",
            params![NOW],
        )
        .unwrap();
    rotate_strategy(
        &mut store,
        FolderKeyRotationTarget::RequiredRecipients,
        2,
        &[grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN)],
        &[reencrypted_strategy_object("event-reencrypt-2", 2)],
    )
    .unwrap();
    assert!(
        store
            .pending_grant_wraps(&acme())
            .unwrap()
            .iter()
            .all(|wrap| wrap.folder_id.as_str() != "strategy")
    );
    // A leftover v1 marker would make every later completion fail as drifted.
    assert_eq!(
        store
            .complete_pending_grant_wraps(
                &acme(),
                &FolderId::new("strategy").unwrap(),
                &[],
                &[],
                &[],
            )
            .unwrap(),
        0
    );
}

#[test]
fn admin_only_folder_rotation_cuts_off_a_demoted_admin() {
    let mut store = empty_org_store();
    let brain_id = acme();
    let demoted = user(DEMOTED);
    store.add_member(&brain_id, &demoted).unwrap();
    store.add_admin(&brain_id, &demoted).unwrap();
    store
        .create_folder(
            &brain_id,
            &admin_only_folder(),
            &BTreeSet::new(),
            &[
                grant("admin-only-admin-v1", "admin-only", 1, ADMIN, ADMIN),
                grant("admin-only-demoted-v1", "admin-only", 1, ADMIN, DEMOTED),
            ],
        )
        .unwrap();
    store.remove_admin(&brain_id, &demoted).unwrap();
    let (stored, admin_only) = folder(&store, &brain_id, "admin-only");
    assert_eq!(
        stale_folder_key_holders(&stored, &admin_only).unwrap(),
        BTreeSet::from([demoted.clone()])
    );

    let folder_id = admin_only.id.clone();
    let outcome = store
        .rotate_folder_key(
            &brain_id,
            FolderKeyRotationTarget::RequiredRecipients,
            FolderKeyRotationRequest {
                folder_id: &folder_id,
                new_key_version: 2,
                grants: &[grant("admin-only-admin-v2", "admin-only", 2, ADMIN, ADMIN)],
                reencrypted_records: &[],
                updated_at: NOW,
                control_records: &[],
            },
        )
        .unwrap();
    assert_eq!(outcome.cut_off_holders, BTreeSet::from([demoted]));
    assert_eq!(
        current_grant_recipients(&store, "admin-only"),
        BTreeSet::from([user(ADMIN)])
    );
}

#[test]
fn personal_brain_rotation_keeps_the_owner_and_personal_agent() {
    let mut store = BrainStore::open_in_memory().unwrap();
    bootstrap_personal_named(&mut store, "personal-a", "npub-owner", "npub-agent", NOW);
    let brain_id = BrainId::new("personal-a").unwrap();
    let personal_folder = Folder {
        id: FolderId::new("journal").unwrap(),
        name: DisplayName::new("folder_name", "Journal").unwrap(),
        role: FolderRole::Folder,
        access: FolderAccessMode::Owner,
        parent_folder_id: None,
        path: SafeRelativePath::new("folder_path", "Journal").unwrap(),
        current_key_version: 1,
    };
    store
        .create_folder(
            &brain_id,
            &personal_folder,
            &BTreeSet::new(),
            &[
                grant("journal-owner-v1", "journal", 1, "npub-owner", "npub-owner"),
                grant("journal-agent-v1", "journal", 1, "npub-owner", "npub-agent"),
            ],
        )
        .unwrap();
    let next = personal_folder.current_key_version + 1;
    let folder_name = personal_folder.id.as_str();
    let rotate = |store: &mut BrainStore, grants: &[FolderKeyGrantMetadata]| {
        store.rotate_folder_key(
            &brain_id,
            FolderKeyRotationTarget::RequiredRecipients,
            FolderKeyRotationRequest {
                folder_id: &personal_folder.id,
                new_key_version: next,
                grants,
                reencrypted_records: &[],
                updated_at: NOW,
                control_records: &[],
            },
        )
    };

    assert_eq!(
        rotate(
            &mut store,
            &[grant(
                "owner-only",
                folder_name,
                next,
                "npub-owner",
                "npub-owner"
            )]
        )
        .unwrap_err(),
        StoreError::MissingRequiredGrant {
            recipient_user_id: "npub-agent".to_owned()
        }
    );
    let outcome = rotate(
        &mut store,
        &[
            grant("owner-next", folder_name, next, "npub-owner", "npub-owner"),
            grant("agent-next", folder_name, next, "npub-owner", "npub-agent"),
        ],
    )
    .unwrap();
    assert_eq!(
        outcome.recipients,
        BTreeSet::from([user("npub-owner"), user("npub-agent")])
    );
    assert!(outcome.cut_off_holders.is_empty());
}

#[test]
fn rotation_rewraps_for_an_active_mount_participant_and_revoke_refuses_it() {
    let mut store = store_with_strategy_folder();
    bootstrap_org_named(&mut store, "dest", "Destination", "npub-dest-admin");
    let source_admin = user(ADMIN);
    let dest_admin = user("npub-dest-admin");
    store
        .create_shared_folder_invitation(
            &acme(),
            &FolderId::new("strategy").unwrap(),
            &BrainId::new("dest").unwrap(),
            "mount-offer",
            &dest_admin,
            &source_admin,
            "/v1/mount-offers/mount-offer/accept",
            &grant("grant-mount-v1", "strategy", 1, ADMIN, dest_admin.as_str()),
            "2026-06-30T00:00:00.000Z",
            NOW,
        )
        .unwrap();
    accept_mount_for_test(
        &mut store,
        "mount-offer",
        &dest_admin,
        "mount-1",
        "projection-1",
        &[],
        NOW,
    )
    .unwrap();

    // The Mount participant is entitled, so leaving it out strands it.
    assert_eq!(
        rotate_strategy(
            &mut store,
            FolderKeyRotationTarget::RequiredRecipients,
            2,
            &[grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN)],
            &[],
        )
        .unwrap_err(),
        StoreError::MissingRequiredGrant {
            recipient_user_id: dest_admin.to_string()
        }
    );
    assert_eq!(
        rotate_strategy(
            &mut store,
            FolderKeyRotationTarget::RemoveAccess(&dest_admin),
            2,
            &[grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN)],
            &[],
        )
        .unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "Folder access target still receives this Folder through an active Mount; remove the target from that Mount before rotating explicit Folder access".to_owned()
        }
    );

    let outcome = rotate_strategy(
        &mut store,
        FolderKeyRotationTarget::RequiredRecipients,
        2,
        &[
            grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN),
            grant(
                "strategy-mount-v2",
                "strategy",
                2,
                ADMIN,
                dest_admin.as_str(),
            ),
        ],
        &[],
    )
    .unwrap();
    assert_eq!(
        outcome.recipients,
        BTreeSet::from([source_admin, dest_admin.clone()])
    );
    assert!(outcome.cut_off_holders.is_empty());
    let connection = store.load_shared_folder_connection("mount-1").unwrap();
    assert_eq!(connection.status, SharedFolderConnectionStatus::Active);
    assert!(current_grant_recipients(&store, "strategy").contains(&dest_admin));
}

#[test]
fn revoking_an_explicit_row_of_a_key_promoted_to_admin_is_refused() {
    let mut store = store_with_strategy_folder();
    let brain_id = acme();
    let promoted = user("npub-promoted");
    store.add_member(&brain_id, &promoted).unwrap();
    store
        .grant_folder_access(
            &brain_id,
            &FolderId::new("strategy").unwrap(),
            &promoted,
            &grant(
                "strategy-promoted-v1",
                "strategy",
                1,
                ADMIN,
                "npub-promoted",
            ),
        )
        .unwrap();
    store.add_admin(&brain_id, &promoted).unwrap();
    let sequence_before = store.latest_sequence(&brain_id).unwrap();

    // Removing the row would leave the admin role, so the rotation would hand
    // the "revoked" key the new version.
    assert_eq!(
        rotate_strategy(
            &mut store,
            FolderKeyRotationTarget::RemoveAccess(&promoted),
            2,
            &[
                grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN),
                grant("strategy-promoted-v2", "strategy", 2, ADMIN, "npub-promoted"),
            ],
            &[],
        )
        .unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "folder access target holds this Folder through its Brain role, not Folder access; change the role (for example revoke admin) instead of revoking Folder access".to_owned()
        }
    );
    let (stored, strategy) = folder(&store, &brain_id, "strategy");
    assert_eq!(strategy.current_key_version, 1);
    assert!(stored.folder_access[&strategy.id].contains(&promoted));
    assert_eq!(store.latest_sequence(&brain_id).unwrap(), sequence_before);
}

#[test]
fn rotation_validates_against_a_roster_committed_by_another_connection() {
    let scratch = TempDir::new().unwrap();
    let database = scratch.path().join("rotation-race.sqlite3");
    let mut store = BrainStore::open(&database).unwrap();
    bootstrap_org_and_strategy_folder(&mut store);
    admin_with_current_wrap(&mut store);

    // Prepared while npub-demoted is still an admin, so its grant set wraps
    // the new key for it. Another connection demotes it before this commits.
    let (result, mut store, elapsed) = interleave(
        &database,
        store,
        "DELETE FROM brain_admins WHERE brain_id = 'acme' AND user_id = 'npub-demoted'",
        |store| {
            rotate_strategy(
                store,
                FolderKeyRotationTarget::RequiredRecipients,
                2,
                &[
                    grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN),
                    grant("strategy-demoted-v2", "strategy", 2, ADMIN, DEMOTED),
                ],
                &[reencrypted_strategy_object("event-reencrypt-2", 2)],
            )
        },
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(400),
        "the rotation must wait for the other writer: {elapsed:?}"
    );
    assert_eq!(
        result.unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "grant recipients must exactly match required recipients".to_owned()
        }
    );
    let (stored, strategy) = folder(&store, &acme(), "strategy");
    assert_eq!(strategy.current_key_version, 1);
    assert!(!stored.brain.admins.contains(&user(DEMOTED)));
    assert!(!stored.grants.iter().any(|grant| grant.key_version == 2));

    // Prepared again against the committed roster, it cuts the key off.
    let outcome = rotate_strategy(
        &mut store,
        FolderKeyRotationTarget::RequiredRecipients,
        2,
        &[grant("strategy-admin-v2", "strategy", 2, ADMIN, ADMIN)],
        &[reencrypted_strategy_object("event-reencrypt-2", 2)],
    )
    .unwrap();
    assert_eq!(outcome.cut_off_holders, BTreeSet::from([user(DEMOTED)]));
}

#[test]
fn member_removal_validates_against_access_committed_by_another_connection() {
    let scratch = TempDir::new().unwrap();
    let database = scratch.path().join("member-removal-race.sqlite3");
    let mut store = BrainStore::open(&database).unwrap();
    bootstrap_org_and_strategy_folder(&mut store);
    let member = user("npub-member");
    store.add_member(&acme(), &member).unwrap();

    // Prepared when the Member reads only the all_members Folder. Another
    // connection grants it the restricted Folder before this commits, so the
    // removal must now rotate that Folder too.
    let (result, store, elapsed) = interleave(
        &database,
        store,
        "INSERT INTO folder_access (brain_id, folder_id, user_id) VALUES ('acme', 'strategy', 'npub-member')",
        move |store| {
            store.remove_member_with_rotations(
                &acme(),
                &user(ADMIN),
                &user("npub-member"),
                &[MemberFolderRotation {
                    folder_id: FolderId::new("team-notes").unwrap(),
                    new_key_version: 2,
                    grants: vec![grant("team-notes-admin-v2", "team-notes", 2, ADMIN, ADMIN)],
                    reencrypted_records: Vec::new(),
                }],
                &[],
                NOW,
            )
        },
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(400),
        "the removal must wait for the other writer: {elapsed:?}"
    );
    assert_eq!(
        result.unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "member removal requires exactly one rotation for every readable Folder"
                .to_owned()
        }
    );
    let stored = store.load_brain(&acme()).unwrap();
    assert!(
        stored
            .brain
            .members
            .iter()
            .any(|entry| entry.user_id == member)
    );
    assert!(stored.folder_access[&FolderId::new("strategy").unwrap()].contains(&member));
}
