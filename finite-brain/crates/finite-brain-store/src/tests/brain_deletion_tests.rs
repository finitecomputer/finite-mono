//! Organization Brain deletion: the confirmed preview digest is rechecked in
//! the deleting transaction, every owned row goes, other Brains stay
//! byte-identical, and the permanent deletion record retires the Brain id.

use super::*;
use crate::brain_deletion::{
    BRAIN_DELETION_UNSCOPED_TABLES, brain_deletion_scope_tables, brain_scoped_rows,
};

const ADMIN: &str = "npub-admin";
const MEMBER: &str = "npub-member";
const GUEST: &str = "npub-guest";
const INVITEE: &str = "npub-invitee";
const SERVER: &str = "https://brain.example.test";
const NOW: &str = "2026-06-23T00:00:00.000Z";
const EXPIRES: &str = "2026-06-30T00:00:00.000Z";

fn v1() -> BrainId {
    BrainId::new("acme").unwrap()
}

fn content() -> BrainId {
    BrainId::new("content-brain").unwrap()
}

fn user(npub: &str) -> UserId {
    UserId::new(npub).unwrap()
}

fn folder_spec(id: &str, access: FolderAccessMode) -> Folder {
    Folder {
        id: FolderId::new(id).unwrap(),
        name: DisplayName::new("folder_name", id).unwrap(),
        role: FolderRole::Folder,
        access,
        parent_folder_id: None,
        path: SafeRelativePath::new("folder_path", id).unwrap(),
        current_key_version: 1,
    }
}

fn create_folder_in(store: &mut BrainStore, brain: &BrainId, id: &str, access: FolderAccessMode) {
    store
        .create_folder(
            brain,
            &folder_spec(id, access),
            &BTreeSet::new(),
            &[grant(&format!("grant-{brain}-{id}"), id, 1, ADMIN, ADMIN)],
        )
        .unwrap();
}

/// `acme` stands in for V1: three Folders (one nested), two live pages, a
/// Member, a Folder-limited guest, a pending invitation, an open Invite Token
/// and the history of an earlier Folder deletion. `content-brain` shares the
/// admin and Member and holds its own page.
fn populate(store: &mut BrainStore) {
    bootstrap_org_and_strategy_folder(store);
    store.add_member(&v1(), &user(MEMBER)).unwrap();
    store
        .grant_folder_access(
            &v1(),
            &FolderId::new("private-project").unwrap(),
            &user(GUEST),
            &grant("grant-private-guest", "private-project", 1, ADMIN, GUEST),
        )
        .unwrap();
    for (event, object) in [
        ("event-v1-page-1", "obj_000000000001"),
        ("event-v1-page-2", "obj_000000000002"),
    ] {
        store
            .submit_sync_record(&v1(), &revision_record(event, object, 1, None, "v1 page"))
            .unwrap();
    }
    store
        .create_brain_invitation(
            &v1(),
            "invitation-v1",
            &user(INVITEE),
            "code-v1",
            "/accept",
            &[],
            &user(ADMIN),
            EXPIRES,
            NOW,
        )
        .unwrap();
    store
        .create_brain_invite_token(
            &v1(),
            &"a".repeat(64),
            BrainInviteTokenRole::Member,
            &user(ADMIN),
            EXPIRES,
            NOW,
        )
        .unwrap();
    create_folder_in(store, &v1(), "archive", FolderAccessMode::AdminOnly);
    store
        .delete_folder_subtree(
            &v1(),
            &FolderId::new("archive").unwrap(),
            &user(ADMIN),
            1,
            "event-delete-archive",
            r#"{"recordType":"folder_subtree_tombstone"}"#,
            NOW,
            APP_SPECIFIC_KIND,
            None,
        )
        .unwrap();

    bootstrap_org_named(store, content().as_str(), "Content", ADMIN);
    create_folder_in(
        store,
        &content(),
        "content-notes",
        FolderAccessMode::AllMembers,
    );
    store.add_member(&content(), &user(MEMBER)).unwrap();
    store
        .submit_sync_record(
            &content(),
            &revision_record_for(
                "content-notes",
                "event-content-page",
                "obj_000000000101",
                1,
                None,
                "content page",
            ),
        )
        .unwrap();
}

fn request<'a>(
    brain_id: &'a BrainId,
    actor: &'a UserId,
    event_id: &'a str,
    digest: &'a str,
) -> BrainDeletionRequest<'a> {
    BrainDeletionRequest {
        brain_id,
        actor_npub: actor,
        deletion_event_id: event_id,
        deletion_event_json: r#"{"kind":30078}"#,
        expected_state_digest: digest,
        server_identity: SERVER,
        deleted_at: "2026-06-24T00:00:00Z",
        confirmation_current: true,
    }
}

fn preview(store: &BrainStore, brain_id: &BrainId) -> BrainDeletionPreview {
    store
        .brain_deletion_preview(brain_id, &user(ADMIN), SERVER)
        .unwrap()
}

#[test]
fn deletion_removes_owned_rows_retires_the_id_and_leaves_other_brains_identical() {
    let temp = TempDir::new().unwrap();
    let db = temp.path().join("deletion.sqlite3");
    let mut store = BrainStore::open(&db).unwrap();
    populate(&mut store);
    let admin = user(ADMIN);
    let content_before = brain_scoped_rows(&store.conn, &content());
    let aliases_before = store
        .conn
        .query_row("SELECT COUNT(*) FROM identity_aliases", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();

    let scope = preview(&store, &v1());
    assert_eq!(scope.name, "Acme");
    assert_eq!(scope.folder_count, 3);
    assert_eq!(scope.object_count, 2);
    assert_eq!(scope.member_count, 2);
    assert_eq!(scope.admin_count, 1);
    assert_eq!(scope.guest_count, 1);
    assert_eq!(scope.pending_invitation_count, 1);
    assert_eq!(scope.open_invite_token_count, 1);
    assert!(scope.blocking_mounts.is_empty());
    assert_eq!(scope.retired_mount_record_count, 0);
    assert!(scope.state_digest.starts_with(BRAIN_DELETION_DIGEST_PREFIX));
    assert_eq!(
        scope.state_digest.len(),
        BRAIN_DELETION_DIGEST_PREFIX.len() + 64
    );
    // The digest binds the server identity.
    assert_ne!(
        store
            .brain_deletion_preview(&v1(), &admin, "https://other.example.test")
            .unwrap()
            .state_digest,
        scope.state_digest
    );

    let outcome = store
        .delete_organization_brain(&request(
            &v1(),
            &admin,
            "event-delete-v1",
            &scope.state_digest,
        ))
        .unwrap();
    let BrainDeletionOutcome::Deleted { receipt, audience } = outcome else {
        panic!("first deletion must delete");
    };
    assert_eq!(
        audience.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([user(ADMIN), user(MEMBER), user(GUEST)])
    );
    assert_eq!(receipt.brain_id, v1());
    assert_eq!(receipt.deletion_event_id, "event-delete-v1");
    assert_eq!(receipt.actor_npub, admin);
    assert_eq!(receipt.state_digest, scope.state_digest);
    assert_eq!((receipt.folder_count, receipt.object_count), (3, 2));

    assert!(brain_scoped_rows(&store.conn, &v1()).is_empty());
    assert_eq!(brain_scoped_rows(&store.conn, &content()), content_before);
    assert_eq!(
        store
            .conn
            .query_row("SELECT COUNT(*) FROM identity_aliases", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        aliases_before
    );
    assert_eq!(
        store.load_brain(&v1()).unwrap_err(),
        StoreError::MissingBrain {
            brain_id: "acme".to_owned()
        }
    );
    let visible = |store: &BrainStore, npub: &str| {
        store
            .list_visible_brains(&user(npub))
            .unwrap()
            .into_iter()
            .map(|brain| brain.id.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(visible(&store, ADMIN), vec!["content-brain"]);
    assert_eq!(visible(&store, MEMBER), vec!["content-brain"]);
    assert!(visible(&store, GUEST).is_empty());
    assert!(visible(&store, INVITEE).is_empty());
    // Invitation and Invite Token answer exactly like unknown ones.
    let unknown_token = store
        .redeem_brain_invite_token(&"b".repeat(64), &user(INVITEE), NOW)
        .unwrap_err();
    assert_eq!(
        store
            .redeem_brain_invite_token(&"a".repeat(64), &user(INVITEE), NOW)
            .unwrap_err(),
        unknown_token
    );
    assert!(store.load_brain_invitation_by_code("code-v1").is_err());
    // A stale client write cannot recreate content.
    assert_eq!(
        store
            .submit_sync_record(
                &v1(),
                &revision_record("event-stale-write", "obj_000000000003", 1, None, "stale"),
            )
            .unwrap_err(),
        StoreError::MissingBrain {
            brain_id: "acme".to_owned()
        }
    );
    // Recreating the id fails exactly like creating over a live Brain.
    let recreate = bootstrap_organization_brain("acme", "Acme", ADMIN).unwrap();
    let live_duplicate = bootstrap_organization_brain("content-brain", "Different", ADMIN).unwrap();
    assert_eq!(
        store
            .create_brain_bootstrap(
                &recreate,
                &grants_for_required(&recreate.required_key_grants, ADMIN)
            )
            .unwrap_err(),
        StoreError::DuplicateId {
            field: "brain_id",
            value: "acme".to_owned()
        }
    );
    assert!(matches!(
        store
            .create_brain_bootstrap(
                &live_duplicate,
                &grants_for_required(&live_duplicate.required_key_grants, ADMIN)
            )
            .unwrap_err(),
        StoreError::DuplicateId {
            field: "brain_id",
            ..
        }
    ));
    let personal = bootstrap_personal_brain("acme", "Personal", "npub-owner").unwrap();
    assert!(matches!(
        store
            .create_personal_brain_bootstrap(
                &personal,
                &[],
                &user("npub-agent"),
                &user("npub-owner"),
                NOW,
            )
            .unwrap_err(),
        StoreError::DuplicateId {
            field: "brain_id",
            ..
        }
    ));
    drop(store);

    // The deletion record survives reopen: exact retries replay the same
    // receipt and change nothing; anything else sees an unknown Brain.
    let mut store = BrainStore::open(&db).unwrap();
    let digest = scope.state_digest.as_str();
    assert_eq!(
        store
            .delete_organization_brain(&request(&v1(), &admin, "event-delete-v1", digest))
            .unwrap(),
        BrainDeletionOutcome::Replayed {
            receipt: receipt.clone()
        }
    );
    let acme = v1();
    let mut expired = request(&acme, &admin, "event-delete-v1", digest);
    expired.confirmation_current = false;
    assert_eq!(
        store.delete_organization_brain(&expired).unwrap(),
        BrainDeletionOutcome::Replayed {
            receipt: receipt.clone()
        }
    );
    for (actor, event_id) in [
        (ADMIN, "event-delete-v1-again"),
        (MEMBER, "event-delete-v1"),
    ] {
        assert_eq!(
            store
                .delete_organization_brain(&request(&v1(), &user(actor), event_id, digest))
                .unwrap_err(),
            StoreError::MissingBrain {
                brain_id: "acme".to_owned()
            }
        );
    }
    assert_eq!(
        store
            .brain_deletion_replay(&v1(), "event-delete-v1", &admin)
            .unwrap(),
        Some(receipt)
    );
    assert_eq!(
        store
            .brain_deletion_replay(&v1(), "event-delete-v1", &user(MEMBER))
            .unwrap(),
        None
    );
    for npub in [ADMIN, MEMBER, GUEST] {
        assert_eq!(
            store.deleted_brain_notice(&v1(), &user(npub)).unwrap(),
            Some(DeletedBrainNotice {
                brain_id: v1(),
                deleted_at: "2026-06-24T00:00:00Z".to_owned()
            })
        );
    }
    for npub in [INVITEE, "npub-outsider"] {
        assert_eq!(
            store.deleted_brain_notice(&v1(), &user(npub)).unwrap(),
            None
        );
    }
    assert_eq!(
        store
            .deleted_brain_notice(&BrainId::new("never-existed").unwrap(), &admin)
            .unwrap(),
        None
    );
}

#[test]
fn a_stale_preview_is_refused_even_when_every_count_is_unchanged() {
    let mut store = BrainStore::open_in_memory().unwrap();
    populate(&mut store);
    let admin = user(ADMIN);
    let first = preview(&store, &v1());

    // Edit an existing page: same Folders, same page count, new revision.
    store
        .submit_sync_record(
            &v1(),
            &revision_record(
                "event-v1-page-1-edit",
                "obj_000000000001",
                2,
                Some(1),
                "edited",
            ),
        )
        .unwrap();
    let edited = preview(&store, &v1());
    assert_eq!(
        (
            edited.folder_count,
            edited.object_count,
            edited.member_count
        ),
        (first.folder_count, first.object_count, first.member_count)
    );
    assert_ne!(edited.state_digest, first.state_digest);
    let before = brain_scoped_rows(&store.conn, &v1());
    assert_eq!(
        store
            .delete_organization_brain(&request(&v1(), &admin, "event-stale", &first.state_digest))
            .unwrap_err(),
        StoreError::Conflict {
            reason: "Brain changed after its deletion preview; nothing was deleted. Review a fresh preview and confirm again".to_owned(),
            current_revision: None,
        }
    );
    assert_eq!(brain_scoped_rows(&store.conn, &v1()), before);
    assert!(
        store
            .brain_deletion_replay(&v1(), "event-stale", &admin)
            .unwrap()
            .is_none()
    );

    // Role, token and invitation changes each move the digest with no
    // change to the Folder and page counts.
    let mut digests = BTreeSet::from([edited.state_digest.clone()]);
    store.add_admin(&v1(), &user(MEMBER)).unwrap();
    assert!(digests.insert(preview(&store, &v1()).state_digest));
    store
        .revoke_brain_invite_token(&v1(), &"a".repeat(64), &admin, NOW)
        .unwrap();
    assert!(digests.insert(preview(&store, &v1()).state_digest));
    store
        .revoke_brain_invitation(&v1(), "invitation-v1", &admin, NOW)
        .unwrap();
    assert!(digests.insert(preview(&store, &v1()).state_digest));

    let fresh = preview(&store, &v1());
    assert!(matches!(
        store
            .delete_organization_brain(&request(&v1(), &admin, "event-fresh", &fresh.state_digest))
            .unwrap(),
        BrainDeletionOutcome::Deleted { .. }
    ));
}

#[test]
fn members_outsiders_personal_brains_and_lost_admins_cannot_delete() {
    let mut store = BrainStore::open_in_memory().unwrap();
    populate(&mut store);
    bootstrap_personal_named(
        &mut store,
        "personal-owner",
        "npub-owner",
        "npub-agent",
        NOW,
    );
    let admin_scope = preview(&store, &v1());
    let v1_before = brain_scoped_rows(&store.conn, &v1());
    let personal = BrainId::new("personal-owner").unwrap();
    let personal_before = brain_scoped_rows(&store.conn, &personal);

    for actor in [MEMBER, GUEST, "npub-outsider"] {
        let refused = StoreError::Conflict {
            reason: "Brain deletion requires current Brain admin standing; nothing was deleted"
                .to_owned(),
            current_revision: None,
        };
        assert_eq!(
            store
                .brain_deletion_preview(&v1(), &user(actor), SERVER)
                .unwrap_err(),
            refused
        );
        assert_eq!(
            store
                .delete_organization_brain(&request(
                    &v1(),
                    &user(actor),
                    "event-not-admin",
                    &admin_scope.state_digest
                ))
                .unwrap_err(),
            refused
        );
    }
    let personal_refused = StoreError::BrokenInvariant {
        reason: "Personal Brains cannot be deleted; only Organization Brains can".to_owned(),
    };
    for actor in ["npub-owner", "npub-agent"] {
        assert_eq!(
            store
                .brain_deletion_preview(&personal, &user(actor), SERVER)
                .unwrap_err(),
            personal_refused
        );
        assert_eq!(
            store
                .delete_organization_brain(&request(
                    &personal,
                    &user(actor),
                    "event-personal",
                    &admin_scope.state_digest
                ))
                .unwrap_err(),
            personal_refused
        );
    }
    assert_eq!(brain_scoped_rows(&store.conn, &personal), personal_before);

    // A signed confirmation older than the server accepts deletes nothing.
    let (acme, admin) = (v1(), user(ADMIN));
    let mut expired = request(&acme, &admin, "event-expired", &admin_scope.state_digest);
    expired.confirmation_current = false;
    assert_eq!(
        store.delete_organization_brain(&expired).unwrap_err(),
        StoreError::Conflict {
            reason: "Brain deletion confirmation expired; nothing was deleted. Review a fresh preview and confirm again".to_owned(),
            current_revision: None,
        }
    );
    assert_eq!(brain_scoped_rows(&store.conn, &v1()), v1_before);

    // A second admin previews, then loses the role before confirming.
    store.add_admin(&v1(), &user(MEMBER)).unwrap();
    let member_scope = store
        .brain_deletion_preview(&v1(), &user(MEMBER), SERVER)
        .unwrap();
    store.remove_admin_keeping_wraps(&v1(), &user(MEMBER));
    assert!(matches!(
        store
            .delete_organization_brain(&request(
                &v1(),
                &user(MEMBER),
                "event-demoted",
                &member_scope.state_digest
            ))
            .unwrap_err(),
        StoreError::Conflict { .. }
    ));
    assert!(store.load_brain(&v1()).is_ok());
}

#[test]
fn a_write_through_another_connection_invalidates_the_confirmation() {
    let temp = TempDir::new().unwrap();
    let db = temp.path().join("concurrent.sqlite3");
    let mut deleting = BrainStore::open(&db).unwrap();
    populate(&mut deleting);
    let mut writer = BrainStore::open(&db).unwrap();
    let scope = preview(&deleting, &v1());
    writer
        .submit_sync_record(
            &v1(),
            &revision_record(
                "event-concurrent",
                "obj_000000000002",
                2,
                Some(1),
                "concurrent",
            ),
        )
        .unwrap();
    assert!(matches!(
        deleting
            .delete_organization_brain(&request(
                &v1(),
                &user(ADMIN),
                "event-after-concurrent",
                &scope.state_digest
            ))
            .unwrap_err(),
        StoreError::Conflict { .. }
    ));
    assert!(deleting.load_brain(&v1()).is_ok());
}

#[test]
fn active_mounts_block_both_directions_and_revoked_artifacts_are_retired() {
    let mut store = BrainStore::open_in_memory().unwrap();
    populate(&mut store);
    let marketing_admin = user("npub-marketing");
    bootstrap_org_named(
        &mut store,
        "marketing-comms-brain",
        "Marketing",
        marketing_admin.as_str(),
    );
    let marketing = BrainId::new("marketing-comms-brain").unwrap();
    let admin = user(ADMIN);
    // acme's second admin controls the incoming Mounts; it is only a Member
    // of content-brain, so the Mount alone gives it the source Folder.
    let controller = user(MEMBER);
    create_folder_in(&mut store, &v1(), "outbound", FolderAccessMode::AdminOnly);
    store.add_admin(&v1(), &controller).unwrap();
    create_folder_in(
        &mut store,
        &content(),
        "shared-in",
        FolderAccessMode::AdminOnly,
    );
    create_folder_in(
        &mut store,
        &content(),
        "offered-in",
        FolderAccessMode::AdminOnly,
    );

    // Outgoing: acme/outbound mounted into marketing.
    store
        .create_shared_folder_invitation(
            &v1(),
            &FolderId::new("outbound").unwrap(),
            &marketing,
            "offer-out",
            &marketing_admin,
            &admin,
            "/v1/mount-offers/offer-out/accept",
            &grant(
                "grant-offer-out",
                "outbound",
                1,
                ADMIN,
                marketing_admin.as_str(),
            ),
            EXPIRES,
            NOW,
        )
        .unwrap();
    accept_mount_for_test(
        &mut store,
        "offer-out",
        &marketing_admin,
        "mount-out",
        "projection-out",
        &[],
        NOW,
    )
    .unwrap();
    // Incoming: content-brain/shared-in mounted into acme, plus a pending
    // offer of content-brain/offered-in to acme.
    for (offer, folder) in [("offer-in", "shared-in"), ("offer-pending", "offered-in")] {
        store
            .create_shared_folder_invitation(
                &content(),
                &FolderId::new(folder).unwrap(),
                &v1(),
                offer,
                &controller,
                &admin,
                &format!("/v1/mount-offers/{offer}/accept"),
                &grant(&format!("grant-{offer}"), folder, 1, ADMIN, MEMBER),
                EXPIRES,
                NOW,
            )
            .unwrap();
    }
    accept_mount_for_test(
        &mut store,
        "offer-in",
        &controller,
        "mount-in",
        "projection-in",
        &[],
        NOW,
    )
    .unwrap();

    let blocked = store.brain_deletion_preview(&v1(), &admin, SERVER).unwrap();
    assert_eq!(
        blocked
            .blocking_mounts
            .iter()
            .map(|mount| (mount.connection_id.as_str(), mount.direction))
            .collect::<Vec<_>>(),
        vec![
            ("mount-in", SharedFolderDirection::Destination),
            ("mount-out", SharedFolderDirection::Source),
        ]
    );
    let before = brain_scoped_rows(&store.conn, &v1());
    assert_eq!(
        store
            .delete_organization_brain(&request(&v1(), &admin, "event-blocked", &blocked.state_digest))
            .unwrap_err(),
        StoreError::Conflict {
            reason: "Brain has 2 active cross-Brain Mount(s) (mount-in, mount-out); revoke them before deleting the Brain. Nothing was deleted".to_owned(),
            current_revision: None,
        }
    );
    assert_eq!(brain_scoped_rows(&store.conn, &v1()), before);

    // Each source admin revokes its Mount, rotating the source Folder Key
    // to everyone still entitled.
    for connection in ["mount-in", "mount-out"] {
        let mount = store.load_shared_folder_connection(connection).unwrap();
        let source = store.load_brain(&mount.source_brain_id).unwrap();
        let folder = source
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == mount.source_folder_id)
            .unwrap();
        let grants = folder_key_required_recipients(&source, folder)
            .unwrap()
            .difference(&mount.managed_access_npubs)
            .map(|recipient| {
                grant(
                    &format!("grant-{}-v2-{recipient}", folder.id),
                    folder.id.as_str(),
                    2,
                    ADMIN,
                    recipient.as_str(),
                )
            })
            .collect::<Vec<_>>();
        revoke_mount_for_test(&mut store, connection, &admin, 2, &grants, &[], NOW).unwrap();
    }
    let semantic = |store: &BrainStore, brain: &BrainId| {
        brain_scoped_rows(&store.conn, brain)
            .into_iter()
            .filter(|row| {
                !row.starts_with("shared_folder_")
                    && !row.starts_with("folder_mounts|")
                    && !row.starts_with("brain_capacity_counts|")
            })
            .collect::<Vec<_>>()
    };
    let content_before = semantic(&store, &content());
    let marketing_before = semantic(&store, &marketing);
    let ready = store.brain_deletion_preview(&v1(), &admin, SERVER).unwrap();
    assert!(ready.blocking_mounts.is_empty());
    // Two revoked Mounts plus the pending incoming offer.
    assert_eq!(ready.retired_mount_record_count, 3);
    assert!(matches!(
        store
            .delete_organization_brain(&request(
                &v1(),
                &admin,
                "event-after-revoke",
                &ready.state_digest
            ))
            .unwrap(),
        BrainDeletionOutcome::Deleted { .. }
    ));
    // Other Brains keep every Folder, page, key grant and role; only the
    // retired Mount artifacts naming acme are gone, so no stale offer can
    // restore access.
    assert_eq!(semantic(&store, &content()), content_before);
    assert_eq!(semantic(&store, &marketing), marketing_before);
    for offer in ["offer-out", "offer-in", "offer-pending"] {
        assert!(store.load_shared_folder_invitation(offer).is_err());
    }
    for connection in ["mount-in", "mount-out"] {
        assert!(store.load_shared_folder_connection(connection).is_err());
    }
    assert!(brain_scoped_rows(&store.conn, &v1()).is_empty());
}

#[test]
fn deletion_scope_names_every_table_in_the_schema() {
    let store = BrainStore::open_in_memory().unwrap();
    let mut stmt = store
        .conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap();
    let tables = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<BTreeSet<_>, _>>()
        .unwrap();
    let scoped = brain_deletion_scope_tables();
    let unclassified = tables
        .iter()
        .filter(|table| {
            !scoped.contains(&table.as_str())
                && !BRAIN_DELETION_UNSCOPED_TABLES.contains(&table.as_str())
        })
        .collect::<Vec<_>>();
    assert!(
        unclassified.is_empty(),
        "decide whether Brain deletion owns these tables: {unclassified:?}"
    );
    for table in scoped.iter().chain(BRAIN_DELETION_UNSCOPED_TABLES) {
        assert!(tables.contains(*table), "{table} is not in the schema");
    }
}
