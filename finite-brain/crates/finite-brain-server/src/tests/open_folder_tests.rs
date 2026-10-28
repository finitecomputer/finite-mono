//! `POST /v1/admin/brains/{brain}/folders/{folder}/open-to-all-members`.

use super::*;

const ROUTE: &str = "/v1/admin/brains/acme/folders/restricted/open-to-all-members";

/// A signed `set-folder-access-mode` event for `restricted` with `note`.
fn open_event(admin: &Keys, change_id: &str, key_version: u32, note: Option<&str>) -> Event {
    let expected = AdminAccessChangeValidation {
        brain_id: BrainId::new("acme").unwrap(),
        change_id: change_id.to_owned(),
        action: AdminAccessAction::SetFolderAccessMode,
        admin_npub: npub(admin),
        folder_id: Some(FolderId::new("restricted").unwrap()),
        target_npub: None,
        key_version: Some(key_version),
        note: note.map(ToOwned::to_owned),
        created_at: test_rfc3339(),
    };
    sign_app_event(
        admin,
        AdminAccessChangePayload::new(&expected).canonical_json(),
        admin_access_change_tags(&expected),
    )
}

fn open_body(admin: &Keys, member: &str, key_version: u32, note: Option<&str>) -> String {
    serde_json::json!({
        "keyVersion": key_version,
        "grants": [
            folder_key_grant_value("open-admin", key_version, &npub(admin)),
            folder_key_grant_value("open-member", key_version, member),
        ],
        "accessChangeEvent": open_event(admin, "open-restricted", key_version, note),
    })
    .to_string()
}

/// `acme` with `getting-started` (All-Members) and `restricted`, plus one
/// Member who cannot read `restricted`.
async fn org_with_member(admin: &Keys, member: &str) -> Router {
    let router = router_with_test_org_folders(admin).await;
    let body = serde_json::json!({
        "accessChangeEvent": admin_event(
            admin, "acme", "add-member", AdminAccessAction::AddMember, None, Some(member), None,
        ),
    })
    .to_string();
    let added = authed_request(
        router.clone(),
        admin,
        "PUT",
        &format!("/v1/admin/brains/acme/members/{member}"),
        Some(body),
        TEST_NOW,
    )
    .await;
    assert_eq!(added.status(), StatusCode::OK);
    router
}

async fn restricted_access(router: &Router, admin: &Keys, at: u64) -> FolderAccessMode {
    let metadata = authed_request(
        router.clone(),
        admin,
        "GET",
        "/v1/brains/acme/metadata",
        None,
        at,
    )
    .await;
    let metadata: BrainMetadataResponse = read_json(metadata).await;
    metadata
        .folders
        .iter()
        .find(|folder| folder.id == "restricted")
        .unwrap()
        .access
}

#[tokio::test]
async fn opening_publishes_the_new_members_grant_and_repeats_as_a_noop() {
    let admin = Keys::generate();
    let member_keys = Keys::generate();
    let member = npub(&member_keys);
    let router = org_with_member(&admin, &member).await;
    let before = authed_request(
        router.clone(),
        &member_keys,
        "GET",
        "/v1/brains/acme/sync/records?after=0&limit=100",
        None,
        TEST_NOW + 1,
    )
    .await;
    let before: SyncPullResponse = read_json(before).await;

    let opened = authed_request(
        router.clone(),
        &admin,
        "POST",
        ROUTE,
        Some(open_body(&admin, &member, 1, Some("all_members"))),
        TEST_NOW + 2,
    )
    .await;
    let status = opened.status();
    let opened: OpenFolderToAllMembersResponse = read_json(opened).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        opened.outcome,
        OpenFolderToAllMembersResponseOutcome::Opened
    );
    assert_eq!(opened.granted_recipients, vec![member.clone()]);
    let folder = opened
        .metadata
        .folders
        .iter()
        .find(|folder| folder.id == "restricted")
        .unwrap();
    assert_eq!(folder.access, FolderAccessMode::AllMembers);
    assert_eq!(folder.current_key_version, 1);

    // The Member's routine pull carries its new grant for the opened Folder.
    let pull = authed_request(
        router.clone(),
        &member_keys,
        "GET",
        &format!(
            "/v1/brains/acme/sync/records?after={}&limit=100",
            before.latest_sequence
        ),
        None,
        TEST_NOW + 3,
    )
    .await;
    let pull: SyncPullResponse = read_json(pull).await;
    assert!(pull.records.iter().any(|record| {
        record.record_type == "folder_key_grant"
            && record.folder_id.as_deref() == Some("restricted")
            && matches!(
                decode_sync_payload(&record.payload_json),
                DecodedSyncPayload::KeyGrant { recipient_npub } if recipient_npub == member
            )
    }));

    // Replaying the request with fresh authentication writes nothing.
    let latest = pull.latest_sequence;
    let again = authed_request(
        router.clone(),
        &admin,
        "POST",
        ROUTE,
        Some(open_body(&admin, &member, 1, Some("all_members"))),
        TEST_NOW + 4,
    )
    .await;
    assert_eq!(again.status(), StatusCode::OK);
    let again: OpenFolderToAllMembersResponse = read_json(again).await;
    assert_eq!(
        again.outcome,
        OpenFolderToAllMembersResponseOutcome::AlreadyOpen
    );
    assert!(again.granted_recipients.is_empty());
    let after = authed_request(
        router,
        &admin,
        "GET",
        &format!("/v1/brains/acme/sync/records?after={latest}&limit=100"),
        None,
        TEST_NOW + 5,
    )
    .await;
    let after: SyncPullResponse = read_json(after).await;
    assert!(after.records.is_empty(), "{:?}", after.records);
}

#[tokio::test]
async fn opening_requires_an_admin_the_signed_note_and_the_current_key() {
    let admin = Keys::generate();
    let member_keys = Keys::generate();
    let member = npub(&member_keys);
    let router = org_with_member(&admin, &member).await;

    let by_member = authed_request(
        router.clone(),
        &member_keys,
        "POST",
        ROUTE,
        Some(open_body(&member_keys, &member, 1, Some("all_members"))),
        TEST_NOW + 1,
    )
    .await;
    assert_eq!(by_member.status(), StatusCode::FORBIDDEN);

    for (offset, note) in [(2, None), (3, Some("restricted"))] {
        let response = authed_request(
            router.clone(),
            &admin,
            "POST",
            ROUTE,
            Some(open_body(&admin, &member, 1, note)),
            TEST_NOW + offset,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{note:?}");
    }

    // Prepared against a key version the Folder is not at: a conflict.
    let stale = authed_request(
        router.clone(),
        &admin,
        "POST",
        ROUTE,
        Some(open_body(&admin, &member, 2, Some("all_members"))),
        TEST_NOW + 4,
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);

    assert_eq!(
        restricted_access(&router, &admin, TEST_NOW + 5).await,
        FolderAccessMode::Restricted
    );
}
