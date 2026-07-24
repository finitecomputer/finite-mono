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

        if !migration_applied(&tx, 13)? {
            tx.execute_batch(SCHEMA_V13)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![13, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 14)? {
            tx.execute_batch(&capacity_guard_schema())?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![14, MIGRATION_TIMESTAMP],
            )?;
        }

        tx.commit()?;
        Ok(())
    }
}

const SCHEMA_V1: &str = r#"
CREATE TABLE brains (
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

CREATE TABLE brain_members (
    brain_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    PRIMARY KEY (brain_id, user_id),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);

CREATE TABLE brain_admins (
    brain_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    PRIMARY KEY (brain_id, user_id),
    FOREIGN KEY (brain_id, user_id) REFERENCES brain_members(brain_id, user_id)
        ON DELETE CASCADE
);

CREATE TABLE folders (
    brain_id TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('personal_home', 'brain_ops', 'general', 'folder')),
    access TEXT NOT NULL CHECK (access IN ('owner', 'admin_only', 'all_members', 'restricted')),
    parent_folder_id TEXT,
    parent_folder_key TEXT NOT NULL,
    path TEXT NOT NULL,
    current_key_version INTEGER NOT NULL CHECK (current_key_version > 0),
    shared_folder_source INTEGER NOT NULL CHECK (shared_folder_source IN (0, 1)),
    setup_incomplete INTEGER NOT NULL CHECK (setup_incomplete IN (0, 1)),
    created_at TEXT NOT NULL,
    PRIMARY KEY (brain_id, id),
    UNIQUE (brain_id, parent_folder_key, name),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE,
    FOREIGN KEY (brain_id, parent_folder_id) REFERENCES folders(brain_id, id)
        ON DELETE RESTRICT
);

CREATE TABLE folder_access (
    brain_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    PRIMARY KEY (brain_id, folder_id, user_id),
    FOREIGN KEY (brain_id, folder_id) REFERENCES folders(brain_id, id)
        ON DELETE CASCADE,
    FOREIGN KEY (brain_id, user_id) REFERENCES brain_members(brain_id, user_id)
        ON DELETE CASCADE
);

CREATE TABLE folder_key_grants (
    id TEXT PRIMARY KEY NOT NULL,
    brain_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    issuer_npub TEXT NOT NULL,
    recipient_npub TEXT NOT NULL,
    format TEXT NOT NULL CHECK (format = 'NIP-59'),
    wrapped_event_json TEXT NOT NULL,
    access_change_event_json TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (brain_id, folder_id, key_version, recipient_npub),
    FOREIGN KEY (brain_id, folder_id) REFERENCES folders(brain_id, id)
        ON DELETE CASCADE
);
"#;

const SCHEMA_V2: &str = r#"
CREATE TABLE brain_record_index (
    brain_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    record_event_id TEXT NOT NULL,
    record_type TEXT NOT NULL CHECK (
        record_type IN (
            'folder_object_revision',
            'folder_object_tombstone',
            'folder_key_grant',
            'brain_admin_access_change'
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
    PRIMARY KEY (brain_id, sequence),
    UNIQUE (brain_id, record_event_id),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE,
    FOREIGN KEY (brain_id, folder_id) REFERENCES folders(brain_id, id)
        ON DELETE RESTRICT
);

CREATE INDEX brain_record_index_by_event
    ON brain_record_index(brain_id, record_event_id);

CREATE TABLE current_encrypted_brain_objects (
    brain_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    updated_at TEXT NOT NULL,
    deleted INTEGER NOT NULL CHECK (deleted IN (0, 1)),
    PRIMARY KEY (brain_id, folder_id, object_id),
    FOREIGN KEY (brain_id, folder_id) REFERENCES folders(brain_id, id)
        ON DELETE CASCADE
);

CREATE TABLE brain_sync_retention (
    brain_id TEXT PRIMARY KEY NOT NULL,
    retention_floor INTEGER NOT NULL CHECK (retention_floor >= 0),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);
"#;

const SCHEMA_V3: &str = r#"
CREATE TABLE brain_invitations (
    id TEXT PRIMARY KEY NOT NULL,
    brain_id TEXT NOT NULL,
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
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX brain_invitations_pending_target
    ON brain_invitations(brain_id, user_id)
    WHERE status = 'pending';

CREATE TABLE share_links (
    id TEXT PRIMARY KEY NOT NULL,
    brain_id TEXT NOT NULL,
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
    FOREIGN KEY (brain_id, folder_id) REFERENCES folders(brain_id, id)
        ON DELETE CASCADE
);

CREATE UNIQUE INDEX share_links_pending_target
    ON share_links(brain_id, folder_id, recipient_npub)
    WHERE status = 'pending';

CREATE TABLE personal_folder_mounts (
    id TEXT PRIMARY KEY NOT NULL,
    owner_npub TEXT NOT NULL,
    source_brain_id TEXT NOT NULL,
    source_folder_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    display_parent_folder_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (owner_npub, source_brain_id, source_folder_id),
    FOREIGN KEY (source_brain_id, source_folder_id)
        REFERENCES folders(brain_id, id) ON DELETE CASCADE
);
"#;

const SCHEMA_V4: &str = r#"
CREATE TABLE shared_folder_invitations (
    id TEXT PRIMARY KEY NOT NULL,
    source_brain_id TEXT NOT NULL,
    source_folder_id TEXT NOT NULL,
    destination_brain_id TEXT NOT NULL,
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
    FOREIGN KEY (source_brain_id, source_folder_id)
        REFERENCES folders(brain_id, id) ON DELETE CASCADE,
    FOREIGN KEY (destination_brain_id) REFERENCES brains(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX shared_folder_invitations_pending_target
    ON shared_folder_invitations(source_brain_id, source_folder_id, destination_brain_id)
    WHERE status = 'pending';

CREATE TABLE shared_folder_connections (
    id TEXT PRIMARY KEY NOT NULL,
    source_brain_id TEXT NOT NULL,
    source_folder_id TEXT NOT NULL,
    destination_brain_id TEXT NOT NULL,
    destination_admin_npub TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'revoked')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (source_brain_id, source_folder_id, destination_brain_id),
    FOREIGN KEY (source_brain_id, source_folder_id)
        REFERENCES folders(brain_id, id) ON DELETE CASCADE,
    FOREIGN KEY (destination_brain_id) REFERENCES brains(id) ON DELETE CASCADE
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
    organization_brain_id TEXT NOT NULL,
    source_brain_id TEXT NOT NULL,
    source_folder_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    display_parent_folder_id TEXT,
    created_by_npub TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (organization_brain_id, source_brain_id, source_folder_id),
    FOREIGN KEY (organization_brain_id) REFERENCES brains(id) ON DELETE CASCADE,
    FOREIGN KEY (source_brain_id, source_folder_id)
        REFERENCES folders(brain_id, id) ON DELETE CASCADE,
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
DROP INDEX IF EXISTS brain_invitations_pending_target;

ALTER TABLE brain_invitations RENAME TO brain_invitations_old;

CREATE TABLE brain_invitations (
    id TEXT PRIMARY KEY NOT NULL,
    brain_id TEXT NOT NULL,
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
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);

INSERT INTO brain_invitations (
    id, brain_id, user_id, target_kind, invited_email, invite_unwrap_npub,
    bootstrap_payload_hash, bootstrap_wrapped_event_json,
    bootstrap_authorization_event_json, bootstrap_scope_json, claimed_by_npub,
    status, invite_code, accept_path, initial_folder_access_json,
    created_by_npub, expires_at, created_at, updated_at, accepted_at
)
SELECT
    id, brain_id, user_id, 'npub', NULL, NULL,
    NULL, NULL, NULL, '[]', NULL,
    status, invite_code, accept_path, initial_folder_access_json,
    created_by_npub, expires_at, created_at, updated_at, accepted_at
FROM brain_invitations_old;

DROP TABLE brain_invitations_old;

CREATE UNIQUE INDEX brain_invitations_pending_npub_target
    ON brain_invitations(brain_id, user_id)
    WHERE status = 'pending' AND target_kind = 'npub';

CREATE UNIQUE INDEX brain_invitations_pending_email_target
    ON brain_invitations(brain_id, invited_email)
    WHERE status = 'pending' AND target_kind = 'email_bootstrap';
"#;

const SCHEMA_V7: &str = "";
const SCHEMA_V8: &str = "";

const SCHEMA_V9: &str = "";
const SCHEMA_V10: &str = r#"
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

const SCHEMA_V13: &str = r#"
CREATE TABLE folder_deletion_audience (
    brain_id TEXT NOT NULL,
    deletion_event_id TEXT NOT NULL,
    actor_npub TEXT NOT NULL,
    PRIMARY KEY (brain_id, deletion_event_id, actor_npub),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);

CREATE INDEX folder_deletion_audience_by_actor
    ON folder_deletion_audience(brain_id, actor_npub, deletion_event_id);
"#;

fn capacity_guard_schema() -> String {
    let limits = BRAIN_CAPACITY_ENVELOPE;
    format!(
        r#"
CREATE TRIGGER capacity_brain_folders
BEFORE INSERT ON folders
WHEN (
    (SELECT COUNT(*) FROM folders WHERE brain_id = NEW.brain_id) +
    (SELECT COUNT(*) FROM deleted_folder_identities WHERE brain_id = NEW.brain_id)
) >= {folders}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:brain_folders:{folders}');
END;

CREATE TRIGGER capacity_folder_depth
BEFORE INSERT ON folders
WHEN NEW.parent_folder_id IS NOT NULL AND (
    WITH RECURSIVE ancestors(id, depth) AS (
        SELECT NEW.parent_folder_id, 1
        UNION ALL
        SELECT f.parent_folder_id, ancestors.depth + 1
        FROM folders f
        JOIN ancestors ON f.brain_id = NEW.brain_id AND f.id = ancestors.id
        WHERE f.parent_folder_id IS NOT NULL AND ancestors.depth <= {folder_depth}
    )
    SELECT COALESCE(MAX(depth), 0) + 1 FROM ancestors
) > {folder_depth}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:folder_depth:{folder_depth}');
END;

CREATE TRIGGER capacity_brain_members
BEFORE INSERT ON brain_members
WHEN (SELECT COUNT(*) FROM brain_members WHERE brain_id = NEW.brain_id) >= {members}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:brain_members:{members}');
END;

CREATE TRIGGER capacity_current_objects
BEFORE INSERT ON current_encrypted_brain_objects
WHEN NOT EXISTS (
    SELECT 1 FROM current_encrypted_brain_objects
    WHERE brain_id = NEW.brain_id AND folder_id = NEW.folder_id AND object_id = NEW.object_id
) AND (
    (SELECT COUNT(*) FROM current_encrypted_brain_objects WHERE brain_id = NEW.brain_id) +
    (SELECT COUNT(*) FROM deleted_object_identities WHERE brain_id = NEW.brain_id)
) >= {current_objects}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:current_objects:{current_objects}');
END;

CREATE TRIGGER capacity_sync_records
BEFORE INSERT ON brain_record_index
WHEN COALESCE(json_extract(NEW.payload_json, '$.recordType'), '') <> 'folder_subtree_tombstone'
AND (
    SELECT COUNT(*) FROM brain_record_index
    WHERE brain_id = NEW.brain_id
      AND COALESCE(json_extract(payload_json, '$.recordType'), '') <> 'folder_subtree_tombstone'
) >= {ordinary_sync_records}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:sync_records:{ordinary_sync_records}');
END;

CREATE TRIGGER capacity_deletion_records
BEFORE INSERT ON brain_record_index
WHEN COALESCE(json_extract(NEW.payload_json, '$.recordType'), '') = 'folder_subtree_tombstone'
AND (
    SELECT COUNT(*) FROM brain_record_index
    WHERE brain_id = NEW.brain_id
      AND COALESCE(json_extract(payload_json, '$.recordType'), '') = 'folder_subtree_tombstone'
) >= {folders}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:folder_deletion_records:{folders}');
END;

CREATE TRIGGER capacity_folder_access
BEFORE INSERT ON folder_access
WHEN (SELECT COUNT(*) FROM folder_access WHERE brain_id = NEW.brain_id) >= {folder_access_entries}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:folder_access_entries:{folder_access_entries}');
END;

CREATE TRIGGER capacity_folder_key_grants
BEFORE INSERT ON folder_key_grants
WHEN (SELECT COUNT(*) FROM folder_key_grants WHERE brain_id = NEW.brain_id) >= {folder_key_grants}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:folder_key_grants:{folder_key_grants}');
END;

CREATE TRIGGER capacity_brain_invitations
BEFORE INSERT ON brain_invitations
WHEN NEW.status = 'pending' AND (
    SELECT COUNT(*) FROM brain_invitations
    WHERE brain_id = NEW.brain_id AND status = 'pending'
) >= {invitations}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:brain_invitations:{invitations}');
END;

CREATE TRIGGER capacity_share_links
BEFORE INSERT ON share_links
WHEN NEW.status = 'pending' AND (
    SELECT COUNT(*) FROM share_links
    WHERE brain_id = NEW.brain_id AND status = 'pending'
) >= {share_links}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:share_links:{share_links}');
END;

CREATE TRIGGER capacity_personal_mounts
BEFORE INSERT ON personal_folder_mounts
WHEN (SELECT COUNT(*) FROM personal_folder_mounts WHERE source_brain_id = NEW.source_brain_id) >= {mounts}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:personal_mounts:{mounts}');
END;

CREATE TRIGGER capacity_shared_invitations
BEFORE INSERT ON shared_folder_invitations
WHEN NEW.status = 'pending' AND (
    SELECT COUNT(*) FROM shared_folder_invitations
    WHERE source_brain_id = NEW.source_brain_id AND status = 'pending'
) >= {invitations}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:shared_folder_invitations:{invitations}');
END;

CREATE TRIGGER capacity_shared_connections
BEFORE INSERT ON shared_folder_connections
WHEN NEW.status = 'active' AND (
    SELECT COUNT(*) FROM shared_folder_connections
    WHERE source_brain_id = NEW.source_brain_id AND status = 'active'
) >= {shared_connections}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:shared_connections:{shared_connections}');
END;

CREATE TRIGGER capacity_connection_delegations
BEFORE INSERT ON shared_folder_connection_members
WHEN (
    SELECT COUNT(*)
    FROM shared_folder_connection_members members
    JOIN shared_folder_connections connections ON connections.id = members.connection_id
    WHERE connections.source_brain_id = (
        SELECT source_brain_id FROM shared_folder_connections WHERE id = NEW.connection_id
    )
) >= {delegations}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:shared_connection_delegations:{delegations}');
END;

CREATE TRIGGER capacity_organization_mounts
BEFORE INSERT ON organization_folder_mounts
WHEN (SELECT COUNT(*) FROM organization_folder_mounts WHERE source_brain_id = NEW.source_brain_id) >= {mounts}
BEGIN
    SELECT RAISE(ABORT, 'finite_capacity:organization_mounts:{mounts}');
END;
"#,
        folders = limits.folders,
        folder_depth = limits.folder_depth,
        current_objects = limits.current_objects,
        ordinary_sync_records = limits.sync_records - limits.folders,
        members = limits.members,
        folder_access_entries = limits.folder_access_entries,
        folder_key_grants = limits.folder_key_grants,
        invitations = limits.invitations,
        share_links = limits.share_links,
        mounts = limits.mounts,
        shared_connections = limits.shared_connections,
        delegations = limits.delegations,
    )
}

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
