use crate::*;

impl BrainStore {
    pub(crate) fn apply_migrations(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            );
            "#,
        )?;

        if !migration_applied(&tx, 1)? {
            tx.execute_batch(SCHEMA_V1)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![1, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 2)? {
            tx.execute_batch(SCHEMA_V2)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![2, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 3)? {
            tx.execute_batch(SCHEMA_V3)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![3, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 4)? {
            tx.execute_batch(SCHEMA_V4)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![4, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 5)? {
            tx.execute_batch(SCHEMA_V5)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![5, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 6)? {
            tx.execute_batch(SCHEMA_V6)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![6, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 7)? {
            tx.execute_batch(SCHEMA_V7)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![7, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 8)? {
            tx.execute_batch(SCHEMA_V8)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![8, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 9)? {
            tx.execute_batch(SCHEMA_V9)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![9, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 10)? {
            tx.execute_batch(SCHEMA_V10)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![10, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 11)? {
            tx.execute_batch(SCHEMA_V11)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![11, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 12)? {
            tx.execute_batch(SCHEMA_V12)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![12, MIGRATION_TIMESTAMP],
            )?;
        }

        tx.commit()?;
        Ok(())
    }
}

const SCHEMA_V1: &str = r#"
CREATE TABLE vaults (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('personal', 'organization')),
    name TEXT NOT NULL,
    owner_user_id TEXT,
    created_at TEXT NOT NULL,
    CHECK (
        (kind = 'personal' AND owner_user_id IS NOT NULL) OR
        (kind = 'organization' AND owner_user_id IS NULL)
    )
);

CREATE TABLE vault_members (
    vault_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    PRIMARY KEY (vault_id, user_id),
    FOREIGN KEY (vault_id) REFERENCES vaults(id) ON DELETE CASCADE
);

CREATE TABLE vault_admins (
    vault_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    PRIMARY KEY (vault_id, user_id),
    FOREIGN KEY (vault_id, user_id) REFERENCES vault_members(vault_id, user_id)
        ON DELETE CASCADE
);

CREATE TABLE folders (
    vault_id TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('personal_home', 'vault_ops', 'general', 'folder')),
    access TEXT NOT NULL CHECK (access IN ('owner', 'admin_only', 'all_members', 'restricted')),
    parent_folder_id TEXT,
    parent_folder_key TEXT NOT NULL,
    path TEXT NOT NULL,
    current_key_version INTEGER NOT NULL CHECK (current_key_version > 0),
    shared_folder_source INTEGER NOT NULL CHECK (shared_folder_source IN (0, 1)),
    setup_incomplete INTEGER NOT NULL CHECK (setup_incomplete IN (0, 1)),
    created_at TEXT NOT NULL,
    PRIMARY KEY (vault_id, id),
    UNIQUE (vault_id, parent_folder_key, name),
    FOREIGN KEY (vault_id) REFERENCES vaults(id) ON DELETE CASCADE,
    FOREIGN KEY (vault_id, parent_folder_id) REFERENCES folders(vault_id, id)
        ON DELETE RESTRICT
);

CREATE TABLE folder_access (
    vault_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    PRIMARY KEY (vault_id, folder_id, user_id),
    FOREIGN KEY (vault_id, folder_id) REFERENCES folders(vault_id, id)
        ON DELETE CASCADE,
    FOREIGN KEY (vault_id, user_id) REFERENCES vault_members(vault_id, user_id)
        ON DELETE CASCADE
);

CREATE TABLE folder_key_grants (
    id TEXT PRIMARY KEY NOT NULL,
    vault_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    issuer_npub TEXT NOT NULL,
    recipient_npub TEXT NOT NULL,
    format TEXT NOT NULL CHECK (format = 'NIP-59'),
    wrapped_event_json TEXT NOT NULL,
    access_change_event_json TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (vault_id, folder_id, key_version, recipient_npub),
    FOREIGN KEY (vault_id, folder_id) REFERENCES folders(vault_id, id)
        ON DELETE CASCADE
);
"#;

const SCHEMA_V2: &str = r#"
CREATE TABLE vault_record_index (
    vault_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    record_event_id TEXT NOT NULL,
    record_type TEXT NOT NULL CHECK (
        record_type IN (
            'folder_object_revision',
            'folder_object_tombstone',
            'folder_key_grant',
            'vault_admin_access_change'
        )
    ),
    folder_id TEXT,
    object_id TEXT,
    revision INTEGER,
    actor_npub TEXT NOT NULL,
    client_created_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    accepted_at TEXT NOT NULL,
    record_event_kind INTEGER NOT NULL,
    PRIMARY KEY (vault_id, sequence),
    UNIQUE (vault_id, record_event_id),
    FOREIGN KEY (vault_id) REFERENCES vaults(id) ON DELETE CASCADE,
    FOREIGN KEY (vault_id, folder_id) REFERENCES folders(vault_id, id)
        ON DELETE RESTRICT
);

CREATE INDEX vault_record_index_by_event
    ON vault_record_index(vault_id, record_event_id);

CREATE TABLE current_encrypted_vault_objects (
    vault_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    updated_at TEXT NOT NULL,
    deleted INTEGER NOT NULL CHECK (deleted IN (0, 1)),
    PRIMARY KEY (vault_id, folder_id, object_id),
    FOREIGN KEY (vault_id, folder_id) REFERENCES folders(vault_id, id)
        ON DELETE CASCADE
);

CREATE TABLE vault_sync_retention (
    vault_id TEXT PRIMARY KEY NOT NULL,
    retention_floor INTEGER NOT NULL CHECK (retention_floor >= 0),
    FOREIGN KEY (vault_id) REFERENCES vaults(id) ON DELETE CASCADE
);
"#;

const SCHEMA_V3: &str = r#"
CREATE TABLE vault_invitations (
    id TEXT PRIMARY KEY NOT NULL,
    vault_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'revoked')),
    invite_code TEXT NOT NULL UNIQUE,
    accept_path TEXT NOT NULL,
    initial_folder_access_json TEXT NOT NULL,
    created_by_npub TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    accepted_at TEXT,
    FOREIGN KEY (vault_id) REFERENCES vaults(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX vault_invitations_pending_target
    ON vault_invitations(vault_id, user_id)
    WHERE status = 'pending';

CREATE TABLE share_links (
    id TEXT PRIMARY KEY NOT NULL,
    vault_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    recipient_npub TEXT NOT NULL,
    created_by_npub TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'revoked')),
    accept_path TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    accepted_at TEXT,
    grant_id TEXT NOT NULL,
    grant_key_version INTEGER NOT NULL CHECK (grant_key_version > 0),
    grant_wrapped_event_json TEXT NOT NULL,
    access_change_event_json TEXT NOT NULL,
    create_personal_mount INTEGER NOT NULL CHECK (create_personal_mount IN (0, 1)),
    personal_mount_id TEXT,
    FOREIGN KEY (vault_id, folder_id) REFERENCES folders(vault_id, id)
        ON DELETE CASCADE
);

CREATE UNIQUE INDEX share_links_pending_target
    ON share_links(vault_id, folder_id, recipient_npub)
    WHERE status = 'pending';

CREATE TABLE personal_folder_mounts (
    id TEXT PRIMARY KEY NOT NULL,
    owner_npub TEXT NOT NULL,
    source_vault_id TEXT NOT NULL,
    source_folder_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    display_parent_folder_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (owner_npub, source_vault_id, source_folder_id),
    FOREIGN KEY (source_vault_id, source_folder_id)
        REFERENCES folders(vault_id, id) ON DELETE CASCADE
);
"#;

const SCHEMA_V4: &str = r#"
CREATE TABLE shared_folder_invitations (
    id TEXT PRIMARY KEY NOT NULL,
    source_vault_id TEXT NOT NULL,
    source_folder_id TEXT NOT NULL,
    destination_vault_id TEXT NOT NULL,
    destination_admin_npub TEXT NOT NULL,
    created_by_npub TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'revoked')),
    current_key_version INTEGER NOT NULL CHECK (current_key_version > 0),
    accept_path TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    accepted_at TEXT,
    grant_id TEXT NOT NULL,
    grant_wrapped_event_json TEXT NOT NULL,
    access_change_event_json TEXT NOT NULL,
    FOREIGN KEY (source_vault_id, source_folder_id)
        REFERENCES folders(vault_id, id) ON DELETE CASCADE,
    FOREIGN KEY (destination_vault_id) REFERENCES vaults(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX shared_folder_invitations_pending_target
    ON shared_folder_invitations(source_vault_id, source_folder_id, destination_vault_id)
    WHERE status = 'pending';

CREATE TABLE shared_folder_connections (
    id TEXT PRIMARY KEY NOT NULL,
    source_vault_id TEXT NOT NULL,
    source_folder_id TEXT NOT NULL,
    destination_vault_id TEXT NOT NULL,
    destination_admin_npub TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'revoked')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (source_vault_id, source_folder_id, destination_vault_id),
    FOREIGN KEY (source_vault_id, source_folder_id)
        REFERENCES folders(vault_id, id) ON DELETE CASCADE,
    FOREIGN KEY (destination_vault_id) REFERENCES vaults(id) ON DELETE CASCADE
);

CREATE TABLE shared_folder_connection_members (
    connection_id TEXT NOT NULL,
    member_npub TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (connection_id, member_npub),
    FOREIGN KEY (connection_id) REFERENCES shared_folder_connections(id)
        ON DELETE CASCADE
);

CREATE TABLE organization_folder_mounts (
    id TEXT PRIMARY KEY NOT NULL,
    organization_vault_id TEXT NOT NULL,
    source_vault_id TEXT NOT NULL,
    source_folder_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    display_parent_folder_id TEXT,
    created_by_npub TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (organization_vault_id, source_vault_id, source_folder_id),
    FOREIGN KEY (organization_vault_id) REFERENCES vaults(id) ON DELETE CASCADE,
    FOREIGN KEY (source_vault_id, source_folder_id)
        REFERENCES folders(vault_id, id) ON DELETE CASCADE,
    FOREIGN KEY (connection_id) REFERENCES shared_folder_connections(id)
        ON DELETE CASCADE
);
"#;

const SCHEMA_V5: &str = r#"
CREATE TABLE identity_aliases (
    npub TEXT PRIMARY KEY NOT NULL,
    hex_public_key TEXT NOT NULL UNIQUE,
    preferred_nip05 TEXT,
    nip05_verified_at TEXT,
    nip05_relays_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX identity_aliases_preferred_nip05
    ON identity_aliases(preferred_nip05)
    WHERE preferred_nip05 IS NOT NULL;
"#;

const SCHEMA_V6: &str = r#"
DROP INDEX IF EXISTS vault_invitations_pending_target;

ALTER TABLE vault_invitations RENAME TO vault_invitations_old;

CREATE TABLE vault_invitations (
    id TEXT PRIMARY KEY NOT NULL,
    vault_id TEXT NOT NULL,
    user_id TEXT,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('npub', 'email_bootstrap')),
    invited_email TEXT,
    invite_unwrap_npub TEXT,
    bootstrap_payload_hash TEXT,
    bootstrap_wrapped_event_json TEXT,
    bootstrap_authorization_event_json TEXT,
    bootstrap_scope_json TEXT NOT NULL DEFAULT '[]',
    claimed_by_npub TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'revoked')),
    invite_code TEXT NOT NULL UNIQUE,
    accept_path TEXT NOT NULL,
    initial_folder_access_json TEXT NOT NULL,
    created_by_npub TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    accepted_at TEXT,
    CHECK (
        (target_kind = 'npub' AND user_id IS NOT NULL AND invited_email IS NULL) OR
        (target_kind = 'email_bootstrap' AND invited_email IS NOT NULL)
    ),
    FOREIGN KEY (vault_id) REFERENCES vaults(id) ON DELETE CASCADE
);

INSERT INTO vault_invitations (
    id, vault_id, user_id, target_kind, invited_email, invite_unwrap_npub,
    bootstrap_payload_hash, bootstrap_wrapped_event_json,
    bootstrap_authorization_event_json, bootstrap_scope_json, claimed_by_npub,
    status, invite_code, accept_path, initial_folder_access_json,
    created_by_npub, expires_at, created_at, updated_at, accepted_at
)
SELECT
    id, vault_id, user_id, 'npub', NULL, NULL,
    NULL, NULL, NULL, '[]', NULL,
    status, invite_code, accept_path, initial_folder_access_json,
    created_by_npub, expires_at, created_at, updated_at, accepted_at
FROM vault_invitations_old;

DROP TABLE vault_invitations_old;

CREATE UNIQUE INDEX vault_invitations_pending_npub_target
    ON vault_invitations(vault_id, user_id)
    WHERE status = 'pending' AND target_kind = 'npub';

CREATE UNIQUE INDEX vault_invitations_pending_email_target
    ON vault_invitations(vault_id, invited_email)
    WHERE status = 'pending' AND target_kind = 'email_bootstrap';
"#;

const SCHEMA_V7: &str = r#"
CREATE TABLE brain_email_access_delegations (
    id TEXT PRIMARY KEY NOT NULL,
    vault_id TEXT NOT NULL,
    owner_npub TEXT NOT NULL,
    agent_npub TEXT NOT NULL,
    workspace_folder_id TEXT NOT NULL,
    scope_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'revoked')),
    created_by_npub TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revoked_at TEXT,
    UNIQUE (vault_id, agent_npub),
    FOREIGN KEY (vault_id) REFERENCES vaults(id) ON DELETE CASCADE,
    FOREIGN KEY (vault_id, workspace_folder_id)
        REFERENCES folders(vault_id, id) ON DELETE RESTRICT
);

CREATE TABLE brain_email_access_delegation_audit (
    id TEXT PRIMARY KEY NOT NULL,
    delegation_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('created', 'revoked')),
    actor_npub TEXT NOT NULL,
    subject_npub TEXT NOT NULL,
    scope_json TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    FOREIGN KEY (delegation_id) REFERENCES brain_email_access_delegations(id)
        ON DELETE CASCADE
);

CREATE INDEX brain_email_access_delegation_audit_by_delegation
    ON brain_email_access_delegation_audit(delegation_id, occurred_at, id);
"#;

const SCHEMA_V8: &str = r#"
CREATE TABLE personal_vault_bootstrap_authorizations (
    authorization_id TEXT PRIMARY KEY NOT NULL,
    authorization_event_id TEXT NOT NULL UNIQUE,
    owner_npub TEXT NOT NULL,
    agent_npub TEXT NOT NULL,
    vault_id TEXT NOT NULL,
    workspace_folder_id TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    consumed_at TEXT NOT NULL,
    FOREIGN KEY (vault_id) REFERENCES vaults(id) ON DELETE CASCADE,
    FOREIGN KEY (vault_id, workspace_folder_id)
        REFERENCES folders(vault_id, id) ON DELETE RESTRICT
);
"#;

const SCHEMA_V9: &str = r#"
ALTER TABLE brain_email_access_delegation_audit
    RENAME TO brain_email_access_delegation_audit_old;

CREATE TABLE brain_email_access_delegation_audit (
    id TEXT PRIMARY KEY NOT NULL,
    delegation_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('created', 'scope_expanded', 'revoked')),
    actor_npub TEXT NOT NULL,
    subject_npub TEXT NOT NULL,
    scope_json TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    FOREIGN KEY (delegation_id) REFERENCES brain_email_access_delegations(id)
        ON DELETE CASCADE
);

INSERT INTO brain_email_access_delegation_audit (
    id, delegation_id, action, actor_npub, subject_npub, scope_json, occurred_at
)
SELECT id, delegation_id, action, actor_npub, subject_npub, scope_json, occurred_at
FROM brain_email_access_delegation_audit_old;

DROP TABLE brain_email_access_delegation_audit_old;

CREATE INDEX brain_email_access_delegation_audit_by_delegation
    ON brain_email_access_delegation_audit(delegation_id, occurred_at, id);
"#;

// Keep migrations 1-9 byte-for-byte compatible with deployed databases. The
// product rename is a forward migration of private SQLite identifiers; durable
// record literals remain unchanged and are translated at the store boundary.
const SCHEMA_V10: &str = r#"
ALTER TABLE vaults RENAME TO brains;
ALTER TABLE vault_members RENAME TO brain_members;
ALTER TABLE vault_admins RENAME TO brain_admins;
ALTER TABLE vault_record_index RENAME TO brain_record_index;
ALTER TABLE current_encrypted_vault_objects RENAME TO current_encrypted_brain_objects;
ALTER TABLE vault_sync_retention RENAME TO brain_sync_retention;
ALTER TABLE vault_invitations RENAME TO brain_invitations;

ALTER TABLE brain_members RENAME COLUMN vault_id TO brain_id;
ALTER TABLE brain_admins RENAME COLUMN vault_id TO brain_id;
ALTER TABLE folders RENAME COLUMN vault_id TO brain_id;
ALTER TABLE folder_access RENAME COLUMN vault_id TO brain_id;
ALTER TABLE folder_key_grants RENAME COLUMN vault_id TO brain_id;
ALTER TABLE brain_record_index RENAME COLUMN vault_id TO brain_id;
ALTER TABLE current_encrypted_brain_objects RENAME COLUMN vault_id TO brain_id;
ALTER TABLE brain_sync_retention RENAME COLUMN vault_id TO brain_id;
ALTER TABLE brain_invitations RENAME COLUMN vault_id TO brain_id;
ALTER TABLE share_links RENAME COLUMN vault_id TO brain_id;
ALTER TABLE personal_folder_mounts RENAME COLUMN source_vault_id TO source_brain_id;
ALTER TABLE shared_folder_invitations RENAME COLUMN source_vault_id TO source_brain_id;
ALTER TABLE shared_folder_invitations RENAME COLUMN destination_vault_id TO destination_brain_id;
ALTER TABLE shared_folder_connections RENAME COLUMN source_vault_id TO source_brain_id;
ALTER TABLE shared_folder_connections RENAME COLUMN destination_vault_id TO destination_brain_id;
ALTER TABLE organization_folder_mounts RENAME COLUMN organization_vault_id TO organization_brain_id;
ALTER TABLE organization_folder_mounts RENAME COLUMN source_vault_id TO source_brain_id;

CREATE TABLE personal_agents (
    brain_id TEXT PRIMARY KEY NOT NULL,
    owner_npub TEXT NOT NULL,
    agent_npub TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status = 'active'),
    created_by_npub TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (owner_npub <> agent_npub),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);

CREATE TABLE personal_agent_audit (
    id TEXT PRIMARY KEY NOT NULL,
    brain_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('established', 'replaced', 'revoked')),
    actor_npub TEXT NOT NULL,
    previous_agent_npub TEXT,
    agent_npub TEXT,
    occurred_at TEXT NOT NULL,
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);

CREATE INDEX personal_agent_audit_by_brain
    ON personal_agent_audit(brain_id, occurred_at, id);
"#;

const SCHEMA_V11: &str = r#"
CREATE TABLE deleted_folder_identities (
    brain_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    root_folder_id TEXT NOT NULL,
    deletion_event_id TEXT NOT NULL,
    actor_npub TEXT NOT NULL,
    deleted_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    root_key_version INTEGER NOT NULL CHECK (root_key_version > 0),
    folder_count INTEGER NOT NULL CHECK (folder_count > 0),
    object_count INTEGER NOT NULL CHECK (object_count >= 0),
    PRIMARY KEY (brain_id, folder_id),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);

CREATE TABLE deleted_object_identities (
    brain_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    root_folder_id TEXT NOT NULL,
    deletion_event_id TEXT NOT NULL,
    actor_npub TEXT NOT NULL,
    deleted_at TEXT NOT NULL,
    PRIMARY KEY (brain_id, folder_id, object_id),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);
"#;

const SCHEMA_V12: &str = r#"
CREATE UNIQUE INDEX personal_brains_one_per_owner
    ON brains(owner_user_id)
    WHERE kind = 'personal';
"#;

fn migration_applied(tx: &Transaction<'_>, version: i64) -> Result<bool, StoreError> {
    let applied = tx
        .query_row(
            "SELECT 1 FROM schema_migrations WHERE version = ?1",
            params![version],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_deployed_v9_schema_and_preserves_brain_data() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            );
            "#,
        )
        .unwrap();

        for (version, migration) in [
            SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8,
            SCHEMA_V9,
        ]
        .into_iter()
        .enumerate()
        {
            conn.execute_batch(migration).unwrap();
            conn.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![version as i64 + 1, MIGRATION_TIMESTAMP],
            )
            .unwrap();
        }

        conn.execute_batch(
            r#"
            INSERT INTO vaults (id, kind, name, owner_user_id, created_at)
            VALUES ('legacy-organization', 'organization', 'Legacy Organization', NULL, '2026-06-23T00:00:00Z');
            INSERT INTO vault_members (vault_id, user_id)
            VALUES ('legacy-organization', 'npub-owner');
            INSERT INTO vault_admins (vault_id, user_id)
            VALUES ('legacy-organization', 'npub-owner');
            INSERT INTO folders (
                vault_id, id, name, role, access, parent_folder_id, parent_folder_key,
                path, current_key_version, shared_folder_source, setup_incomplete, created_at
            ) VALUES (
                'legacy-organization', 'ops', 'Operations', 'vault_ops', 'owner', NULL, '',
                'operations', 1, 0, 0, '2026-06-23T00:00:00Z'
            );
            INSERT INTO folder_access (vault_id, folder_id, user_id)
            VALUES ('legacy-organization', 'ops', 'npub-owner');
            INSERT INTO folder_key_grants (
                id, vault_id, folder_id, key_version, issuer_npub, recipient_npub,
                format, wrapped_event_json, access_change_event_json, created_at
            ) VALUES (
                'grant-1', 'legacy-organization', 'ops', 1, 'npub-owner', 'npub-owner',
                'NIP-59', '{}', NULL, '2026-06-23T00:00:00Z'
            );
            INSERT INTO vault_record_index (
                vault_id, sequence, record_event_id, record_type, folder_id, object_id,
                revision, actor_npub, client_created_at, payload_json, accepted_at,
                record_event_kind
            ) VALUES (
                'legacy-organization', 1, 'event-1', 'vault_admin_access_change', NULL, NULL,
                NULL, 'npub-owner', '2026-06-23T00:00:00Z', '{}',
                '2026-06-23T00:00:00Z', 30078
            );
            INSERT INTO current_encrypted_vault_objects (
                vault_id, folder_id, object_id, payload_json, revision, updated_at, deleted
            ) VALUES (
                'legacy-organization', 'ops', 'object-1', '{"ciphertext":"preserved"}', 1,
                '2026-06-23T00:00:00Z', 0
            );
            INSERT INTO vault_sync_retention (vault_id, retention_floor)
            VALUES ('legacy-organization', 0);
            "#,
        )
        .unwrap();

        let store = BrainStore::from_connection(conn).unwrap();
        let stored = store
            .load_brain(&BrainId::new("legacy-organization").unwrap())
            .unwrap();

        assert_eq!(stored.brain.name.as_str(), "Legacy Organization");
        assert_eq!(stored.brain.kind, BrainKind::Organization);
        assert_eq!(stored.brain.folders.len(), 1);
        assert_eq!(stored.brain.folders[0].role, FolderRole::BrainOps);

        let record_type: String = store
            .conn
            .query_row(
                "SELECT record_type FROM brain_record_index WHERE record_event_id = 'event-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            SyncRecordType::try_from(record_type.as_str()).unwrap(),
            SyncRecordType::BrainAdminAccessChange
        );

        let preserved_payload: String = store
            .conn
            .query_row(
                "SELECT payload_json FROM current_encrypted_brain_objects \
                 WHERE brain_id = 'legacy-organization' AND object_id = 'object-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preserved_payload, r#"{"ciphertext":"preserved"}"#);

        let latest_version: i64 = store
            .conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(latest_version, 12);

        let old_table_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema \
                 WHERE type = 'table' AND name IN ('vaults', 'vault_record_index')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old_table_count, 0);

        store
            .conn
            .execute(
                r#"
                INSERT INTO folders (
                    brain_id, id, name, role, access, parent_folder_id, parent_folder_key,
                    path, current_key_version, shared_folder_source, setup_incomplete, created_at
                ) VALUES (
                    'legacy-organization', 'post-migration', 'Post migration', 'vault_ops',
                    'owner', NULL, '', 'post-migration', 1, 0, 0,
                    '2026-06-23T00:00:00Z'
                )
                "#,
                [],
            )
            .unwrap();

        let foreign_key_failures: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(foreign_key_failures, 0);
    }
}
