//! Folder Key rotation and stale-holder revocation through the signed router.
//! Every identity here is synthetic.

use super::*;

const REPORT_BODY_LIMIT: usize = 1024 * 1024;
const ROTATE_PATH: &str = "/v1/admin/brains/acme/folders/restricted/key-rotations";

struct Cast {
    admin: Keys,
    demoted: Keys,
    member: Keys,
    guest: Keys,
    outsider: Keys,
}

/// `acme` with a restricted Folder holding one page. `demoted` received the
/// current wrap as an admin and was then demoted, `guest` holds explicit
/// Folder access, and `member` is an ordinary Member with no Folder access.
async fn demoted_admin_brain() -> (ServerState, Router, Cast) {
    let cast = Cast {
        admin: Keys::generate(),
        demoted: Keys::generate(),
        member: Keys::generate(),
        guest: Keys::generate(),
        outsider: Keys::generate(),
    };
    let state = ServerState::new(BrainStore::open_in_memory().unwrap(), TEST_BASE_URL)
        .with_auth_clock(TEST_NOW, 60);
    let router = router_with_state(state.clone());
    let created = post_brain(
        router.clone(),
        &cast.admin,
        &create_brain_body("acme", "organization"),
        TEST_NOW,
        None,
        None,
        None,
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    add_test_org_folders(&router, &cast.admin).await;
    let brain = BrainId::new("acme").unwrap();
    {
        let mut store = state.store.lock().unwrap();
        for keys in [&cast.member, &cast.demoted] {
            store
                .add_member(&brain, &UserId::new(npub(keys)).unwrap())
                .unwrap();
        }
        store
            .add_admin(&brain, &UserId::new(npub(&cast.demoted)).unwrap())
            .unwrap();
    }
    grant_restricted(&router, &cast.admin, &cast.demoted, TEST_NOW + 1).await;
    grant_restricted(&router, &cast.admin, &cast.guest, TEST_NOW + 2).await;
    let page = authed_request(
        router.clone(),
        &cast.admin,
        "PUT",
        "/v1/brains/acme/folders/restricted/objects/obj_000000000001",
        Some(object_write_body(
            &cast.admin,
            RevisionFixture {
                brain_id: "acme",
                folder_id: "restricted",
                object_id: "obj_000000000001",
                operation: FolderObjectOperation::Create,
                revision: 1,
                base_revision: None,
                key_version: 1,
                content: "restricted page",
                nonce: 21,
                record_type: false,
            },
        )),
        TEST_NOW + 3,
    )
    .await;
    assert_eq!(page.status(), StatusCode::OK);
    {
        let mut store = state.store.lock().unwrap();
        store
            .remove_admin(&brain, &UserId::new(npub(&cast.demoted)).unwrap())
            .unwrap();
    }
    (state, router, cast)
}

async fn grant_restricted(router: &Router, admin: &Keys, target: &Keys, at: u64) {
    let target = npub(target);
    let change_id = format!("grant-restricted-{}", &target[target.len() - 8..]);
    let response = authed_request(
        router.clone(),
        admin,
        "PUT",
        &format!("/v1/admin/brains/acme/folders/restricted/access/{target}"),
        Some(
            serde_json::json!({
                "grant": folder_key_grant_value(&change_id, 1, &target),
                "accessChangeEvent": admin_event(admin, "acme", &change_id,
                    AdminAccessAction::GrantFolderAccess, Some("restricted"), Some(&target), Some(1)),
            })
            .to_string(),
        ),
        at,
    )
    .await;
    let status = response.status();
    let text = read_text(response).await;
    assert_eq!(status, StatusCode::OK, "{text}");
}

/// Rotation material for `restricted` at `new_key_version`, wrapped for
/// `recipients` and signed by `signer` with the given event binding.
#[allow(clippy::too_many_arguments)]
fn rotation_body(
    signer: &Keys,
    change_id: &str,
    action: AdminAccessAction,
    event_folder: &str,
    event_target: Option<&str>,
    event_key_version: u32,
    new_key_version: u32,
    recipients: &[String],
) -> String {
    let grants = recipients
        .iter()
        .map(|recipient| {
            folder_key_grant_value(
                &format!("{change_id}-{}", &recipient[recipient.len() - 8..]),
                new_key_version,
                recipient,
            )
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "newKeyVersion": new_key_version,
        "grants": grants,
        "reencryptedRecords": [rotation_object_value(
            signer,
            "acme",
            "restricted",
            "obj_000000000001",
            u64::from(new_key_version),
            Some(u64::from(new_key_version) - 1),
            new_key_version,
            "reencrypted restricted page",
            22,
        )],
        "accessChangeEvent": admin_event(
            signer,
            "acme",
            change_id,
            action,
            Some(event_folder),
            event_target,
            Some(event_key_version),
        ),
    })
    .to_string()
}

fn admin_rotation_body(cast: &Cast, change_id: &str) -> String {
    rotation_body(
        &cast.admin,
        change_id,
        AdminAccessAction::RotateFolderKey,
        "restricted",
        None,
        2,
        2,
        &[npub(&cast.admin), npub(&cast.guest)],
    )
}

fn restricted_key_version(state: &ServerState) -> u32 {
    let store = state.store.lock().unwrap();
    let stored = store.load_brain(&BrainId::new("acme").unwrap()).unwrap();
    stored
        .brain
        .folders
        .iter()
        .find(|folder| folder.id.as_str() == "restricted")
        .unwrap()
        .current_key_version
}

/// Per Folder, the keys the access report lists as `revocationIncomplete`.
async fn report_revocation_incomplete(
    router: &Router,
    admin: &Keys,
    at: u64,
) -> BTreeMap<String, BTreeSet<String>> {
    report_revocation_incomplete_for(router, admin, "acme", at).await
}

async fn report_revocation_incomplete_for(
    router: &Router,
    admin: &Keys,
    brain: &str,
    at: u64,
) -> BTreeMap<String, BTreeSet<String>> {
    let response = authed_request(
        router.clone(),
        admin,
        "GET",
        &format!("/v1/brains/{brain}/access-report"),
        None,
        at,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let report = read_json_with_limit::<serde_json::Value>(response, REPORT_BODY_LIMIT).await;
    let mut by_folder = BTreeMap::<String, BTreeSet<String>>::new();
    for row in report["identities"].as_array().unwrap() {
        for folder in row["revocationIncomplete"].as_array().unwrap() {
            by_folder
                .entry(folder.as_str().unwrap().to_owned())
                .or_default()
                .insert(row["npub"].as_str().unwrap().to_owned());
        }
    }
    by_folder
}

/// The store predicate behind revoke and rotation, per Folder.
fn store_stale_holders(state: &ServerState) -> BTreeMap<String, BTreeSet<String>> {
    store_stale_holders_for(state, "acme")
}

fn store_stale_holders_for(state: &ServerState, brain: &str) -> BTreeMap<String, BTreeSet<String>> {
    let store = state.store.lock().unwrap();
    let stored = store.load_brain(&BrainId::new(brain).unwrap()).unwrap();
    stored
        .brain
        .folders
        .iter()
        .filter_map(|folder| {
            let holders = finite_brain_store::stale_folder_key_holders(&stored, folder)
                .unwrap()
                .into_iter()
                .map(|holder| holder.to_string())
                .collect::<BTreeSet<_>>();
            (!holders.is_empty()).then(|| (folder.id.to_string(), holders))
        })
        .collect()
}

#[tokio::test]
async fn rotation_cuts_off_a_demoted_admin_and_the_report_agrees() {
    let (state, router, cast) = demoted_admin_brain().await;
    let before = report_revocation_incomplete(&router, &cast.admin, TEST_NOW + 4).await;
    assert_eq!(
        before,
        BTreeMap::from([(
            "restricted".to_owned(),
            BTreeSet::from([npub(&cast.demoted)])
        )])
    );
    assert_eq!(store_stale_holders(&state), before);

    let mut updates = state.brain_updates.subscribe();
    let body = admin_rotation_body(&cast, "rotate-restricted");
    let response = authed_request(
        router.clone(),
        &cast.admin,
        "POST",
        ROTATE_PATH,
        Some(body.clone()),
        TEST_NOW + 5,
    )
    .await;
    let status = response.status();
    let text = read_text(response).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let rotated: RotateFolderKeyResponse = serde_json::from_str(&text).unwrap();
    let mut recipients = vec![npub(&cast.admin), npub(&cast.guest)];
    recipients.sort();
    assert_eq!(
        rotated.rotation,
        FolderKeyRotationReceipt {
            folder_id: "restricted".to_owned(),
            previous_key_version: 1,
            new_key_version: 2,
            recipient_npubs: recipients,
            cut_off_npubs: vec![npub(&cast.demoted)],
        }
    );
    let restricted = rotated
        .metadata
        .folders
        .iter()
        .find(|folder| folder.id == "restricted")
        .unwrap();
    assert_eq!(restricted.current_key_version, 2);
    assert_eq!(restricted.access_user_ids, vec![npub(&cast.guest)]);
    // The response still reads as plain Brain metadata for older clients.
    let plain: BrainMetadataResponse = serde_json::from_str(&text).unwrap();
    assert_eq!(plain.brain_id, "acme");
    let update = updates.try_recv().unwrap();
    assert_eq!(update.reason, BrainUpdateReason::AccessUpdated);
    assert!(update.notify_npubs.is_empty());

    assert!(
        report_revocation_incomplete(&router, &cast.admin, TEST_NOW + 6)
            .await
            .is_empty()
    );
    assert!(store_stale_holders(&state).is_empty());

    // Replaying the same signed rotation finds the version already moved.
    let replay = authed_request(
        router.clone(),
        &cast.admin,
        "POST",
        ROTATE_PATH,
        Some(body),
        TEST_NOW + 7,
    )
    .await;
    assert_error(replay, StatusCode::CONFLICT, "Key version changed").await;
    assert_eq!(restricted_key_version(&state), 2);
}

#[tokio::test]
async fn rotation_requires_brain_admin_standing() {
    let (state, router, cast) = demoted_admin_brain().await;
    let recipients = [npub(&cast.admin), npub(&cast.guest)];
    for (index, keys) in [&cast.member, &cast.guest, &cast.demoted, &cast.outsider]
        .into_iter()
        .enumerate()
    {
        let body = rotation_body(
            keys,
            &format!("not-admin-{index}"),
            AdminAccessAction::RotateFolderKey,
            "restricted",
            None,
            2,
            2,
            &recipients,
        );
        let response = authed_request(
            router.clone(),
            keys,
            "POST",
            ROTATE_PATH,
            Some(body),
            TEST_NOW + 5,
        )
        .await;
        assert_error(
            response,
            StatusCode::FORBIDDEN,
            "brain admin access required",
        )
        .await;
    }
    assert_eq!(restricted_key_version(&state), 1);
}

#[tokio::test]
async fn rotation_rejects_events_bound_to_another_change() {
    let (state, router, cast) = demoted_admin_brain().await;
    let recipients = [npub(&cast.admin), npub(&cast.guest)];
    let demoted = npub(&cast.demoted);
    let cases = [
        (
            "wrong-folder",
            AdminAccessAction::RotateFolderKey,
            "getting-started",
            None,
            2,
        ),
        (
            "wrong-version",
            AdminAccessAction::RotateFolderKey,
            "restricted",
            None,
            3,
        ),
        (
            "wrong-action",
            AdminAccessAction::RemoveFolderAccess,
            "restricted",
            Some(demoted.as_str()),
            2,
        ),
        (
            "unexpected-target",
            AdminAccessAction::RotateFolderKey,
            "restricted",
            Some(demoted.as_str()),
            2,
        ),
    ];
    for (change_id, action, folder, target, event_version) in cases {
        let body = rotation_body(
            &cast.admin,
            change_id,
            action,
            folder,
            target,
            event_version,
            2,
            &recipients,
        );
        let response = authed_request(
            router.clone(),
            &cast.admin,
            "POST",
            ROTATE_PATH,
            Some(body),
            TEST_NOW + 5,
        )
        .await;
        assert_error(
            response,
            StatusCode::BAD_REQUEST,
            "access-change payload fields differ",
        )
        .await;
    }
    // Wrapping for the cut-off key, or skipping a required one, is refused.
    for (change_id, wrapped) in [
        (
            "rewrap-demoted",
            vec![npub(&cast.admin), npub(&cast.guest), demoted.clone()],
        ),
        ("skip-guest", vec![npub(&cast.admin)]),
    ] {
        let response = authed_request(
            router.clone(),
            &cast.admin,
            "POST",
            ROTATE_PATH,
            Some(rotation_body(
                &cast.admin,
                change_id,
                AdminAccessAction::RotateFolderKey,
                "restricted",
                None,
                2,
                2,
                &wrapped,
            )),
            TEST_NOW + 5,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(restricted_key_version(&state), 1);
}

#[tokio::test]
async fn revoke_accepts_a_stale_holder_and_refuses_role_or_unknown_keys() {
    let (state, router, cast) = demoted_admin_brain().await;
    let revoke = |target: &Keys, change_id: &str| {
        let target = npub(target);
        (
            format!("/v1/admin/brains/acme/folders/restricted/access/{target}"),
            rotation_body(
                &cast.admin,
                change_id,
                AdminAccessAction::RemoveFolderAccess,
                "restricted",
                Some(&target),
                2,
                2,
                &[npub(&cast.admin), npub(&cast.guest)],
            ),
        )
    };

    let (path, body) = revoke(&cast.admin, "revoke-admin");
    let response = authed_request(
        router.clone(),
        &cast.admin,
        "DELETE",
        &path,
        Some(body),
        TEST_NOW + 5,
    )
    .await;
    assert_error(response, StatusCode::BAD_REQUEST, "through its Brain role").await;
    let (path, body) = revoke(&cast.member, "revoke-member");
    let response = authed_request(
        router.clone(),
        &cast.admin,
        "DELETE",
        &path,
        Some(body),
        TEST_NOW + 5,
    )
    .await;
    assert_error(
        response,
        StatusCode::BAD_REQUEST,
        "does not currently have access",
    )
    .await;
    assert_eq!(restricted_key_version(&state), 1);

    let (path, body) = revoke(&cast.demoted, "revoke-demoted");
    let response = authed_request(
        router.clone(),
        &cast.admin,
        "DELETE",
        &path,
        Some(body),
        TEST_NOW + 6,
    )
    .await;
    let status = response.status();
    let text = read_text(response).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let metadata: BrainMetadataResponse = serde_json::from_str(&text).unwrap();
    let restricted = metadata
        .folders
        .iter()
        .find(|folder| folder.id == "restricted")
        .unwrap();
    assert_eq!(restricted.current_key_version, 2);
    assert_eq!(restricted.access_user_ids, vec![npub(&cast.guest)]);
    assert!(
        report_revocation_incomplete(&router, &cast.admin, TEST_NOW + 7)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn revoke_refuses_an_explicit_row_whose_key_was_later_promoted() {
    let (state, router, cast) = demoted_admin_brain().await;
    grant_restricted(&router, &cast.admin, &cast.member, TEST_NOW + 4).await;
    let member = npub(&cast.member);
    {
        let mut store = state.store.lock().unwrap();
        store
            .add_admin(
                &BrainId::new("acme").unwrap(),
                &UserId::new(member.clone()).unwrap(),
            )
            .unwrap();
    }
    let response = authed_request(
        router.clone(),
        &cast.admin,
        "DELETE",
        &format!("/v1/admin/brains/acme/folders/restricted/access/{member}"),
        Some(rotation_body(
            &cast.admin,
            "revoke-promoted",
            AdminAccessAction::RemoveFolderAccess,
            "restricted",
            Some(&member),
            2,
            2,
            &[npub(&cast.admin), npub(&cast.guest), member.clone()],
        )),
        TEST_NOW + 5,
    )
    .await;
    assert_error(response, StatusCode::BAD_REQUEST, "through its Brain role").await;
    assert_eq!(restricted_key_version(&state), 1);
    let store = state.store.lock().unwrap();
    let stored = store.load_brain(&BrainId::new("acme").unwrap()).unwrap();
    assert!(
        stored.folder_access[&FolderId::new("restricted").unwrap()]
            .contains(&UserId::new(member).unwrap())
    );
}

fn stored_grant(
    folder: &str,
    issuer: &Keys,
    recipient: &Keys,
) -> finite_brain_store::FolderKeyGrantMetadata {
    finite_brain_store::FolderKeyGrantMetadata {
        id: format!("{folder}-{}", &npub(recipient)[60..]),
        folder_id: FolderId::new(folder).unwrap(),
        key_version: 1,
        issuer_npub: UserId::new(npub(issuer)).unwrap(),
        recipient_npub: UserId::new(npub(recipient)).unwrap(),
        format: "NIP-59".to_owned(),
        wrapped_event_json: gift_wrap_event_json(&npub(recipient)),
        access_change_event_json: None,
        created_at: test_rfc3339(),
    }
}

/// The report and the store predicate share one stale-holder rule only under
/// current invariants, so pin them on a Personal Brain too: the owner and
/// Personal Agent are entitled everywhere, while a key whose Guest row was
/// dropped and a key holding a stray wrap on an owner Folder are stale.
#[tokio::test]
async fn personal_brain_report_and_store_agree_on_stale_holders() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let guest = Keys::generate();
    let stray = Keys::generate();
    let scratch = tempfile::TempDir::new().unwrap();
    let database = scratch.path().join("personal-equivalence.sqlite3");
    let mut store = BrainStore::open(&database).unwrap();
    let output = bootstrap_personal_brain("personal", "Personal Brain", npub(&owner)).unwrap();
    store
        .create_personal_brain_bootstrap(
            &output,
            &[],
            &UserId::new(npub(&agent)).unwrap(),
            &UserId::new(npub(&owner)).unwrap(),
            &test_rfc3339(),
        )
        .unwrap();
    let brain = BrainId::new("personal").unwrap();
    for (id, access, guests, recipients) in [
        (
            "journal",
            FolderAccessMode::Owner,
            vec![],
            vec![&owner, &agent],
        ),
        (
            "shared",
            FolderAccessMode::Restricted,
            vec![&guest],
            vec![&owner, &agent, &guest],
        ),
    ] {
        let folder = Folder {
            id: FolderId::new(id).unwrap(),
            name: DisplayName::new("folder_name", id).unwrap(),
            role: FolderRole::Folder,
            access,
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", id).unwrap(),
            current_key_version: 1,
        };
        let access_user_ids = guests
            .iter()
            .map(|keys| UserId::new(npub(keys)).unwrap())
            .collect::<BTreeSet<_>>();
        let grants = recipients
            .iter()
            .map(|recipient| stored_grant(id, &owner, recipient))
            .collect::<Vec<_>>();
        store
            .create_folder(&brain, &folder, &access_user_ids, &grants)
            .unwrap();
    }
    drop(store);
    {
        // Synthetic drift no current API produces: drop the Guest row and
        // copy the owner's journal wrap to an unrelated key.
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        connection
            .execute(
                "DELETE FROM folder_access WHERE brain_id = 'personal' AND user_id = ?1",
                rusqlite::params![npub(&guest)],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO folder_key_grants (id, brain_id, folder_id, key_version, issuer_npub, recipient_npub, format, wrapped_event_json, access_change_event_json, created_at, delegated_by_npub, origin_kind, origin_ref, roster_revision) SELECT 'journal-stray', brain_id, folder_id, key_version, issuer_npub, ?1, format, wrapped_event_json, access_change_event_json, created_at, delegated_by_npub, origin_kind, origin_ref, roster_revision FROM folder_key_grants WHERE brain_id = 'personal' AND folder_id = 'journal' AND recipient_npub = ?2",
                rusqlite::params![npub(&stray), npub(&owner)],
            )
            .unwrap();
    }
    let state = ServerState::new(BrainStore::open(&database).unwrap(), TEST_BASE_URL)
        .with_auth_clock(TEST_NOW, 60);
    let router = router_with_state(state.clone());

    let expected = BTreeMap::from([
        ("journal".to_owned(), BTreeSet::from([npub(&stray)])),
        ("shared".to_owned(), BTreeSet::from([npub(&guest)])),
    ]);
    assert_eq!(store_stale_holders_for(&state, "personal"), expected);
    assert_eq!(
        report_revocation_incomplete_for(&router, &owner, "personal", TEST_NOW).await,
        expected
    );
}

#[tokio::test]
async fn oversized_revoke_is_refused_before_resolving_the_target() {
    let admin = Keys::generate();
    let lookups = Arc::new(Mutex::new(0usize));
    let counted = lookups.clone();
    let mut state = ServerState::new(BrainStore::open_in_memory().unwrap(), TEST_BASE_URL)
        .with_auth_clock(TEST_NOW, 60);
    state.nip05_fetcher = Arc::new(move |_| {
        *counted.lock().unwrap() += 1;
        Err("synthetic NIP-05 lookup".to_owned())
    });
    let router = router_with_state(state);
    let grants = (0..=finite_brain_core::MAX_FOLDER_ACCESS_REMOVAL_GRANTS)
        .map(|index| {
            serde_json::json!({
                "id": format!("oversized-{index}"),
                "keyVersion": 2,
                "recipientNpub": "unchecked",
                "wrappedEventJson": "{}"
            })
        })
        .collect::<Vec<_>>();
    let response = authed_request(
        router,
        &admin,
        "DELETE",
        "/v1/admin/brains/acme/folders/restricted/access/someone@example.test",
        Some(
            serde_json::json!({
                "newKeyVersion": 2,
                "grants": grants,
                "reencryptedRecords": [],
                "accessChangeEvent": {}
            })
            .to_string(),
        ),
        TEST_NOW,
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(*lookups.lock().unwrap(), 0);
}

/// Add a padding field so a valid body crosses the ordinary 1 MiB cap.
fn padded(body: String, bytes: usize) -> String {
    let mut value: serde_json::Value = serde_json::from_str(&body).unwrap();
    value["padding"] = serde_json::Value::String("x".repeat(bytes));
    value.to_string()
}

#[tokio::test]
async fn rotation_routes_accept_bodies_above_the_ordinary_cap_up_to_their_own_limit() {
    let (state, router, cast) = demoted_admin_brain().await;
    let over_ordinary = MAX_REQUEST_BODY_BYTES + 1024;

    // DELETE access, with a body over 1 MiB, revokes the stale holder.
    let demoted = npub(&cast.demoted);
    let revoke = padded(
        rotation_body(
            &cast.admin,
            "revoke-padded",
            AdminAccessAction::RemoveFolderAccess,
            "restricted",
            Some(&demoted),
            2,
            2,
            &[npub(&cast.admin), npub(&cast.guest)],
        ),
        over_ordinary,
    );
    assert!(revoke.len() > MAX_REQUEST_BODY_BYTES);
    let response = authed_request(
        router.clone(),
        &cast.admin,
        "DELETE",
        &format!("/v1/admin/brains/acme/folders/restricted/access/{demoted}"),
        Some(revoke),
        TEST_NOW + 5,
    )
    .await;
    let status = response.status();
    assert_eq!(status, StatusCode::OK, "{}", read_text(response).await);

    // POST key-rotations, with a body over 1 MiB, rotates again.
    let rotate = padded(
        rotation_body(
            &cast.admin,
            "rotate-padded",
            AdminAccessAction::RotateFolderKey,
            "restricted",
            None,
            3,
            3,
            &[npub(&cast.admin), npub(&cast.guest)],
        ),
        over_ordinary,
    );
    let response = authed_request(
        router.clone(),
        &cast.admin,
        "POST",
        ROTATE_PATH,
        Some(rotate),
        TEST_NOW + 6,
    )
    .await;
    let status = response.status();
    assert_eq!(status, StatusCode::OK, "{}", read_text(response).await);
    assert_eq!(restricted_key_version(&state), 3);

    // Member removal reads its body past the ordinary cap (then rejects the
    // malformed JSON), while the sibling grant method keeps the 1 MiB cap.
    let member = npub(&cast.member);
    let oversized_garbage = "x".repeat(over_ordinary);
    let removal = authed_request(
        router.clone(),
        &cast.admin,
        "DELETE",
        &format!("/v1/admin/brains/acme/members/{member}"),
        Some(oversized_garbage.clone()),
        TEST_NOW + 7,
    )
    .await;
    assert_error(
        removal,
        StatusCode::BAD_REQUEST,
        "invalid JSON request body",
    )
    .await;
    let grant = authed_request(
        router,
        &cast.admin,
        "PUT",
        &format!("/v1/admin/brains/acme/folders/restricted/access/{member}"),
        Some(oversized_garbage),
        TEST_NOW + 7,
    )
    .await;
    assert_eq!(grant.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn rotation_routes_refuse_bodies_over_their_limit() {
    let (state, router, cast) = demoted_admin_brain().await;
    let body = "x".repeat(finite_brain_core::MAX_FOLDER_ROTATION_REQUEST_BODY_BYTES + 1);
    let response = authed_request(
        router,
        &cast.admin,
        "POST",
        ROTATE_PATH,
        Some(body),
        TEST_NOW + 5,
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(restricted_key_version(&state), 1);
}

/// `demoted_admin_brain` plus a destination Brain that mounts `restricted`.
async fn mounted_brain() -> (ServerState, Router, Cast, Keys, String) {
    let (state, router, cast) = demoted_admin_brain().await;
    let destination_admin = Keys::generate();
    let created = post_brain(
        router.clone(),
        &destination_admin,
        &create_brain_body("dest", "organization"),
        TEST_NOW,
        None,
        None,
        None,
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let controller = npub(&destination_admin);
    let offer = authed_request(
        router.clone(),
        &cast.admin,
        "POST",
        "/v1/brains/acme/folders/restricted/mount-offers",
        Some(
            serde_json::json!({
                "destinationBrainId": "dest",
                "destinationControllerNpub": controller,
                "grant": folder_key_grant_value("grant-restricted-mount-v1", 1, &controller),
                "accessChangeEvent": admin_event(&cast.admin, "acme", "mount-offer-restricted",
                    AdminAccessAction::GrantFolderAccess, Some("restricted"), Some(&controller), Some(1)),
                "expiresAt": "2026-06-04T20:26:40Z",
            })
            .to_string(),
        ),
        TEST_NOW + 4,
    )
    .await;
    let status = offer.status();
    let offer: serde_json::Value = serde_json::from_str(&read_text(offer).await).unwrap();
    assert_eq!(status, StatusCode::OK, "{offer}");
    let accept = authed_request(
        router.clone(),
        &destination_admin,
        "POST",
        &format!("/v1/mount-offers/{}/accept", offer["id"].as_str().unwrap()),
        Some(String::new()),
        TEST_NOW + 5,
    )
    .await;
    let status = accept.status();
    let mount: serde_json::Value = serde_json::from_str(&read_text(accept).await).unwrap();
    assert_eq!(status, StatusCode::OK, "{mount}");
    let mount_id = mount["mountId"].as_str().unwrap().to_owned();
    (state, router, cast, destination_admin, mount_id)
}

fn oversized_mount_bodies() -> [(&'static str, String); 2] {
    let over = finite_brain_core::MAX_FOLDER_ACCESS_REMOVAL_GRANTS + 1;
    let grants = (0..over)
        .map(|index| {
            serde_json::json!({
                "id": format!("oversized-{index}"),
                "keyVersion": 2,
                "recipientNpub": "unchecked",
                "wrappedEventJson": "{}"
            })
        })
        .collect::<Vec<_>>();
    let records = (0..finite_brain_core::MAX_FOLDER_ACCESS_REMOVAL_RECORDS + 1)
        .map(|index| {
            serde_json::json!({
                "objectId": format!("obj_{index:012}"),
                "baseRevision": 1,
                "keyVersion": 2,
                "cipher": "AES-256-GCM",
                "ciphertext": "{}",
                "revisionEvent": {}
            })
        })
        .collect::<Vec<_>>();
    [
        (
            "grants",
            serde_json::json!({ "newKeyVersion": 2, "grants": grants, "reencryptedRecords": [] })
                .to_string(),
        ),
        (
            "records",
            serde_json::json!({ "newKeyVersion": 2, "grants": [], "reencryptedRecords": records })
                .to_string(),
        ),
    ]
}

#[tokio::test]
async fn mount_rotation_routes_bound_fan_out_before_lookup_or_store_reads() {
    let outsider = Keys::generate();
    let lookups = Arc::new(Mutex::new(0usize));
    let counted = lookups.clone();
    let mut state = ServerState::new(BrainStore::open_in_memory().unwrap(), TEST_BASE_URL)
        .with_auth_clock(TEST_NOW, 60);
    state.nip05_fetcher = Arc::new(move |_| {
        *counted.lock().unwrap() += 1;
        Err("synthetic NIP-05 lookup".to_owned())
    });
    let router = router_with_state(state);
    // The Mount does not exist, so a 413 proves fan-out ran before any store
    // read, and the NIP-05 participant proves it ran before identity lookup.
    for path in [
        "/v1/mounts/mount-missing",
        "/v1/mounts/mount-missing/participants/someone@example.test",
    ] {
        for (kind, body) in oversized_mount_bodies() {
            let response = authed_request(
                router.clone(),
                &outsider,
                "DELETE",
                path,
                Some(body),
                TEST_NOW,
            )
            .await;
            assert_eq!(
                response.status(),
                StatusCode::PAYLOAD_TOO_LARGE,
                "{path} oversized {kind}"
            );
        }
    }
    assert_eq!(*lookups.lock().unwrap(), 0);
}

#[tokio::test]
async fn mount_rotation_routes_refuse_an_unauthorized_caller_before_reading_material() {
    let (state, router, cast, destination_admin, mount_id) = mounted_brain().await;
    // A malformed grant would fail conversion; the authority refusal must
    // come first, so the caller learns nothing from the material.
    let body = serde_json::json!({
        "newKeyVersion": 2,
        "grants": [{
            "id": "malformed",
            "keyVersion": 2,
            "recipientNpub": npub(&cast.admin),
            "wrappedEventJson": "not a gift wrap"
        }],
        "reencryptedRecords": []
    })
    .to_string();
    let revoke = authed_request(
        router.clone(),
        &cast.member,
        "DELETE",
        &format!("/v1/mounts/{mount_id}"),
        Some(body.clone()),
        TEST_NOW + 6,
    )
    .await;
    assert_error(
        revoke,
        StatusCode::BAD_REQUEST,
        "mount revocation requires source or destination brain control",
    )
    .await;
    let participant = npub(&destination_admin);
    let remove = authed_request(
        router.clone(),
        &cast.member,
        "DELETE",
        &format!("/v1/mounts/{mount_id}/participants/{participant}"),
        Some(body),
        TEST_NOW + 6,
    )
    .await;
    assert_error(
        remove,
        StatusCode::BAD_REQUEST,
        "mount participant management requires destination brain control",
    )
    .await;
    let store = state.store.lock().unwrap();
    let connection = store.load_shared_folder_connection(&mount_id).unwrap();
    assert_eq!(connection.status, SharedFolderConnectionStatus::Active);
}

#[tokio::test]
async fn rotation_routes_admit_a_bounded_number_of_requests_at_once() {
    let (state, router, cast) = demoted_admin_brain().await;
    let held = state
        .rotation_request_permits
        .clone()
        .try_acquire_many_owned(MAX_CONCURRENT_ROTATION_REQUESTS as u32)
        .unwrap();
    let body = admin_rotation_body(&cast, "rotate-while-busy");
    let busy = authed_request(
        router.clone(),
        &cast.admin,
        "POST",
        ROTATE_PATH,
        Some(body.clone()),
        TEST_NOW + 5,
    )
    .await;
    assert_error(
        busy,
        StatusCode::SERVICE_UNAVAILABLE,
        "too many Folder Key rotation requests are in progress",
    )
    .await;
    assert_eq!(restricted_key_version(&state), 1);
    // The sibling grant method on a rotation path is not limited.
    grant_restricted(&router, &cast.admin, &cast.member, TEST_NOW + 5).await;

    drop(held);
    let response = authed_request(
        router,
        &cast.admin,
        "POST",
        ROTATE_PATH,
        Some(admin_rotation_body_for(
            &cast,
            "rotate-after-busy",
            &[npub(&cast.admin), npub(&cast.guest), npub(&cast.member)],
        )),
        TEST_NOW + 6,
    )
    .await;
    let status = response.status();
    assert_eq!(status, StatusCode::OK, "{}", read_text(response).await);
    assert_eq!(restricted_key_version(&state), 2);
}

fn admin_rotation_body_for(cast: &Cast, change_id: &str, recipients: &[String]) -> String {
    rotation_body(
        &cast.admin,
        change_id,
        AdminAccessAction::RotateFolderKey,
        "restricted",
        None,
        2,
        2,
        recipients,
    )
}

#[tokio::test]
async fn a_rotation_that_skips_versions_is_a_bad_request() {
    let (state, router, cast) = demoted_admin_brain().await;
    let response = authed_request(
        router,
        &cast.admin,
        "POST",
        ROTATE_PATH,
        Some(rotation_body(
            &cast.admin,
            "rotate-skip",
            AdminAccessAction::RotateFolderKey,
            "restricted",
            None,
            3,
            3,
            &[npub(&cast.admin), npub(&cast.guest)],
        )),
        TEST_NOW + 5,
    )
    .await;
    assert_error(response, StatusCode::BAD_REQUEST, "skips versions").await;
    assert_eq!(restricted_key_version(&state), 1);
}
