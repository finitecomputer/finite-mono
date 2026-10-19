//! Admin demotion routes: the rotating `POST .../demotion` and the older
//! rotation-free `DELETE` shape.

use super::*;

const EVENT_MISMATCH: &str =
    "signed event mismatch: access-change payload fields differ from expected request";

/// `acme` with `getting-started` (All-Members) and `restricted`; `target` is
/// a Member promoted to admin who then received the restricted v1 key by role.
async fn org_with_promoted_target(admin: &Keys, target: &str) -> Router {
    let router = router_with_test_org_folders(admin).await;
    for (action, route, id) in [
        (
            AdminAccessAction::AddMember,
            format!("/v1/admin/brains/acme/members/{target}"),
            "add-member",
        ),
        (
            AdminAccessAction::AddAdmin,
            format!("/v1/admin/brains/acme/roles/admin/{target}"),
            "promote",
        ),
        (
            AdminAccessAction::GrantFolderAccess,
            format!("/v1/admin/brains/acme/folders/restricted/access/{target}"),
            "role-wrap",
        ),
    ] {
        let (folder, key_version) = if action == AdminAccessAction::GrantFolderAccess {
            (Some("restricted"), Some(1))
        } else {
            (None, None)
        };
        let mut body = serde_json::json!({
            "accessChangeEvent": admin_event(admin, "acme", id, action, folder, Some(target), key_version)
        });
        if action == AdminAccessAction::GrantFolderAccess {
            body["grant"] = folder_key_grant_value("restricted-target-v1", 1, target);
        }
        let response = authed_request(
            router.clone(),
            admin,
            "PUT",
            &route,
            Some(body.to_string()),
            TEST_NOW,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{id}");
    }
    router
}

/// One restricted rotation signed by `signer`, re-wrapping the key for
/// `recipient` only.
fn restricted_rotation(
    signer: &Keys,
    recipient: &str,
    change_id: &str,
    folder: &str,
    key_version: u32,
) -> serde_json::Value {
    serde_json::json!({
        "folderId": "restricted",
        "newKeyVersion": 2,
        "grants": [folder_key_grant_value("restricted-admin-v2", 2, recipient)],
        "reencryptedRecords": [],
        "accessChangeEvent": admin_event(
            signer,
            "acme",
            change_id,
            AdminAccessAction::RotateFolderKey,
            Some(folder),
            None,
            Some(key_version),
        ),
    })
}

fn demotion_body(signer: &Keys, target: &str, rotations: Vec<serde_json::Value>) -> String {
    serde_json::json!({
        "accessChangeEvent": admin_event(
            signer,
            "acme",
            "demote",
            AdminAccessAction::RemoveAdmin,
            None,
            Some(target),
            None,
        ),
        "rotations": rotations,
    })
    .to_string()
}

/// Signed reads replay-protect identical requests, so each read in a test
/// passes its own offset from `TEST_NOW`.
async fn metadata(router: &Router, admin: &Keys, read: u64) -> BrainMetadataResponse {
    read_json(
        authed_request(
            router.clone(),
            admin,
            "GET",
            "/v1/brains/acme/metadata",
            None,
            TEST_NOW + read,
        )
        .await,
    )
    .await
}

async fn sequence(router: &Router, admin: &Keys, read: u64) -> u64 {
    latest_sync_sequence_at(router, admin, "acme", TEST_NOW + read).await
}

fn key_version(metadata: &BrainMetadataResponse, folder: &str) -> u32 {
    metadata
        .folders
        .iter()
        .find(|candidate| candidate.id == folder)
        .unwrap()
        .current_key_version
}

#[tokio::test]
async fn demotion_rotates_role_only_folders_and_records_each_signed_change() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let target = npub(&member);
    let router = org_with_promoted_target(&admin, &target).await;
    let sequence = latest_sync_sequence(&router, &admin, "acme").await;

    let response = authed_request(
        router.clone(),
        &admin,
        "POST",
        &format!("/v1/admin/brains/acme/roles/admin/{target}/demotion"),
        Some(demotion_body(
            &admin,
            &target,
            vec![restricted_rotation(
                &admin,
                &npub(&admin),
                "rotate-restricted",
                "restricted",
                2,
            )],
        )),
        TEST_NOW,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let demoted: DemoteAdminResponse = read_json(response).await;
    assert_eq!(demoted.metadata.admins, vec![npub(&admin)]);
    assert!(demoted.metadata.members.contains(&target));
    assert_eq!(
        demoted.rotations,
        vec![FolderKeyRotationReceipt {
            folder_id: "restricted".to_owned(),
            previous_key_version: 1,
            new_key_version: 2,
            recipient_npubs: vec![npub(&admin)],
            cut_off_npubs: vec![target.clone()],
        }]
    );
    assert_eq!(key_version(&demoted.metadata, "restricted"), 2);
    assert_eq!(key_version(&demoted.metadata, "getting-started"), 1);

    // The audit trail holds the demotion, the rotation it caused and the new
    // grant, all signed by the acting admin.
    let pulled: SyncPullResponse = read_json(
        authed_request(
            router.clone(),
            &admin,
            "GET",
            &format!("/v1/brains/acme/sync/records?after={sequence}&limit=20"),
            None,
            TEST_NOW,
        )
        .await,
    )
    .await;
    let changes = pulled
        .records
        .iter()
        .filter(|record| record.record_type == "brain_admin_access_change")
        .map(|record| {
            let payload: serde_json::Value = serde_json::from_str(&record.payload_json).unwrap();
            assert_eq!(record.actor_npub, npub(&admin));
            (
                payload["action"].as_str().unwrap().to_owned(),
                payload["folderId"].as_str().map(ToOwned::to_owned),
                payload["targetNpub"].as_str().map(ToOwned::to_owned),
                payload["keyVersion"].as_u64(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        changes,
        vec![
            ("remove-admin".to_owned(), None, Some(target.clone()), None),
            (
                "rotate-folder-key".to_owned(),
                Some("restricted".to_owned()),
                None,
                Some(2)
            ),
        ]
    );
    assert_eq!(
        pulled
            .records
            .iter()
            .filter(|record| record.record_type == "folder_key_grant")
            .count(),
        1
    );
}

#[tokio::test]
async fn older_demotion_request_refuses_when_folder_keys_must_rotate() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let target = npub(&member);
    let router = org_with_promoted_target(&admin, &target).await;
    let before = sequence(&router, &admin, 1).await;
    let older = serde_json::json!({
        "accessChangeEvent": admin_event(&admin, "acme", "demote", AdminAccessAction::RemoveAdmin,
            None, Some(&target), None),
    })
    .to_string();

    let refused = authed_request(
        router.clone(),
        &admin,
        "DELETE",
        &format!("/v1/admin/brains/acme/roles/admin/{target}"),
        Some(older),
        TEST_NOW,
    )
    .await;
    assert_error(
        refused,
        StatusCode::CONFLICT,
        "must also rotate the Folder Keys of [restricted]",
    )
    .await;
    let after = metadata(&router, &admin, 2).await;
    assert!(after.admins.contains(&target));
    assert_eq!(key_version(&after, "restricted"), 1);
    assert_eq!(sequence(&router, &admin, 3).await, before);
}

#[tokio::test]
async fn older_demotion_request_still_demotes_when_nothing_must_rotate() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let target = npub(&member);
    let router = router_with_test_org_folders(&admin).await;
    // Explicit restricted access granted before the promotion survives it.
    for (action, route, id, folder, key_version) in [
        (
            AdminAccessAction::AddMember,
            format!("/v1/admin/brains/acme/members/{target}"),
            "add-member",
            None,
            None,
        ),
        (
            AdminAccessAction::GrantFolderAccess,
            format!("/v1/admin/brains/acme/folders/restricted/access/{target}"),
            "explicit-access",
            Some("restricted"),
            Some(1),
        ),
        (
            AdminAccessAction::AddAdmin,
            format!("/v1/admin/brains/acme/roles/admin/{target}"),
            "promote",
            None,
            None,
        ),
    ] {
        let mut body = serde_json::json!({
            "accessChangeEvent": admin_event(&admin, "acme", id, action, folder, Some(&target), key_version)
        });
        if folder.is_some() {
            body["grant"] = folder_key_grant_value("restricted-target-v1", 1, &target);
        }
        let response = authed_request(
            router.clone(),
            &admin,
            "PUT",
            &route,
            Some(body.to_string()),
            TEST_NOW,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{id}");
    }

    let demoted = authed_request(
        router.clone(),
        &admin,
        "DELETE",
        &format!("/v1/admin/brains/acme/roles/admin/{target}"),
        Some(
            serde_json::json!({
                "accessChangeEvent": admin_event(&admin, "acme", "demote",
                    AdminAccessAction::RemoveAdmin, None, Some(&target), None),
            })
            .to_string(),
        ),
        TEST_NOW,
    )
    .await;
    assert_eq!(demoted.status(), StatusCode::OK);
    let demoted: BrainMetadataResponse = read_json(demoted).await;
    assert_eq!(demoted.admins, vec![npub(&admin)]);
    assert_eq!(key_version(&demoted, "restricted"), 1);
}

#[tokio::test]
async fn demotion_requires_an_admin_and_binds_every_signed_event() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let target = npub(&member);
    let router = org_with_promoted_target(&admin, &target).await;
    let other = Keys::generate();
    let other_npub = npub(&other);
    let add_other = authed_request(
        router.clone(),
        &admin,
        "PUT",
        &format!("/v1/admin/brains/acme/members/{other_npub}"),
        Some(
            serde_json::json!({
                "accessChangeEvent": admin_event(&admin, "acme", "add-other",
                    AdminAccessAction::AddMember, None, Some(&other_npub), None),
            })
            .to_string(),
        ),
        TEST_NOW,
    )
    .await;
    assert_eq!(add_other.status(), StatusCode::OK);
    let route = format!("/v1/admin/brains/acme/roles/admin/{target}/demotion");
    let before = sequence(&router, &admin, 1).await;
    let rotation =
        || restricted_rotation(&admin, &npub(&admin), "rotate-restricted", "restricted", 2);

    let mut wrong_action: serde_json::Value =
        serde_json::from_str(&demotion_body(&admin, &target, vec![rotation()])).unwrap();
    wrong_action["accessChangeEvent"] = serde_json::to_value(admin_event(
        &admin,
        "acme",
        "demote",
        AdminAccessAction::RemoveMember,
        None,
        Some(&target),
        None,
    ))
    .unwrap();
    let mut wrong_rotation_action = rotation();
    wrong_rotation_action["accessChangeEvent"] = serde_json::to_value(admin_event(
        &admin,
        "acme",
        "rotate-restricted",
        AdminAccessAction::RemoveFolderAccess,
        Some("restricted"),
        Some(&target),
        Some(2),
    ))
    .unwrap();
    let refusals = [
        (
            &other,
            demotion_body(&other, &target, vec![rotation()]),
            StatusCode::FORBIDDEN,
            "brain admin access required",
        ),
        (
            &admin,
            wrong_action.to_string(),
            StatusCode::BAD_REQUEST,
            EVENT_MISMATCH,
        ),
        (
            &admin,
            demotion_body(&admin, &other_npub, vec![rotation()]),
            StatusCode::BAD_REQUEST,
            EVENT_MISMATCH,
        ),
        (
            &admin,
            demotion_body(
                &admin,
                &target,
                vec![restricted_rotation(
                    &admin,
                    &npub(&admin),
                    "rotate-restricted",
                    "getting-started",
                    2,
                )],
            ),
            StatusCode::BAD_REQUEST,
            EVENT_MISMATCH,
        ),
        (
            &admin,
            demotion_body(
                &admin,
                &target,
                vec![restricted_rotation(
                    &admin,
                    &npub(&admin),
                    "rotate-restricted",
                    "restricted",
                    3,
                )],
            ),
            StatusCode::BAD_REQUEST,
            EVENT_MISMATCH,
        ),
        (
            &admin,
            demotion_body(&admin, &target, vec![wrong_rotation_action]),
            StatusCode::BAD_REQUEST,
            EVENT_MISMATCH,
        ),
        (
            &admin,
            demotion_body(&admin, &target, Vec::new()),
            StatusCode::CONFLICT,
            "admin demotion must rotate exactly the Folder Keys of [restricted]",
        ),
    ];
    for (read, (signer, body, status, message)) in (10..).step_by(2).zip(refusals) {
        let response =
            authed_request(router.clone(), signer, "POST", &route, Some(body), TEST_NOW).await;
        assert_error(response, status, message).await;
        assert!(
            metadata(&router, &admin, read)
                .await
                .admins
                .contains(&target)
        );
        assert_eq!(sequence(&router, &admin, read + 1).await, before);
    }

    let body = demotion_body(&admin, &target, vec![rotation()]);
    let accepted = authed_request(
        router.clone(),
        &admin,
        "POST",
        &route,
        Some(body.clone()),
        TEST_NOW,
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let committed = sequence(&router, &admin, 2).await;
    // Resubmitting the committed demotion under a fresh HTTP auth event is
    // refused by the business rule, not by replay protection.
    let resubmitted = authed_request(
        router.clone(),
        &admin,
        "POST",
        &route,
        Some(body),
        TEST_NOW + 5,
    )
    .await;
    assert_error(
        resubmitted,
        StatusCode::BAD_REQUEST,
        "brain admin does not exist",
    )
    .await;
    assert_eq!(
        key_version(&metadata(&router, &admin, 3).await, "restricted"),
        2
    );
    assert_eq!(sequence(&router, &admin, 4).await, committed);
}

#[tokio::test]
async fn an_admin_demotes_itself_and_signs_the_rotations() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let target = npub(&member);
    let router = org_with_promoted_target(&admin, &target).await;

    // The target signs its own demotion and every rotation; the new key is
    // wrapped only for the remaining admin.
    let response = authed_request(
        router.clone(),
        &member,
        "POST",
        &format!("/v1/admin/brains/acme/roles/admin/{target}/demotion"),
        Some(demotion_body(
            &member,
            &target,
            vec![restricted_rotation(
                &member,
                &npub(&admin),
                "rotate-restricted",
                "restricted",
                2,
            )],
        )),
        TEST_NOW,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let demoted: DemoteAdminResponse = read_json(response).await;
    assert_eq!(demoted.metadata.admins, vec![npub(&admin)]);
    assert_eq!(
        demoted.rotations,
        vec![FolderKeyRotationReceipt {
            folder_id: "restricted".to_owned(),
            previous_key_version: 1,
            new_key_version: 2,
            recipient_npubs: vec![npub(&admin)],
            cut_off_npubs: vec![target.clone()],
        }]
    );

    // Now a Member, the same key can no longer demote anyone.
    let refused = authed_request(
        router.clone(),
        &member,
        "POST",
        &format!(
            "/v1/admin/brains/acme/roles/admin/{}/demotion",
            npub(&admin)
        ),
        Some(demotion_body(&member, &npub(&admin), Vec::new())),
        TEST_NOW + 1,
    )
    .await;
    assert_error(
        refused,
        StatusCode::FORBIDDEN,
        "brain admin access required",
    )
    .await;
}

#[tokio::test]
async fn demotion_accepts_bodies_over_the_default_limit_up_to_its_own() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let target = npub(&member);
    let router = org_with_promoted_target(&admin, &target).await;
    let route = format!("/v1/admin/brains/acme/roles/admin/{target}/demotion");
    // Unknown fields are ignored, so padding sizes the body without changing
    // the demotion it carries.
    let padded = |size: usize| -> String {
        let mut body: serde_json::Value = serde_json::from_str(&demotion_body(
            &admin,
            &target,
            vec![restricted_rotation(
                &admin,
                &npub(&admin),
                "rotate-restricted",
                "restricted",
                2,
            )],
        ))
        .unwrap();
        let unpadded = body.to_string().len() + r#","padding":"""#.len();
        body["padding"] = "x".repeat(size - unpadded).into();
        let body = body.to_string();
        assert_eq!(body.len(), size);
        body
    };

    let over = authed_request(
        router.clone(),
        &admin,
        "POST",
        &route,
        Some(padded(
            finite_brain_core::MAX_FOLDER_ROTATION_REQUEST_BODY_BYTES + 1,
        )),
        TEST_NOW,
    )
    .await;
    assert_eq!(over.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(metadata(&router, &admin, 1).await.admins.contains(&target));

    let large = padded(MAX_REQUEST_BODY_BYTES * 4);
    let accepted = authed_request(
        router.clone(),
        &admin,
        "POST",
        &route,
        Some(large),
        TEST_NOW,
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(
        metadata(&router, &admin, 2).await.admins,
        vec![npub(&admin)]
    );
}
