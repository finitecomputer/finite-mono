use finitesites_store::Store;
use rusqlite::{Connection, params};
use std::path::Path;

const OLD_OWNER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const CURRENT_KEY: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const ORIGIN_KEY: &str = "3333333333333333333333333333333333333333333333333333333333333333";

fn legacy_registry(path: &Path) {
    let mut store = Store::open(path).unwrap();
    store
        .create_site_with_claim("site_1", "claim_1", "preserved", OLD_OWNER, 100)
        .unwrap();
    drop(store);
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "INSERT INTO principals (id, kind, pubkey, created_at, updated_at)
         VALUES ('old_owner', 'native', ?1, 100, 100),
                ('current_key', 'native', ?2, 100, 100),
                ('origin_key', 'native', ?3, 100, 100)",
        params![OLD_OWNER, CURRENT_KEY, ORIGIN_KEY],
    )
    .unwrap();
    conn.execute_batch(
        "INSERT INTO sites_email_principals (id, email, verified_at, created_at, updated_at)
         VALUES ('publisher', 'publisher@example.com', 100, 100, 100);
         INSERT INTO sites_authorized_keys
           (id, email_principal_id, native_principal_id, proof_kind,
            verified_at, revoked_at, created_at, updated_at)
         VALUES ('old_key', 'publisher', 'old_owner', 'mailbox_challenge', 100, 101, 100, 101),
                ('new_key', 'publisher', 'current_key', 'mailbox_challenge', 100, NULL, 100, 100);
         PRAGMA foreign_keys = OFF;
         CREATE TABLE legacy_sites (
           id TEXT PRIMARY KEY,
           owner_pubkey TEXT NOT NULL CHECK (length(owner_pubkey) = 64),
           status TEXT NOT NULL CHECK (status IN ('claimed_unpublished', 'published', 'disabled', 'deleted')),
           visibility TEXT NOT NULL CHECK (visibility IN ('private', 'shared', 'public')),
           kind TEXT NOT NULL DEFAULT 'static' CHECK (kind IN ('static', 'app')),
           app_port INTEGER UNIQUE,
           active_version_id TEXT REFERENCES versions(id),
           publisher_email_principal_id TEXT REFERENCES sites_email_principals(id),
           originating_publisher_principal_id TEXT REFERENCES principals(id),
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL
         );
         INSERT INTO legacy_sites
         SELECT id, owner_pubkey, status, visibility, kind, app_port, active_version_id,
                'publisher', 'origin_key', created_at, updated_at FROM sites;
         DROP TABLE sites;
         ALTER TABLE legacy_sites RENAME TO sites;",
    )
    .unwrap();
}

#[test]
fn static_kind_migration_preserves_publisher_access_and_key_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.db");
    legacy_registry(&path);

    assert_publisher_access_survives_restart(&path);
}

#[test]
fn legacy_owner_shape_migration_preserves_publisher_access_and_key_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.db");
    legacy_registry(&path);
    drop(Store::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute_batch("ALTER TABLE sites ADD COLUMN owner_email TEXT;")
        .unwrap();

    assert_publisher_access_survives_restart(&path);
}

#[test]
fn missing_or_null_attribution_does_not_infer_a_publisher() {
    for remove_columns in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.db");
        legacy_registry(&path);
        let sql = if remove_columns {
            "ALTER TABLE sites DROP COLUMN publisher_email_principal_id;
             ALTER TABLE sites DROP COLUMN originating_publisher_principal_id;"
        } else {
            "UPDATE sites SET publisher_email_principal_id = NULL,
                              originating_publisher_principal_id = NULL;"
        };
        Connection::open(&path).unwrap().execute_batch(sql).unwrap();

        for _ in 0..2 {
            let store = Store::open(&path).unwrap();
            assert_eq!(store.publisher_email_for_site("site_1").unwrap(), None);
            assert!(store.actor_can_manage_site(OLD_OWNER, "site_1").unwrap());
            assert!(!store.actor_can_manage_site(CURRENT_KEY, "site_1").unwrap());
            assert_eq!(site_attribution(&path), (None, None));
        }
    }
}

fn legacy_project(path: &Path) {
    legacy_registry(path);
    Connection::open(path)
        .unwrap()
        .execute_batch(
            "PRAGMA foreign_keys = OFF;
             DROP TABLE projects;
             CREATE TABLE projects (
               id TEXT PRIMARY KEY,
               slug TEXT NOT NULL UNIQUE,
               owner_principal_id TEXT NOT NULL REFERENCES principals(id),
               publisher_email_principal_id TEXT REFERENCES sites_email_principals(id),
               originating_publisher_principal_id TEXT REFERENCES principals(id),
               visibility TEXT NOT NULL CHECK (visibility IN ('private', 'shared', 'public')),
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             INSERT INTO projects VALUES
               ('project_1', 'preserved-project', 'old_owner', 'publisher', 'origin_key', 'private', 100, 100);
             INSERT INTO project_collaborators
               (project_id, principal_id, role, added_by_principal_id, added_at)
             VALUES ('project_1', 'old_owner', 'owner', 'old_owner', 100);",
        )
        .unwrap();
}

#[test]
fn project_visibility_migration_preserves_publisher_access_and_key_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.db");
    legacy_project(&path);

    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        assert!(
            store
                .project_access_by_actor(CURRENT_KEY, "preserved-project")
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .project_access_by_actor(OLD_OWNER, "preserved-project")
                .unwrap()
                .is_none()
        );
        let attribution: (String, String) = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT publisher_email_principal_id, originating_publisher_principal_id
                 FROM projects WHERE id = 'project_1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(attribution, ("publisher".into(), "origin_key".into()));
    }
}

#[test]
fn missing_or_null_project_attribution_does_not_infer_a_publisher() {
    for remove_columns in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.db");
        legacy_project(&path);
        let sql = if remove_columns {
            "ALTER TABLE projects DROP COLUMN publisher_email_principal_id;
             ALTER TABLE projects DROP COLUMN originating_publisher_principal_id;"
        } else {
            "UPDATE projects SET publisher_email_principal_id = NULL,
                                 originating_publisher_principal_id = NULL;"
        };
        Connection::open(&path).unwrap().execute_batch(sql).unwrap();

        for _ in 0..2 {
            let store = Store::open(&path).unwrap();
            assert!(
                store
                    .project_access_by_actor(OLD_OWNER, "preserved-project")
                    .unwrap()
                    .is_some()
            );
            assert!(
                store
                    .project_access_by_actor(CURRENT_KEY, "preserved-project")
                    .unwrap()
                    .is_none()
            );
            let attribution: (Option<String>, Option<String>) = Connection::open(&path)
                .unwrap()
                .query_row(
                    "SELECT publisher_email_principal_id, originating_publisher_principal_id
                     FROM projects WHERE id = 'project_1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(attribution, (None, None));
        }
    }
}

fn assert_publisher_access_survives_restart(path: &Path) {
    for _ in 0..2 {
        let store = Store::open(path).unwrap();
        assert_eq!(
            store.publisher_email_for_site("site_1").unwrap().as_deref(),
            Some("publisher@example.com")
        );
        assert!(store.actor_can_manage_site(CURRENT_KEY, "site_1").unwrap());
        assert!(!store.actor_can_manage_site(OLD_OWNER, "site_1").unwrap());
        assert!(
            store
                .is_email_authorized_publisher("site_1", "publisher@example.com")
                .unwrap()
        );
        assert!(
            !store
                .is_email_authorized_publisher("site_1", "unshared@example.com")
                .unwrap()
        );
        assert_eq!(
            site_attribution(path),
            (Some("publisher".into()), Some("origin_key".into()))
        );
    }
}

fn site_attribution(path: &Path) -> (Option<String>, Option<String>) {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT publisher_email_principal_id, originating_publisher_principal_id
             FROM sites WHERE id = 'site_1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}
