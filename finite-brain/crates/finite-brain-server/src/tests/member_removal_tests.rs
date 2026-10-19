//! Member removal through the signed router: the removal plan names the
//! Folders where a demoted Member still holds the current key, and a removal
//! that omits them (an older fbrain, or a stale plan) is refused without
//! changing anything.

use super::*;
use tempfile::TempDir;

/// `acme` with an admin and a Member who was promoted, received the
/// restricted Folder's current wrap, and was then demoted the legacy way: the
/// role row deleted in SQLite and every wrap kept. The Member now holds that
/// wrap without entitlement (`revocationIncomplete`), whatever the demotion
/// route does today. The returned directory owns the database file.
async fn router_with_demoted_member(admin: &Keys, member: &Keys) -> (Router, TempDir) {
    let target = npub(member);
    let database = TempDir::new().unwrap();
    let database_path = database.path().join("brain.sqlite3");
    let state = ServerState::new(BrainStore::open(&database_path).unwrap(), TEST_BASE_URL)
        .with_auth_clock(TEST_NOW, 60);
    let router = router_with_state(state);
    let created = post_brain(
        router.clone(),
        admin,
        &create_brain_body("acme", "organization"),
        TEST_NOW,
        None,
        None,
        None,
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    add_test_org_folders(&router, admin).await;
    for (action, route, id) in [
        (
            AdminAccessAction::AddMember,
            format!("/v1/admin/brains/acme/members/{target}"),
            "removal-add-member",
        ),
        (
            AdminAccessAction::AddAdmin,
            format!("/v1/admin/brains/acme/roles/admin/{target}"),
            "removal-promote",
        ),
    ] {
        let response = authed_request(
            router.clone(),
            admin,
            "PUT",
            &route,
            Some(
                serde_json::json!({"accessChangeEvent": admin_event(admin, "acme", id,
                    action, None, Some(&target), None)})
                .to_string(),
            ),
            TEST_NOW,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let grant = authed_request(
        router.clone(),
        admin,
        "PUT",
        &format!("/v1/admin/brains/acme/folders/restricted/access/{target}"),
        Some(
            serde_json::json!({
                "grant": folder_key_grant_value("removal-restricted-v1", 1, &target),
                "accessChangeEvent": admin_event(admin, "acme", "removal-restricted-v1",
                    AdminAccessAction::GrantFolderAccess, Some("restricted"), Some(&target), Some(1))
            })
            .to_string(),
        ),
        TEST_NOW,
    )
    .await;
    assert_eq!(grant.status(), StatusCode::OK);
    let demoted = rusqlite::Connection::open(&database_path)
        .unwrap()
        .execute(
            "DELETE FROM brain_admins WHERE brain_id = 'acme' AND user_id = ?1",
            [target.as_str()],
        )
        .unwrap();
    assert_eq!(demoted, 1);
    (router, database)
}

async fn removal_plan(
    router: &Router,
    signer: &Keys,
    target: &str,
    created_at: u64,
) -> axum::response::Response {
    authed_request(
        router.clone(),
        signer,
        "GET",
        &format!("/v1/admin/brains/acme/members/{target}/removal-plan"),
        None,
        created_at,
    )
    .await
}

/// Rotation material for `folder_ids`, wrapped for the admin alone (the only
/// remaining Member). No live objects exist in the test Folders.
fn remove_member_body(admin: &Keys, target: &str, folder_ids: &[&str]) -> String {
    let admin_npub = npub(admin);
    serde_json::json!({
        "accessChangeEvent": admin_event(admin, "acme", &format!("removal-{}", folder_ids.join("-")),
            AdminAccessAction::RemoveMember, None, Some(target), None),
        "rotations": folder_ids.iter().map(|folder_id| serde_json::json!({
            "folderId": folder_id,
            "newKeyVersion": 2,
            "grants": [folder_key_grant_value(
                &format!("removal-{folder_id}-admin-v2"),
                2,
                &admin_npub,
            )],
            "reencryptedRecords": [],
        })).collect::<Vec<_>>(),
        "mountRotations": [],
    })
    .to_string()
}

#[tokio::test]
async fn removal_plan_names_readable_and_stale_grant_folders_for_admins_only() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let target = npub(&member);
    let (router, _database) = router_with_demoted_member(&admin, &member).await;

    let response = removal_plan(&router, &admin, &target, TEST_NOW).await;
    assert_eq!(response.status(), StatusCode::OK);
    let plan: MemberRemovalPlanResponse = read_json(response).await;
    assert_eq!(plan.brain_id, "acme");
    assert_eq!(plan.target_npub, target);
    let reasons = plan
        .folder_rotations
        .iter()
        .map(|rotation| {
            assert_eq!(rotation.current_key_version, 1);
            (rotation.folder_id.as_str(), rotation.reason)
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        reasons.get("getting-started"),
        Some(&MemberRemovalRotationReasonResponse::Readable)
    );
    assert_eq!(
        reasons.get("restricted"),
        Some(&MemberRemovalRotationReasonResponse::StaleCurrentGrant)
    );

    // The Member itself is no admin; the plan is admin-only metadata.
    let denied = removal_plan(&router, &member, &target, TEST_NOW + 1).await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    // Same refusals as the removal: demote an admin first, and only Members.
    let admin_target = removal_plan(&router, &admin, &npub(&admin), TEST_NOW + 2).await;
    assert_error(admin_target, StatusCode::BAD_REQUEST, "remove admin role").await;
    let stranger = npub(&Keys::generate());
    let not_member = removal_plan(&router, &admin, &stranger, TEST_NOW + 3).await;
    assert_error(
        not_member,
        StatusCode::BAD_REQUEST,
        "brain member does not exist",
    )
    .await;
    let malformed = removal_plan(&router, &admin, "not-a-key", TEST_NOW + 4).await;
    assert_error(
        malformed,
        StatusCode::BAD_REQUEST,
        "invalid member removal target",
    )
    .await;
}

#[tokio::test]
async fn removal_without_the_stale_grant_rotation_is_refused_then_full_removal_rotates() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let target = npub(&member);
    let (router, _database) = router_with_demoted_member(&admin, &member).await;
    let plan: MemberRemovalPlanResponse =
        read_json(removal_plan(&router, &admin, &target, TEST_NOW).await).await;
    let readable = plan
        .folder_rotations
        .iter()
        .filter(|rotation| rotation.reason == MemberRemovalRotationReasonResponse::Readable)
        .map(|rotation| rotation.folder_id.as_str())
        .collect::<Vec<_>>();
    let planned = plan
        .folder_rotations
        .iter()
        .map(|rotation| rotation.folder_id.as_str())
        .collect::<Vec<_>>();
    let route = format!("/v1/admin/brains/acme/members/{target}");
    let sequence_before = latest_sync_sequence(&router, &admin, "acme").await;

    // An fbrain from before this rule rotates only the Folders it can see the
    // Member reading. The server refuses and names what is missing.
    let old_client = authed_request(
        router.clone(),
        &admin,
        "DELETE",
        &route,
        Some(remove_member_body(&admin, &target, &readable)),
        TEST_NOW + 1,
    )
    .await;
    // Same typed 409 contract as admin demotion's rotation-set mismatch.
    assert_eq!(old_client.status(), StatusCode::CONFLICT);
    let message = read_text(old_client).await;
    assert!(message.contains("[restricted]"), "{message}");
    assert!(
        message.contains("without being entitled to it"),
        "{message}"
    );
    assert!(
        message.contains("Upgrade fbrain if using an older version"),
        "{message}"
    );
    assert!(
        message.contains("otherwise retry the removal to refresh its plan"),
        "{message}"
    );
    assert!(message.contains("nothing was changed"), "{message}");
    // Any other mismatch, here a duplicated rotation, is the same 409 with
    // the shared rotation-set wording.
    let mut duplicated = planned.clone();
    duplicated.push(planned[0]);
    let mismatched = authed_request(
        router.clone(),
        &admin,
        "DELETE",
        &route,
        Some(remove_member_body(&admin, &target, &duplicated)),
        TEST_NOW + 1,
    )
    .await;
    assert_eq!(mismatched.status(), StatusCode::CONFLICT);
    let message = read_text(mismatched).await;
    assert!(
        message.contains("member removal must rotate exactly the Folder Keys of"),
        "{message}"
    );
    assert!(message.contains("nothing was changed"), "{message}");
    let unchanged: BrainMetadataResponse =
        read_json(get_metadata(router.clone(), &admin, "acme", TEST_NOW + 2).await).await;
    assert!(unchanged.members.contains(&target));
    assert!(
        unchanged
            .folders
            .iter()
            .all(|folder| folder.current_key_version == 1)
    );
    assert_eq!(
        latest_sync_sequence_at(&router, &admin, "acme", TEST_NOW + 3).await,
        sequence_before
    );

    let removed = authed_request(
        router.clone(),
        &admin,
        "DELETE",
        &route,
        Some(remove_member_body(&admin, &target, &planned)),
        TEST_NOW + 4,
    )
    .await;
    let status = removed.status();
    let text = read_text(removed).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let metadata: BrainMetadataResponse = serde_json::from_str(&text).unwrap();
    assert!(!metadata.members.contains(&target));
    for folder in &metadata.folders {
        let expected = if planned.contains(&folder.id.as_str()) {
            2
        } else {
            1
        };
        assert_eq!(folder.current_key_version, expected, "{}", folder.id);
    }
}
