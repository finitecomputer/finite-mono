//! Admin demotion through `BrainStore::remove_admin`: the role change and
//! every Folder Key rotation it requires commit together, or nothing does.

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

fn folder_id(id: &str) -> FolderId {
    FolderId::new(id).unwrap()
}

/// `acme` where `npub-demoted` is an admin holding the v1 key of:
/// `team-notes` (All-Members), `private-project` (restricted, with an explicit
/// access row granted before the promotion), `strategy` (restricted, one live
/// object) and `admin-only`. Only `strategy` and `admin-only` are reached by
/// the role alone.
fn admin_store() -> BrainStore {
    let mut store = store_with_strategy_folder();
    let brain_id = acme();
    let demoted = user(DEMOTED);
    store.add_member(&brain_id, &demoted).unwrap();
    store
        .grant_folder_access(
            &brain_id,
            &folder_id("private-project"),
            &demoted,
            &grant(
                "private-project-demoted-v1",
                "private-project",
                1,
                ADMIN,
                DEMOTED,
            ),
        )
        .unwrap();
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
    for (id, folder) in [
        ("team-notes-demoted-v1", "team-notes"),
        ("strategy-demoted-v1", "strategy"),
    ] {
        store
            .grant_folder_access(
                &brain_id,
                &folder_id(folder),
                &demoted,
                &grant(id, folder, 1, ADMIN, DEMOTED),
            )
            .unwrap();
    }
    store
        .submit_sync_record(
            &brain_id,
            &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
        )
        .unwrap();
    store
}

fn rotation(folder: &str, new_key_version: u32) -> MemberFolderRotation {
    rotation_issued_by(folder, new_key_version, ADMIN)
}

/// One rotation re-wrapping `folder` for `npub-admin` only, issued by `issuer`.
fn rotation_issued_by(folder: &str, new_key_version: u32, issuer: &str) -> MemberFolderRotation {
    MemberFolderRotation {
        folder_id: folder_id(folder),
        new_key_version,
        grants: vec![grant(
            &format!("{folder}-admin-v{new_key_version}"),
            folder,
            new_key_version,
            issuer,
            ADMIN,
        )],
        reencrypted_records: if folder == "strategy" {
            vec![revision_record_struct(
                &format!("event-rotate-strategy-v{new_key_version}"),
                "strategy",
                "obj_000000000001",
                2,
                Some(1),
                "rotated",
            )]
        } else {
            Vec::new()
        },
    }
}

fn demote<'a>(
    store: &mut BrainStore,
    rotations: &'a [MemberFolderRotation],
    control_records: &'a [SyncRecordInput],
) -> Result<Vec<FolderKeyRotationOutcome>, StoreError> {
    store.remove_admin(
        &acme(),
        AdminDemotionRequest {
            removed_user_id: &user(DEMOTED),
            rotations,
            updated_at: NOW,
            control_records,
        },
    )
}

fn current_recipients(store: &BrainStore, folder: &str) -> BTreeSet<UserId> {
    let stored = store.load_brain(&acme()).unwrap();
    let version = stored
        .brain
        .folders
        .iter()
        .find(|candidate| candidate.id.as_str() == folder)
        .unwrap()
        .current_key_version;
    stored
        .grants
        .iter()
        .filter(|grant| grant.folder_id.as_str() == folder && grant.key_version == version)
        .map(|grant| grant.recipient_npub.clone())
        .collect()
}

fn key_versions(store: &BrainStore) -> BTreeMap<String, u32> {
    store
        .load_brain(&acme())
        .unwrap()
        .brain
        .folders
        .iter()
        .map(|folder| (folder.id.to_string(), folder.current_key_version))
        .collect()
}

#[test]
fn demotion_rotates_exactly_the_folders_reached_only_through_the_role() {
    let mut store = admin_store();
    let stored = store.load_brain(&acme()).unwrap();
    assert_eq!(
        admin_demotion_rotation_folders(&stored, &user(DEMOTED)).unwrap(),
        BTreeSet::from([folder_id("admin-only"), folder_id("strategy")])
    );
    let before_sequence = store.latest_sequence(&acme()).unwrap();
    let control_records = [
        brain_admin_control_record("demote-access-change", ADMIN),
        brain_admin_control_record("rotate-strategy-access-change", ADMIN),
    ];

    let outcomes = demote(
        &mut store,
        &[rotation("strategy", 2), rotation("admin-only", 2)],
        &control_records,
    )
    .unwrap();

    assert_eq!(
        outcomes
            .iter()
            .map(|outcome| (
                outcome.folder_id.to_string(),
                outcome.previous_key_version,
                outcome.new_key_version,
                outcome.recipients.clone(),
                outcome.cut_off_holders.clone(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "strategy".to_owned(),
                1,
                2,
                BTreeSet::from([user(ADMIN)]),
                BTreeSet::from([user(DEMOTED)])
            ),
            (
                "admin-only".to_owned(),
                1,
                2,
                BTreeSet::from([user(ADMIN)]),
                BTreeSet::from([user(DEMOTED)])
            ),
        ]
    );
    let stored = store.load_brain(&acme()).unwrap();
    assert!(!stored.brain.admins.contains(&user(DEMOTED)));
    assert!(
        stored
            .brain
            .members
            .iter()
            .any(|member| member.user_id == user(DEMOTED))
    );
    assert_eq!(
        key_versions(&store),
        BTreeMap::from([
            ("admin-only".to_owned(), 2),
            ("private-project".to_owned(), 1),
            ("strategy".to_owned(), 2),
            ("team-notes".to_owned(), 1),
        ])
    );
    for folder in &stored.brain.folders {
        assert!(
            stale_folder_key_holders(&stored, folder)
                .unwrap()
                .is_empty(),
            "{} still has a stale current wrap",
            folder.id
        );
    }
    assert!(!current_recipients(&store, "strategy").contains(&user(DEMOTED)));
    assert!(!current_recipients(&store, "admin-only").contains(&user(DEMOTED)));
    // Explicit access and All-Members membership keep the current key.
    assert!(current_recipients(&store, "private-project").contains(&user(DEMOTED)));
    assert!(current_recipients(&store, "team-notes").contains(&user(DEMOTED)));
    assert!(
        stored
            .folder_access
            .get(&folder_id("private-project"))
            .unwrap()
            .contains(&user(DEMOTED))
    );
    let strategy_object = store
        .load_current_objects(&acme())
        .unwrap()
        .into_iter()
        .find(|object| object.folder_id.as_str() == "strategy")
        .unwrap();
    assert_eq!(strategy_object.revision, 2);
    // Control records plus the one re-encrypted revision.
    assert_eq!(store.latest_sequence(&acme()).unwrap(), before_sequence + 3);
}

#[test]
fn demotion_refuses_any_rotation_set_but_the_required_one_and_changes_nothing() {
    let mut store = admin_store();
    let before = store.load_brain(&acme()).unwrap();
    let before_sequence = store.latest_sequence(&acme()).unwrap();
    let required = vec!["admin-only".to_owned(), "strategy".to_owned()];
    for (rotations, supplied) in [
        (Vec::new(), Vec::new()),
        (vec![rotation("strategy", 2)], vec!["strategy"]),
        (
            vec![
                rotation("strategy", 2),
                rotation("admin-only", 2),
                rotation("team-notes", 2),
            ],
            vec!["strategy", "admin-only", "team-notes"],
        ),
        (
            vec![
                rotation("strategy", 2),
                rotation("admin-only", 2),
                rotation("strategy", 2),
            ],
            vec!["strategy", "admin-only", "strategy"],
        ),
    ] {
        assert_eq!(
            demote(&mut store, &rotations, &[]).unwrap_err(),
            StoreError::FolderRotationSetMismatch {
                operation: "admin demotion",
                required_folder_ids: required.clone(),
                supplied_folder_ids: supplied.into_iter().map(ToOwned::to_owned).collect(),
            }
        );
        let after = store.load_brain(&acme()).unwrap();
        assert_eq!(after.brain, before.brain);
        assert_eq!(after.grants, before.grants);
        assert_eq!(store.latest_sequence(&acme()).unwrap(), before_sequence);
    }
}

#[test]
fn demotion_refuses_a_moved_key_version_or_a_wrap_for_the_demoted_key() {
    let mut store = admin_store();
    let before = store.load_brain(&acme()).unwrap();
    let error = demote(
        &mut store,
        &[rotation("strategy", 3), rotation("admin-only", 2)],
        &[],
    )
    .unwrap_err();
    assert_eq!(
        error,
        StoreError::StaleFolderKeyVersion {
            folder_id: "strategy".to_owned(),
            current_key_version: 1,
            requested_key_version: 3,
        }
    );
    let mut still_wraps_demoted = rotation("admin-only", 2);
    still_wraps_demoted.grants.push(grant(
        "admin-only-demoted-v2",
        "admin-only",
        2,
        ADMIN,
        DEMOTED,
    ));
    assert_eq!(
        demote(
            &mut store,
            &[rotation("strategy", 2), still_wraps_demoted],
            &[],
        )
        .unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "grant recipients must exactly match required recipients".to_owned()
        }
    );
    let after = store.load_brain(&acme()).unwrap();
    assert_eq!(after.brain, before.brain);
    assert_eq!(after.grants, before.grants);
}

#[test]
fn concurrent_rotation_makes_a_prepared_demotion_fail_closed() {
    let mut store = admin_store();
    store
        .rotate_folder_key(
            &acme(),
            FolderKeyRotationTarget::RequiredRecipients,
            FolderKeyRotationRequest {
                folder_id: &folder_id("admin-only"),
                new_key_version: 2,
                grants: &[
                    grant("admin-only-admin-v2", "admin-only", 2, ADMIN, ADMIN),
                    grant("admin-only-demoted-v2", "admin-only", 2, ADMIN, DEMOTED),
                ],
                reencrypted_records: &[],
                updated_at: NOW,
                control_records: &[],
            },
        )
        .unwrap();
    let error = demote(
        &mut store,
        &[rotation("strategy", 2), rotation("admin-only", 2)],
        &[],
    )
    .unwrap_err();
    assert!(
        matches!(error, StoreError::StaleFolderKeyVersion { ref folder_id, .. } if folder_id == "admin-only"),
        "{error:?}"
    );
    assert!(
        store
            .load_brain(&acme())
            .unwrap()
            .brain
            .admins
            .contains(&user(DEMOTED))
    );
}

#[test]
fn demotion_with_nothing_to_rotate_only_removes_the_role() {
    let mut store = org_store_with_access_test_folders();
    let brain_id = acme();
    let demoted = user(DEMOTED);
    store.add_member(&brain_id, &demoted).unwrap();
    store
        .grant_folder_access(
            &brain_id,
            &folder_id("private-project"),
            &demoted,
            &grant(
                "private-project-demoted-v1",
                "private-project",
                1,
                ADMIN,
                DEMOTED,
            ),
        )
        .unwrap();
    store.add_admin(&brain_id, &demoted).unwrap();
    let before_versions = key_versions(&store);

    let outcomes = demote(
        &mut store,
        &[],
        &[brain_admin_control_record("demote-access-change", ADMIN)],
    )
    .unwrap();

    assert!(outcomes.is_empty());
    assert!(
        !store
            .load_brain(&brain_id)
            .unwrap()
            .brain
            .admins
            .contains(&demoted)
    );
    assert_eq!(key_versions(&store), before_versions);
}

#[test]
fn demotion_keeps_the_last_admin_and_refuses_personal_brains() {
    let mut store = org_store_with_access_test_folders();
    assert_eq!(
        store
            .remove_admin(
                &acme(),
                AdminDemotionRequest {
                    removed_user_id: &user(ADMIN),
                    rotations: &[],
                    updated_at: NOW,
                    control_records: &[],
                },
            )
            .unwrap_err(),
        StoreError::BrokenInvariant {
            reason: "organization brain must keep at least one admin".to_owned()
        }
    );

    let mut store = BrainStore::open_in_memory().unwrap();
    bootstrap_personal_named(&mut store, "personal", "npub-owner", "npub-agent", NOW);
    let brain_id = BrainId::new("personal").unwrap();
    let before = store.load_brain(&brain_id).unwrap();
    for removed in ["npub-owner", "npub-agent"] {
        assert_eq!(
            store
                .remove_admin(
                    &brain_id,
                    AdminDemotionRequest {
                        removed_user_id: &user(removed),
                        rotations: &[],
                        updated_at: NOW,
                        control_records: &[],
                    },
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "member/admin mutation requires an organization brain".to_owned()
            }
        );
    }
    let after = store.load_brain(&brain_id).unwrap();
    assert_eq!(after.brain, before.brain);
    assert_eq!(after.grants, before.grants);
}

#[test]
fn an_admin_may_demote_itself_and_issues_the_rotations_as_an_admin() {
    let mut store = admin_store();
    // The demoted key signs every new grant: it is an admin when it acts.
    let outcomes = demote(
        &mut store,
        &[
            rotation_issued_by("strategy", 2, DEMOTED),
            rotation_issued_by("admin-only", 2, DEMOTED),
        ],
        &[brain_admin_control_record("self-demotion", DEMOTED)],
    )
    .unwrap();

    assert_eq!(outcomes.len(), 2);
    for outcome in &outcomes {
        assert_eq!(outcome.recipients, BTreeSet::from([user(ADMIN)]));
        assert_eq!(outcome.cut_off_holders, BTreeSet::from([user(DEMOTED)]));
    }
    let stored = store.load_brain(&acme()).unwrap();
    assert_eq!(stored.brain.admins, vec![user(ADMIN)]);
    let strategy_v2 = stored
        .grants
        .iter()
        .find(|grant| grant.folder_id.as_str() == "strategy" && grant.key_version == 2)
        .unwrap();
    assert_eq!(strategy_v2.issuer_npub, user(DEMOTED));
}

#[test]
fn rotation_grants_from_a_key_that_is_not_an_admin_are_refused() {
    let mut store = admin_store();
    let member = user("npub-member");
    store.add_member(&acme(), &member).unwrap();
    let before = store.load_brain(&acme()).unwrap();
    let error = demote(
        &mut store,
        &[
            rotation_issued_by("strategy", 2, member.as_str()),
            rotation("admin-only", 2),
        ],
        &[],
    )
    .unwrap_err();
    assert_eq!(
        error,
        StoreError::BrokenInvariant {
            reason: "organization folder grants must be issued by a brain admin".to_owned()
        }
    );
    let after = store.load_brain(&acme()).unwrap();
    assert_eq!(after.brain, before.brain);
    assert_eq!(after.grants, before.grants);
}

#[test]
fn demotion_validates_against_access_committed_by_another_connection() {
    let scratch = tempfile::TempDir::new().unwrap();
    let database = scratch.path().join("demotion-race.sqlite3");
    let mut store = BrainStore::open(&database).unwrap();
    bootstrap_org_and_strategy_folder(&mut store);
    let demoted = user(DEMOTED);
    store.add_member(&acme(), &demoted).unwrap();
    store.add_admin(&acme(), &demoted).unwrap();
    let rotations = [rotation("private-project", 2), rotation("strategy", 2)];

    // Prepared while the role alone reaches both restricted Folders. Another
    // connection grants explicit `strategy` access first, so only
    // `private-project` may rotate once the demotion reads under the lock.
    let (result, store) = super::folder_key_rotation_tests::interleave(
        &database,
        store,
        "INSERT INTO folder_access (brain_id, folder_id, user_id) VALUES ('acme', 'strategy', 'npub-demoted')",
        move |store| demote(store, &rotations, &[]),
    );
    assert_eq!(
        result.unwrap_err(),
        StoreError::FolderRotationSetMismatch {
            operation: "admin demotion",
            required_folder_ids: vec!["private-project".to_owned()],
            supplied_folder_ids: vec!["private-project".to_owned(), "strategy".to_owned()],
        }
    );
    let stored = store.load_brain(&acme()).unwrap();
    assert!(stored.brain.admins.contains(&demoted));
    assert!(
        stored
            .brain
            .folders
            .iter()
            .all(|folder| folder.current_key_version == 1)
    );
}
