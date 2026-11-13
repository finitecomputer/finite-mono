//! Opening a Restricted Folder to every Member: the mode change, the new
//! grants and the signed audit record commit together, or nothing does.

use super::*;

const ADMIN: &str = "npub-admin";
const MEMBER: &str = "npub-member";
const INSIDER: &str = "npub-insider";
const GUEST: &str = "npub-guest";

fn acme() -> BrainId {
    BrainId::new("acme").unwrap()
}

fn user(npub: &str) -> UserId {
    UserId::new(npub).unwrap()
}

fn private_project() -> FolderId {
    FolderId::new("private-project").unwrap()
}

/// `acme` with Restricted `private-project` read by the admin, `npub-insider`
/// (a Member with explicit access) and `npub-guest` (a Folder Guest), plus a
/// Restricted child only the admin reads. `npub-member` reads neither.
fn store() -> BrainStore {
    let mut store = org_store_with_access_test_folders();
    let brain_id = acme();
    store.add_member(&brain_id, &user(MEMBER)).unwrap();
    store.add_member(&brain_id, &user(INSIDER)).unwrap();
    for recipient in [INSIDER, GUEST] {
        store
            .grant_folder_access(
                &brain_id,
                &private_project(),
                &user(recipient),
                &grant(
                    &format!("private-project-{recipient}"),
                    "private-project",
                    1,
                    ADMIN,
                    recipient,
                ),
            )
            .unwrap();
    }
    store
        .create_folder(
            &brain_id,
            &Folder {
                id: FolderId::new("private-child").unwrap(),
                name: DisplayName::new("folder_name", "Child").unwrap(),
                role: FolderRole::Folder,
                access: FolderAccessMode::Restricted,
                parent_folder_id: Some(private_project()),
                path: SafeRelativePath::new("folder_path", "Private Project/Child").unwrap(),
                current_key_version: 1,
            },
            &BTreeSet::new(),
            &[grant(
                "private-child-admin",
                "private-child",
                1,
                ADMIN,
                ADMIN,
            )],
        )
        .unwrap();
    store
}

/// Client-prepared grants for `recipients` at `key_version`, with records.
fn wraps(recipients: &[&str], key_version: u32) -> Vec<(FolderKeyGrantMetadata, SyncRecordInput)> {
    recipients
        .iter()
        .map(|recipient| {
            let grant = grant(
                &format!("open-{recipient}-v{key_version}"),
                "private-project",
                key_version,
                ADMIN,
                recipient,
            );
            let record = folder_key_grant_control_record(&grant, &format!("open-{recipient}"));
            (grant, record)
        })
        .collect()
}

fn audit(record_event_id: &str) -> SyncRecordInput {
    folder_access_control_record(
        record_event_id,
        SyncRecordType::BrainAdminAccessChange,
        "private-project",
        ADMIN,
    )
}

fn open(
    store: &mut BrainStore,
    key_version: u32,
    grants: &[(FolderKeyGrantMetadata, SyncRecordInput)],
) -> Result<OpenFolderToAllMembersOutcome, StoreError> {
    store.open_folder_to_all_members_with_control_records(
        &acme(),
        &private_project(),
        key_version,
        grants,
        &audit("open-audit"),
    )
}

/// Everything the operation may write, to prove a rejection wrote nothing.
fn snapshot(store: &BrainStore) -> (FolderAccessMode, usize, u64) {
    let stored = store.load_brain(&acme()).unwrap();
    let access = stored
        .brain
        .folders
        .iter()
        .find(|folder| folder.id == private_project())
        .unwrap()
        .access;
    (
        access,
        stored.grants.len(),
        store.latest_sequence(&acme()).unwrap(),
    )
}

#[test]
fn opening_grants_new_readers_and_keeps_guests_and_children() {
    let mut store = store();
    let before = store.latest_sequence(&acme()).unwrap();

    // The client wraps for every reader; current holders are skipped.
    let outcome = open(&mut store, 1, &wraps(&[ADMIN, MEMBER, INSIDER, GUEST], 1)).unwrap();

    assert_eq!(
        outcome,
        OpenFolderToAllMembersOutcome::Opened {
            granted_recipients: vec![user(MEMBER)]
        }
    );
    let stored = store.load_brain(&acme()).unwrap();
    let folder = |id: &str| {
        stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id.as_str() == id)
            .unwrap()
    };
    assert_eq!(
        folder("private-project").access,
        FolderAccessMode::AllMembers
    );
    assert_eq!(folder("private-project").current_key_version, 1);
    assert_eq!(folder("private-child").access, FolderAccessMode::Restricted);
    assert_eq!(
        stored.folder_access[&private_project()],
        BTreeSet::from([user(GUEST), user(INSIDER)])
    );
    let holders = |folder_id: &str| {
        stored
            .grants
            .iter()
            .filter(|grant| grant.folder_id.as_str() == folder_id)
            .map(|grant| grant.recipient_npub.to_string())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        holders("private-project"),
        BTreeSet::from([ADMIN, GUEST, INSIDER, MEMBER].map(ToOwned::to_owned))
    );
    assert_eq!(holders("private-child"), BTreeSet::from([ADMIN.to_owned()]));
    let appended = store
        .pull_sync_records(&acme(), before, 100)
        .unwrap()
        .records;
    let appended = appended
        .iter()
        .map(|record| (record.record_type, record.record_event_id.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        appended,
        vec![
            (SyncRecordType::FolderKeyGrant, "open-npub-member"),
            (SyncRecordType::BrainAdminAccessChange, "open-audit"),
        ]
    );
}

#[test]
fn opening_twice_is_a_noop() {
    let mut store = store();
    let grants = wraps(&[ADMIN, MEMBER, INSIDER, GUEST], 1);
    open(&mut store, 1, &grants).unwrap();
    let after_first = snapshot(&store);

    assert_eq!(
        open(&mut store, 1, &grants).unwrap(),
        OpenFolderToAllMembersOutcome::AlreadyOpen
    );
    assert_eq!(snapshot(&store), after_first);
}

#[test]
fn a_member_who_joined_after_preparation_is_a_conflict() {
    let mut store = store();
    let prepared = wraps(&[ADMIN, MEMBER, INSIDER, GUEST], 1);
    store.add_member(&acme(), &user("npub-joined")).unwrap();
    let before = snapshot(&store);

    let error = open(&mut store, 1, &prepared).unwrap_err();

    assert!(
        matches!(&error, StoreError::Conflict { reason, .. } if reason.contains("npub-joined")),
        "{error:?}"
    );
    assert_eq!(snapshot(&store), before);
}

#[test]
fn a_grant_for_someone_who_cannot_read_the_opened_folder_is_a_conflict() {
    let mut store = store();
    let before = snapshot(&store);

    let error = open(&mut store, 1, &wraps(&[MEMBER, "npub-outsider"], 1)).unwrap_err();

    assert!(
        matches!(&error, StoreError::Conflict { reason, .. } if reason.contains("npub-outsider")),
        "{error:?}"
    );
    assert_eq!(snapshot(&store), before);
}

#[test]
fn a_key_rotation_after_preparation_is_a_conflict() {
    let mut store = store();
    store
        .conn
        .execute(
            "UPDATE folders SET current_key_version = 2 WHERE brain_id = 'acme' AND id = 'private-project'",
            [],
        )
        .unwrap();
    let before = snapshot(&store);

    let error = open(&mut store, 1, &wraps(&[MEMBER], 1)).unwrap_err();

    assert!(matches!(error, StoreError::Conflict { .. }), "{error:?}");
    assert_eq!(snapshot(&store), before);
}

#[test]
fn a_failed_audit_append_rolls_back_the_mode_and_grants() {
    let mut store = store();
    let before = snapshot(&store);
    // Reuse an accepted record id so the last append in the transaction fails.
    let duplicate = audit("private-project-npub-guest-access-record");

    let error = store
        .open_folder_to_all_members_with_control_records(
            &acme(),
            &private_project(),
            1,
            &wraps(&[MEMBER], 1),
            &duplicate,
        )
        .unwrap_err();

    assert!(
        !matches!(
            error,
            StoreError::Conflict { .. } | StoreError::MissingFolder { .. }
        ),
        "{error:?}"
    );
    assert_eq!(snapshot(&store), before);
}

#[test]
fn only_a_restricted_organization_folder_opens() {
    let mut store = store();
    store
        .create_folder(
            &acme(),
            &admin_only_folder(),
            &BTreeSet::new(),
            &[grant("admin-only-admin", "admin-only", 1, ADMIN, ADMIN)],
        )
        .unwrap();
    let error = store
        .open_folder_to_all_members_with_control_records(
            &acme(),
            &FolderId::new("admin-only").unwrap(),
            1,
            &[],
            &folder_access_control_record(
                "admin-only-audit",
                SyncRecordType::BrainAdminAccessChange,
                "admin-only",
                ADMIN,
            ),
        )
        .unwrap_err();
    assert!(
        matches!(error, StoreError::BrokenInvariant { .. }),
        "{error:?}"
    );
}
