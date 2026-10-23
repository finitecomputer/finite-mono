//! Member removal rotates every Folder whose current key the Member holds:
//! the Folders it reads and the Folders where it keeps a stale current wrap
//! (`revocationIncomplete`, left by an earlier admin demotion).

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

/// `acme` with the all_members `team-notes` Folder and a restricted
/// `strategy` Folder holding one live object. `npub-demoted` received the
/// strategy wrap as an admin and was then demoted, so it is a Member that
/// still holds that key without entitlement (`revocationIncomplete`).
fn demoted_admin_store() -> BrainStore {
    let mut store = store_with_strategy_folder();
    let demoted = user(DEMOTED);
    store.add_member(&acme(), &demoted).unwrap();
    store.add_admin(&acme(), &demoted).unwrap();
    store
        .grant_folder_access(
            &acme(),
            &FolderId::new("strategy").unwrap(),
            &demoted,
            &grant("removal-strategy-demoted-v1", "strategy", 1, ADMIN, DEMOTED),
        )
        .unwrap();
    store
        .submit_sync_record(
            &acme(),
            &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
        )
        .unwrap();
    // Legacy demotion: the role row goes, every wrap stays.
    store
        .conn
        .execute(
            "DELETE FROM brain_admins WHERE brain_id = 'acme' AND user_id = ?1",
            [DEMOTED],
        )
        .unwrap();
    store
}

fn folder(store: &BrainStore, folder_id: &str) -> Folder {
    store
        .load_brain(&acme())
        .unwrap()
        .brain
        .folders
        .into_iter()
        .find(|folder| folder.id.as_str() == folder_id)
        .unwrap()
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

fn current_grant_recipients(store: &BrainStore, folder_id: &str) -> BTreeSet<UserId> {
    let current = folder(store, folder_id);
    store
        .load_brain(&acme())
        .unwrap()
        .grants
        .into_iter()
        .filter(|grant| {
            grant.folder_id == current.id && grant.key_version == current.current_key_version
        })
        .map(|grant| grant.recipient_npub)
        .collect()
}

/// One rotation per planned Folder, wrapped for exactly the post-removal
/// required recipients, re-encrypting the strategy object where it lives.
fn rotations_for(
    store: &BrainStore,
    planned: &[MemberRemovalFolderRotation],
) -> Vec<MemberFolderRotation> {
    let mut post_removal = store.load_brain(&acme()).unwrap();
    post_removal
        .brain
        .members
        .retain(|member| member.user_id != user(DEMOTED));
    planned
        .iter()
        .map(|planned| {
            let current = folder(store, planned.folder_id.as_str());
            let new_key_version = current.current_key_version + 1;
            let grants = folder_key_required_recipients(&post_removal, &current)
                .unwrap()
                .into_iter()
                .map(|recipient| {
                    grant(
                        &format!("removal-{}-{}", planned.folder_id, recipient),
                        planned.folder_id.as_str(),
                        new_key_version,
                        ADMIN,
                        recipient.as_str(),
                    )
                })
                .collect();
            let reencrypted_records = if planned.folder_id.as_str() == "strategy" {
                vec![reencrypted_strategy_object(
                    "event-removal-reencrypt",
                    new_key_version,
                )]
            } else {
                Vec::new()
            };
            MemberFolderRotation {
                folder_id: planned.folder_id.clone(),
                new_key_version,
                grants,
                reencrypted_records,
            }
        })
        .collect()
}

fn plan(store: &BrainStore, removed: &str) -> Result<Vec<MemberRemovalFolderRotation>, StoreError> {
    member_removal_folder_rotations(&store.load_brain(&acme()).unwrap(), &user(removed))
}

fn remove_demoted(
    store: &mut BrainStore,
    rotations: &[MemberFolderRotation],
) -> Result<(), StoreError> {
    store.remove_member_with_rotations(&acme(), &user(ADMIN), &user(DEMOTED), rotations, &[], NOW)
}

fn key_versions(store: &BrainStore) -> BTreeMap<FolderId, u32> {
    store
        .load_brain(&acme())
        .unwrap()
        .brain
        .folders
        .into_iter()
        .map(|folder| (folder.id, folder.current_key_version))
        .collect()
}

#[test]
fn plan_lists_readable_folders_and_the_stale_grant_folder() {
    let store = demoted_admin_store();
    let planned = plan(&store, DEMOTED).unwrap();
    let reasons = planned
        .iter()
        .map(|rotation| (rotation.folder_id.as_str(), rotation.reason))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        reasons.get("team-notes"),
        Some(&MemberRemovalRotationReason::Readable)
    );
    assert_eq!(
        reasons.get("strategy"),
        Some(&MemberRemovalRotationReason::StaleCurrentGrant)
    );
    // A restricted Folder the demoted key never held is not touched.
    assert_eq!(reasons.get("private-project"), None);
    // Exactly the stale holders of the stored state carry the stale reason.
    let stored = store.load_brain(&acme()).unwrap();
    for rotation in &planned {
        let current = folder(&store, rotation.folder_id.as_str());
        assert_eq!(rotation.current_key_version, current.current_key_version);
        assert_eq!(
            rotation.reason == MemberRemovalRotationReason::StaleCurrentGrant,
            stale_folder_key_holders(&stored, &current)
                .unwrap()
                .contains(&user(DEMOTED)),
        );
    }
}

#[test]
fn removal_rotates_the_stale_grant_folder_and_cuts_off_the_removed_key() {
    let mut store = demoted_admin_store();
    let planned = plan(&store, DEMOTED).unwrap();
    let before = key_versions(&store);
    let rotations = rotations_for(&store, &planned);

    remove_demoted(&mut store, &rotations).unwrap();

    let stored = store.load_brain(&acme()).unwrap();
    assert!(
        !stored
            .brain
            .members
            .iter()
            .any(|member| member.user_id == user(DEMOTED))
    );
    let after = key_versions(&store);
    for rotation in &planned {
        assert_eq!(
            after[&rotation.folder_id],
            before[&rotation.folder_id] + 1,
            "{} was not rotated",
            rotation.folder_id
        );
        let recipients = current_grant_recipients(&store, rotation.folder_id.as_str());
        assert!(
            !recipients.contains(&user(DEMOTED)),
            "{} still wraps the current key for the removed Member",
            rotation.folder_id
        );
    }
    let strategy = folder(&store, "strategy");
    assert_eq!(strategy.current_key_version, 2);
    assert_eq!(
        current_grant_recipients(&store, "strategy"),
        BTreeSet::from([user(ADMIN)])
    );
    assert!(
        stale_folder_key_holders(&stored, &strategy)
            .unwrap()
            .is_empty()
    );
    // The removed key keeps only its old version-1 wrap of strategy, which
    // no longer opens anything: the object was re-encrypted at version 2.
    assert!(stored.grants.iter().any(|grant| {
        grant.folder_id == strategy.id
            && grant.recipient_npub == user(DEMOTED)
            && grant.key_version == 1
    }));
    let object = store
        .sync_bootstrap(&acme())
        .unwrap()
        .objects
        .into_iter()
        .find(|object| object.object_id.as_str() == "obj_000000000001")
        .unwrap();
    assert_eq!(object.revision, 2);
    // Untouched Folders keep their key.
    for (folder_id, version) in &before {
        if !planned
            .iter()
            .any(|rotation| rotation.folder_id == *folder_id)
        {
            assert_eq!(
                after[folder_id], *version,
                "{folder_id} rotated unexpectedly"
            );
        }
    }
}

#[test]
fn removal_that_omits_only_the_stale_grant_folder_is_refused_with_its_ids() {
    let mut store = demoted_admin_store();
    let planned = plan(&store, DEMOTED).unwrap();
    // What a client from before this rule sends: readable Folders only.
    let readable_only = rotations_for(&store, &planned)
        .into_iter()
        .filter(|rotation| rotation.folder_id.as_str() != "strategy")
        .collect::<Vec<_>>();
    let before = key_versions(&store);
    let sequence_before = store.latest_sequence(&acme()).unwrap();

    let error = remove_demoted(&mut store, &readable_only).unwrap_err();
    let StoreError::FolderRotationSetMismatch {
        operation,
        required_folder_ids,
        supplied_folder_ids,
    } = &error
    else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(*operation, "member removal");
    let mut expected_required = planned
        .iter()
        .map(|rotation| rotation.folder_id.to_string())
        .collect::<Vec<_>>();
    expected_required.sort();
    assert_eq!(required_folder_ids, &expected_required);
    assert!(!supplied_folder_ids.contains(&"strategy".to_owned()));
    // The server names exactly the skipped stale-grant Folder.
    assert_eq!(
        member_removal_skipped_stale_grant_folders(&planned, supplied_folder_ids),
        vec!["strategy".to_owned()]
    );

    // Nothing changed: still a Member, still holding the stale wrap.
    let stored = store.load_brain(&acme()).unwrap();
    assert!(
        stored
            .brain
            .members
            .iter()
            .any(|member| member.user_id == user(DEMOTED))
    );
    assert_eq!(key_versions(&store), before);
    assert_eq!(store.latest_sequence(&acme()).unwrap(), sequence_before);
    let strategy = folder(&store, "strategy");
    assert_eq!(
        stale_folder_key_holders(&stored, &strategy).unwrap(),
        BTreeSet::from([user(DEMOTED)])
    );
}

#[test]
fn removal_with_a_missing_readable_duplicate_or_extra_rotation_is_a_plain_set_mismatch() {
    let mut store = demoted_admin_store();
    let planned = plan(&store, DEMOTED).unwrap();
    let rotations = rotations_for(&store, &planned);
    let without_team_notes = rotations
        .iter()
        .filter(|rotation| rotation.folder_id.as_str() != "team-notes")
        .cloned()
        .collect::<Vec<_>>();
    let mut duplicated = rotations.clone();
    duplicated.push(rotations[0].clone());
    let mut with_extra = rotations.clone();
    with_extra.extend(rotations_for(
        &store,
        &[MemberRemovalFolderRotation {
            folder_id: FolderId::new("private-project").unwrap(),
            current_key_version: 1,
            reason: MemberRemovalRotationReason::Readable,
        }],
    ));
    for candidate in [without_team_notes, duplicated, with_extra] {
        let error = remove_demoted(&mut store, &candidate).unwrap_err();
        let StoreError::FolderRotationSetMismatch {
            operation,
            supplied_folder_ids,
            ..
        } = &error
        else {
            panic!("unexpected error: {error}");
        };
        assert_eq!(*operation, "member removal");
        // Not a stale-grant-only skip, so no Folder is singled out.
        assert!(
            member_removal_skipped_stale_grant_folders(&planned, supplied_folder_ids).is_empty()
        );
    }
}

#[test]
fn plan_refuses_an_admin_and_a_non_member_like_the_removal() {
    let store = demoted_admin_store();
    for (removed, reason) in [
        (ADMIN, "remove admin role before removing member"),
        ("npub-stranger", "brain member does not exist"),
    ] {
        assert_eq!(
            plan(&store, removed).unwrap_err(),
            StoreError::BrokenInvariant {
                reason: reason.to_owned()
            }
        );
    }
}

#[test]
fn plan_for_an_entitled_member_without_stale_wraps_lists_only_readable_folders() {
    let mut store = demoted_admin_store();
    store.add_member(&acme(), &user("npub-plain")).unwrap();
    let planned = plan(&store, "npub-plain").unwrap();
    assert!(
        planned
            .iter()
            .all(|rotation| rotation.reason == MemberRemovalRotationReason::Readable)
    );
    assert!(
        planned
            .iter()
            .any(|rotation| rotation.folder_id.as_str() == "team-notes")
    );
    assert!(
        !planned
            .iter()
            .any(|rotation| rotation.folder_id.as_str() == "strategy")
    );
}

#[test]
fn removal_after_another_admin_rotated_a_planned_folder_says_to_rerun() {
    let mut store = demoted_admin_store();
    let planned = plan(&store, DEMOTED).unwrap();
    let rotations = rotations_for(&store, &planned);
    // Between plan and removal another admin clears the stale wrap.
    let strategy_id = FolderId::new("strategy").unwrap();
    store
        .rotate_folder_key(
            &acme(),
            FolderKeyRotationTarget::RequiredRecipients,
            FolderKeyRotationRequest {
                folder_id: &strategy_id,
                new_key_version: 2,
                grants: &[grant(
                    "concurrent-strategy-admin-v2",
                    "strategy",
                    2,
                    ADMIN,
                    ADMIN,
                )],
                reencrypted_records: &[reencrypted_strategy_object("event-concurrent-2", 2)],
                updated_at: NOW,
                control_records: &[],
            },
        )
        .unwrap();
    let before = key_versions(&store);

    let error = remove_demoted(&mut store, &rotations).unwrap_err();
    assert!(
        matches!(
            &error,
            StoreError::FolderRotationSetMismatch {
                operation: "member removal",
                ..
            }
        ) && error.to_string().contains("Prepare the change again"),
        "{error}"
    );
    assert_eq!(key_versions(&store), before);
    assert!(
        store
            .load_brain(&acme())
            .unwrap()
            .brain
            .members
            .iter()
            .any(|member| member.user_id == user(DEMOTED))
    );
    // A fresh plan no longer lists the cleared Folder and the removal succeeds.
    let replanned = plan(&store, DEMOTED).unwrap();
    assert!(
        !replanned
            .iter()
            .any(|rotation| rotation.folder_id == strategy_id)
    );
    let fresh = rotations_for(&store, &replanned);
    remove_demoted(&mut store, &fresh).unwrap();
}

/// An accepted Folder stays revocable however many objects it holds. The
/// per-Folder rotation limit once refused more than 1,000 re-encrypted
/// records, so a Member holding the key of a larger Folder could not be removed.
#[test]
fn removal_rotates_a_folder_holding_more_than_a_thousand_objects() {
    let mut store = demoted_admin_store();
    // demoted_admin_store already holds obj_000000000001.
    for index in 2..=1_001 {
        store
            .submit_sync_record(
                &acme(),
                &revision_record(
                    &format!("event-create-{index}"),
                    &format!("obj_{index:012}"),
                    1,
                    None,
                    "create",
                ),
            )
            .unwrap();
    }
    let planned = plan(&store, DEMOTED).unwrap();
    let mut rotations = rotations_for(&store, &planned);
    let strategy = rotations
        .iter_mut()
        .find(|rotation| rotation.folder_id.as_str() == "strategy")
        .unwrap();
    strategy.reencrypted_records = (1..=1_001)
        .map(|index| {
            revision_record_struct(
                &format!("event-removal-reencrypt-{index}"),
                "strategy",
                &format!("obj_{index:012}"),
                2,
                Some(1),
                "reencrypted",
            )
        })
        .collect();

    remove_demoted(&mut store, &rotations).unwrap();

    assert_eq!(folder(&store, "strategy").current_key_version, 2);
    assert!(!current_grant_recipients(&store, "strategy").contains(&user(DEMOTED)));
    assert!(
        store
            .load_brain(&acme())
            .unwrap()
            .brain
            .members
            .iter()
            .all(|member| member.user_id != user(DEMOTED))
    );
}
