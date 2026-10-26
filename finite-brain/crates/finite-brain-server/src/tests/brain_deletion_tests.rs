//! Organization Brain deletion through the signed router: preview authority,
//! atomic refusal of stale or expired confirmations, exact-retry receipts, and
//! a deleted Brain that former readers learn about while everyone else sees
//! an unknown Brain id.

use super::*;

fn delete_event(keys: &Keys, brain_id: &str, change_id: &str, note: Option<&str>) -> Event {
    let expected = AdminAccessChangeValidation {
        brain_id: BrainId::new(brain_id).unwrap(),
        change_id: change_id.to_owned(),
        action: AdminAccessAction::DeleteBrain,
        admin_npub: npub(keys),
        folder_id: None,
        target_npub: None,
        key_version: None,
        note: note.map(ToOwned::to_owned),
        created_at: test_rfc3339(),
    };
    let payload = AdminAccessChangePayload::new(&expected);
    sign_app_event(
        keys,
        payload.canonical_json(),
        admin_access_change_tags(&expected),
    )
}

fn delete_body(event: &Event) -> String {
    serde_json::json!({ "deletionEvent": event }).to_string()
}

async fn preview(
    router: &Router,
    keys: &Keys,
    brain_id: &str,
    at: u64,
) -> axum::response::Response {
    authed_request(
        router.clone(),
        keys,
        "GET",
        &format!("/v1/brains/{brain_id}/deletion-preview"),
        None,
        at,
    )
    .await
}

async fn delete(
    router: &Router,
    keys: &Keys,
    brain_id: &str,
    body: &str,
    at: u64,
) -> axum::response::Response {
    authed_request(
        router.clone(),
        keys,
        "DELETE",
        &format!("/v1/brains/{brain_id}"),
        Some(body.to_owned()),
        at,
    )
    .await
}

async fn put_page(
    router: &Router,
    keys: &Keys,
    brain_id: &str,
    revision: u64,
    at: u64,
) -> axum::response::Response {
    let body = object_write_body(
        keys,
        RevisionFixture {
            brain_id,
            folder_id: "getting-started",
            object_id: "obj_000000000401",
            operation: if revision == 1 {
                FolderObjectOperation::Create
            } else {
                FolderObjectOperation::Update
            },
            revision,
            base_revision: (revision > 1).then(|| revision - 1),
            key_version: 1,
            content: "deletion page",
            nonce: 40 + revision as u8,
            record_type: false,
        },
    );
    authed_request(
        router.clone(),
        keys,
        "PUT",
        &format!("/v1/brains/{brain_id}/folders/getting-started/objects/obj_000000000401"),
        Some(body),
        at,
    )
    .await
}

/// `acme` with the two test Folders, a Member and one page; a second
/// Organization Brain `content-brain` the same admin controls.
async fn deletion_router(admin: &Keys, member: &Keys) -> (ServerState, Router) {
    let state = ServerState::new(BrainStore::open_in_memory().unwrap(), TEST_BASE_URL)
        .with_auth_clock(TEST_NOW, 60);
    let router = router_with_state(state.clone());
    for brain_id in ["acme", "content-brain"] {
        let created = post_brain(
            router.clone(),
            admin,
            &create_brain_body(brain_id, "organization"),
            TEST_NOW - 30,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(
            created.status(),
            StatusCode::OK,
            "{}",
            read_text(created).await
        );
    }
    add_test_org_folders(&router, admin).await;
    let target = npub(member);
    let added = authed_request(
        router.clone(),
        admin,
        "PUT",
        &format!("/v1/admin/brains/acme/members/{target}"),
        Some(
            serde_json::json!({"accessChangeEvent": admin_event(admin, "acme", "deletion-add-member",
                AdminAccessAction::AddMember, None, Some(&target), None)})
            .to_string(),
        ),
        TEST_NOW - 20,
    )
    .await;
    assert_eq!(added.status(), StatusCode::OK);
    assert_eq!(
        put_page(&router, admin, "acme", 1, TEST_NOW - 10)
            .await
            .status(),
        StatusCode::OK
    );
    (state, router)
}

#[tokio::test]
async fn admins_delete_through_the_signed_api_and_former_readers_see_gone() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let outsider = Keys::generate();
    let (state, router) = deletion_router(&admin, &member).await;
    let mut updates = state.brain_updates.subscribe();

    assert_error(
        preview(&router, &member, "acme", TEST_NOW).await,
        StatusCode::FORBIDDEN,
        "permanent deletion requires brain destructive authority",
    )
    .await;
    assert_error(
        preview(&router, &outsider, "acme", TEST_NOW).await,
        StatusCode::FORBIDDEN,
        "brain access required",
    )
    .await;
    let first = preview(&router, &admin, "acme", TEST_NOW + 1).await;
    assert_eq!(first.status(), StatusCode::OK);
    let first: BrainDeletionPreviewResponse = read_json(first).await;
    assert_eq!(first.server, TEST_BASE_URL);
    assert_eq!(first.name, "Acme");
    assert_eq!((first.folder_count, first.page_count), (2, 1));
    assert_eq!((first.member_count, first.admin_count), (2, 1));
    assert!(first.deletable);
    assert_eq!(first.confirmation_max_age_seconds, 600);

    // The signed note must carry a well-formed preview digest.
    for note in [None, Some("not-a-digest"), Some("sha256:XYZ")] {
        let event = delete_event(&admin, "acme", "delete-bad-note", note);
        assert_error(
            delete(&router, &admin, "acme", &delete_body(&event), TEST_NOW + 2).await,
            StatusCode::BAD_REQUEST,
            "stateDigest",
        )
        .await;
    }
    // A Member cannot delete even with the admin's digest.
    let member_event = delete_event(&member, "acme", "delete-member", Some(&first.state_digest));
    assert_error(
        delete(
            &router,
            &member,
            "acme",
            &delete_body(&member_event),
            TEST_NOW + 3,
        )
        .await,
        StatusCode::FORBIDDEN,
        "permanent deletion requires brain destructive authority",
    )
    .await;

    // An edit after the preview keeps every count but invalidates it.
    assert_eq!(
        put_page(&router, &admin, "acme", 2, TEST_NOW + 4)
            .await
            .status(),
        StatusCode::OK
    );
    let stale = delete_event(&admin, "acme", "delete-stale", Some(&first.state_digest));
    assert_error(
        delete(&router, &admin, "acme", &delete_body(&stale), TEST_NOW + 5).await,
        StatusCode::CONFLICT,
        "Brain changed after its deletion preview",
    )
    .await;
    let fresh: BrainDeletionPreviewResponse =
        read_json(preview(&router, &admin, "acme", TEST_NOW + 6).await).await;
    assert_eq!(
        (fresh.folder_count, fresh.page_count),
        (first.folder_count, first.page_count)
    );
    assert_ne!(fresh.state_digest, first.state_digest);

    let event = delete_event(&admin, "acme", "delete-acme", Some(&fresh.state_digest));
    let deleted = delete(&router, &admin, "acme", &delete_body(&event), TEST_NOW + 7).await;
    assert_eq!(deleted.status(), StatusCode::OK);
    let deleted: BrainDeleteResponse = read_json(deleted).await;
    assert!(!deleted.duplicate);
    assert_eq!(deleted.receipt.brain_id, "acme");
    assert_eq!(deleted.receipt.deletion_event_id, event.id.to_hex());
    assert_eq!(deleted.receipt.actor_npub, npub(&admin));
    assert_eq!(deleted.receipt.state_digest, fresh.state_digest);
    assert_eq!(
        (deleted.receipt.folder_count, deleted.receipt.page_count),
        (2, 1)
    );
    let notice = loop {
        let update = updates.recv().await.unwrap();
        if update.reason == BrainUpdateReason::BrainDeleted {
            break update;
        }
    };
    assert_eq!(notice.brain_id, "acme");
    assert_eq!(notice.reason, BrainUpdateReason::BrainDeleted);
    assert_eq!(
        notice.notify_npubs.iter().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([npub(&admin), npub(&member)])
    );

    // An exact retry (same signed deletion, fresh request signature) answers
    // the same receipt and changes nothing.
    let retry = delete(&router, &admin, "acme", &delete_body(&event), TEST_NOW + 8).await;
    assert_eq!(retry.status(), StatusCode::OK);
    let retry: BrainDeleteResponse = read_json(retry).await;
    assert!(retry.duplicate);
    assert_eq!(retry.receipt, deleted.receipt);
    assert!(updates.try_recv().is_err());

    // Former readers learn the Brain is gone; a new deletion attempt too.
    let deleted_at = deleted.receipt.deleted_at.as_str();
    let again = delete_event(
        &admin,
        "acme",
        "delete-acme-again",
        Some(&fresh.state_digest),
    );
    assert_gone(
        deleted_at,
        delete(&router, &admin, "acme", &delete_body(&again), TEST_NOW + 9).await,
    )
    .await;
    assert_gone(
        deleted_at,
        get_metadata(router.clone(), &member, "acme", TEST_NOW + 10).await,
    )
    .await;
    assert_gone(
        deleted_at,
        authed_request(
            router.clone(),
            &member,
            "GET",
            "/v1/brains/acme/sync/records?after=0",
            None,
            TEST_NOW + 11,
        )
        .await,
    )
    .await;
    assert_gone(
        deleted_at,
        put_page(&router, &admin, "acme", 3, TEST_NOW + 12).await,
    )
    .await;

    // Everyone else gets exactly the unknown-Brain answer.
    let unknown = get_metadata(router.clone(), &outsider, "never-existed", TEST_NOW + 13).await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let unknown = read_text(unknown).await;
    let deleted_for_outsider = get_metadata(router.clone(), &outsider, "acme", TEST_NOW + 14).await;
    assert_eq!(deleted_for_outsider.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        read_text(deleted_for_outsider)
            .await
            .replace("acme", "never-existed"),
        unknown
    );
    let outsider_event = delete_event(
        &outsider,
        "acme",
        "delete-outsider",
        Some(&fresh.state_digest),
    );
    assert_eq!(
        delete(
            &router,
            &outsider,
            "acme",
            &delete_body(&outsider_event),
            TEST_NOW + 15
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    // The Brain left every list; its id cannot be created again, and the
    // refusal matches creating over a live Brain.
    let listed = authed_request(
        router.clone(),
        &admin,
        "GET",
        "/v1/brains",
        None,
        TEST_NOW + 16,
    )
    .await;
    let listed: VisibleBrainsResponse = read_json(listed).await;
    assert_eq!(
        listed
            .brains
            .iter()
            .map(|brain| brain.brain_id.as_str())
            .collect::<Vec<_>>(),
        vec!["content-brain"]
    );
    for (brain_id, at) in [("acme", TEST_NOW + 17), ("content-brain", TEST_NOW + 18)] {
        let recreate = post_brain(
            router.clone(),
            &outsider,
            &create_brain_body(brain_id, "organization"),
            at,
            None,
            None,
            None,
        )
        .await;
        assert_error(
            recreate,
            StatusCode::CONFLICT,
            &format!("duplicate id for brain_id: {brain_id}"),
        )
        .await;
    }
}

#[tokio::test]
async fn active_mounts_in_either_direction_block_deletion() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let (state, router) = deletion_router(&admin, &member).await;
    let admin_id = UserId::new(npub(&admin)).unwrap();
    {
        let mut store = state.store.lock().unwrap();
        store
            .create_folder(
                &BrainId::new("content-brain").unwrap(),
                &Folder {
                    id: FolderId::new("shared-in").unwrap(),
                    name: DisplayName::new("folder_name", "shared-in").unwrap(),
                    role: FolderRole::Folder,
                    access: FolderAccessMode::AdminOnly,
                    parent_folder_id: None,
                    path: SafeRelativePath::new("folder_path", "shared-in").unwrap(),
                    current_key_version: 1,
                },
                &BTreeSet::new(),
                &[grant_metadata(
                    "grant-shared-in-admin",
                    "shared-in",
                    &admin_id,
                )],
            )
            .unwrap();
        for (offer, source, folder, destination) in [
            ("offer-out", "acme", "restricted", "content-brain"),
            ("offer-in", "content-brain", "shared-in", "acme"),
        ] {
            let offer_grant = grant_metadata(&format!("grant-{offer}"), folder, &admin_id);
            store
                .create_shared_folder_invitation(
                    &BrainId::new(source).unwrap(),
                    &FolderId::new(folder).unwrap(),
                    &BrainId::new(destination).unwrap(),
                    offer,
                    &admin_id,
                    &admin_id,
                    &format!("/v1/mount-offers/{offer}/accept"),
                    &offer_grant,
                    &format_unix_timestamp(TEST_NOW + 86_400).unwrap(),
                    &test_rfc3339(),
                )
                .unwrap();
            let record = folder_key_grant_sync_record(&offer_grant).unwrap();
            store
                .accept_shared_folder_invitation(
                    offer,
                    &admin_id,
                    &format!("mount-{offer}"),
                    &format!("projection-{offer}"),
                    &[],
                    &[record],
                    &test_rfc3339(),
                )
                .unwrap();
        }
    }
    let blocked: BrainDeletionPreviewResponse =
        read_json(preview(&router, &admin, "acme", TEST_NOW).await).await;
    assert!(!blocked.deletable);
    assert_eq!(
        blocked
            .blocking_mounts
            .iter()
            .map(|mount| (
                mount.connection_id.as_str(),
                mount.direction.as_str(),
                mount.source_brain_id.as_str(),
                mount.destination_brain_id.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("mount-offer-in", "incoming", "content-brain", "acme"),
            ("mount-offer-out", "outgoing", "acme", "content-brain"),
        ]
    );
    let event = delete_event(
        &admin,
        "acme",
        "delete-blocked",
        Some(&blocked.state_digest),
    );
    assert_error(
        delete(&router, &admin, "acme", &delete_body(&event), TEST_NOW + 1).await,
        StatusCode::CONFLICT,
        "active cross-Brain Mount(s) (mount-offer-in, mount-offer-out)",
    )
    .await;
    assert_eq!(
        get_metadata(router.clone(), &admin, "acme", TEST_NOW + 2)
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_confirmation_signed_too_long_ago_deletes_nothing() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let (state, router) = deletion_router(&admin, &member).await;
    // The event is signed at TEST_NOW; the server clock moves past its window.
    let event_digest: BrainDeletionPreviewResponse =
        read_json(preview(&router, &admin, "acme", TEST_NOW).await).await;
    let signed = delete_event(
        &admin,
        "acme",
        "delete-expired",
        Some(&event_digest.state_digest),
    );
    state.advance_auth_clock(BRAIN_DELETION_CONFIRMATION_MAX_AGE_SECONDS + 5);
    let later = TEST_NOW + BRAIN_DELETION_CONFIRMATION_MAX_AGE_SECONDS + 5;
    let current: BrainDeletionPreviewResponse =
        read_json(preview(&router, &admin, "acme", later).await).await;
    assert_eq!(current.state_digest, event_digest.state_digest);
    assert_error(
        delete(&router, &admin, "acme", &delete_body(&signed), later + 1).await,
        StatusCode::CONFLICT,
        "Brain deletion confirmation expired",
    )
    .await;
    assert_eq!(
        get_metadata(router.clone(), &admin, "acme", later + 2)
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn personal_brains_cannot_be_deleted_by_their_owner_or_agent() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let state = ServerState::new(BrainStore::open_in_memory().unwrap(), TEST_BASE_URL)
        .with_auth_clock(TEST_NOW, 60);
    let personal = bootstrap_personal_brain("personal-owner", "Personal", npub(&owner)).unwrap();
    state
        .store
        .lock()
        .unwrap()
        .create_personal_brain_bootstrap(
            &personal,
            &[],
            &UserId::new(npub(&agent)).unwrap(),
            &UserId::new(npub(&owner)).unwrap(),
            &test_rfc3339(),
        )
        .unwrap();
    let router = router_with_state(state);
    for (keys, at) in [(&owner, TEST_NOW), (&agent, TEST_NOW + 1)] {
        assert_error(
            preview(&router, keys, "personal-owner", at).await,
            StatusCode::BAD_REQUEST,
            "Personal Brains cannot be deleted",
        )
        .await;
        let digest = format!("sha256:{}", "0".repeat(64));
        let event = delete_event(keys, "personal-owner", "delete-personal", Some(&digest));
        assert_error(
            delete(
                &router,
                keys,
                "personal-owner",
                &delete_body(&event),
                at + 10,
            )
            .await,
            StatusCode::BAD_REQUEST,
            "Personal Brains cannot be deleted",
        )
        .await;
    }
    assert_eq!(
        get_metadata(router.clone(), &owner, "personal-owner", TEST_NOW + 20)
            .await
            .status(),
        StatusCode::OK
    );
}

async fn assert_gone(deleted_at: &str, response: axum::response::Response) {
    assert_eq!(response.status(), StatusCode::GONE);
    let body: DeletedBrainResponse = read_json(response).await;
    assert_eq!(body.code, "brain_deleted");
    assert_eq!(body.brain_deleted.brain_id, "acme");
    assert_eq!(body.brain_deleted.deleted_at, deleted_at);
}

fn grant_metadata(id: &str, folder_id: &str, recipient: &UserId) -> FolderKeyGrantMetadata {
    FolderKeyGrantMetadata {
        id: id.to_owned(),
        folder_id: FolderId::new(folder_id).unwrap(),
        key_version: 1,
        issuer_npub: recipient.clone(),
        recipient_npub: recipient.clone(),
        format: "NIP-59".to_owned(),
        wrapped_event_json: gift_wrap_event_json(recipient.as_str()),
        access_change_event_json: Some(r#"{"kind":30078}"#.to_owned()),
        created_at: test_rfc3339(),
    }
}
