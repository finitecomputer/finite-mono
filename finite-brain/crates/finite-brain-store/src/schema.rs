use crate::*;

impl BrainStore {
    pub(crate) fn apply_migrations(&mut self) -> Result<(), StoreError> {
        self.apply_migrations_through_v22()?;
        let v23_applied = {
            let tx = self.conn.transaction()?;
            let applied = migration_applied(&tx, 23)?;
            tx.commit()?;
            applied
        };
        if !v23_applied {
            // SCHEMA_V23 rebuilds folder_key_grants and brain_members to extend
            // their provenance origin CHECKs. The rebuild is only safe with
            // legacy_alter_table=ON (so the RENAMEs do not rewrite folder_access
            // and brain_admins foreign keys toward the retired table names) and
            // foreign_keys=OFF (so the DROPs do not cascade into those children).
            // Neither pragma can change inside a transaction, so V23 runs as its
            // own guarded step outside the main migration transaction.
            self.conn.pragma_update(None, "legacy_alter_table", "ON")?;
            self.conn.pragma_update(None, "foreign_keys", "OFF")?;
            let result = (|| -> Result<(), StoreError> {
                let tx = self.conn.transaction()?;
                tx.execute_batch(SCHEMA_V23)?;
                tx.execute(
                    "INSERT INTO brain_departure_fact_cursor (id, last_applied_revision, updated_at) VALUES (1, 0, ?1)",
                    params![MIGRATION_TIMESTAMP],
                )?;
                tx.execute(
                    "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                    params![23, MIGRATION_TIMESTAMP],
                )?;
                tx.commit()?;
                Ok(())
            })();
            self.conn.pragma_update(None, "foreign_keys", "ON")?;
            self.conn.pragma_update(None, "legacy_alter_table", "OFF")?;
            result?;
            let violations: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_check",
                [],
                |row| row.get(0),
            )?;
            if violations > 0 {
                return Err(StoreError::BrokenInvariant {
                    reason: "schema V23 table rebuild left foreign key violations".to_owned(),
                });
            }
        }

        // V24 is additive only (one new column with a default plus new
        // tables), so it runs inside an ordinary migration transaction.
        let tx = self.conn.transaction()?;
        if !migration_applied(&tx, 24)? {
            tx.execute_batch(SCHEMA_V24)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![24, MIGRATION_TIMESTAMP],
            )?;
        }

        // V25 is additive only (one new marker table), so it runs inside the
        // same ordinary migration transaction.
        if !migration_applied(&tx, 25)? {
            tx.execute_batch(SCHEMA_V25)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![25, MIGRATION_TIMESTAMP],
            )?;
        }

        // V26 is additive only (one nullable column), same ordinary path.
        if !migration_applied(&tx, 26)? {
            tx.execute_batch(SCHEMA_V26)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![26, MIGRATION_TIMESTAMP],
            )?;
        }

        // V27 is additive only (one nullable column recording the committed
        // invitation ids of an approved request), same ordinary path.
        if !migration_applied(&tx, 27)? {
            tx.execute_batch(SCHEMA_V27)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![27, MIGRATION_TIMESTAMP],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn apply_migrations_through_v22(&mut self) -> Result<(), StoreError> {
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

        if !migration_applied(&tx, 15)? {
            tx.execute_batch(SCHEMA_V15)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![15, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 16)? {
            tx.execute_batch(SCHEMA_V16)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![16, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 17)? {
            tx.execute_batch(SCHEMA_V17)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![17, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 18)? {
            tx.execute_batch(SCHEMA_V18)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![18, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 19)? {
            tx.execute_batch(SCHEMA_V19)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![19, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 20)? {
            tx.execute_batch(SCHEMA_V20)?;
            tx.execute_batch(&format!(
                r#"
                CREATE TRIGGER capacity_folder_mounts
                BEFORE INSERT ON folder_mounts
                WHEN (
                    SELECT COUNT(*) FROM folder_mounts
                    WHERE source_brain_id = NEW.source_brain_id
                ) >= {}
                BEGIN
                    SELECT RAISE(ABORT, 'finite_capacity:folder_mounts:{}');
                END;
                "#,
                BRAIN_CAPACITY_ENVELOPE.mounts, BRAIN_CAPACITY_ENVELOPE.mounts
            ))?;
            migrate_legacy_personal_mounts(&tx)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![20, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 21)? {
            tx.execute_batch(SCHEMA_V21)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![21, MIGRATION_TIMESTAMP],
            )?;
        }

        if !migration_applied(&tx, 22)? {
            tx.execute_batch(SCHEMA_V22)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![22, MIGRATION_TIMESTAMP],
            )?;
        }

        tx.commit()?;
        Ok(())
    }
}

const SCHEMA_V25: &str = r#"
-- Pending Grant Wraps: the JOIN side of the departure pending-rotation
-- pattern. When a Principal gains Folder entitlement without receiving a
-- wrapped Folder Key Grant (invitation commit, invitation accept, or an
-- ensure-access membership repair), the server records one marker per
-- (Brain, Folder, recipient, key version). Key-holding clients discover the
-- markers on sync, wrap the current Folder Key for the waiting recipients,
-- and the markers clear when the grants validate. A marker never blocks
-- anything: it is a delivery hint, not a gate.
CREATE TABLE brain_pending_grant_wraps (
    brain_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    recipient_npub TEXT NOT NULL,
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    reason TEXT NOT NULL
        CHECK (reason IN ('invitation', 'accept', 'ensure-access', 'bootstrap')),
    created_at TEXT NOT NULL,
    UNIQUE (brain_id, folder_id, recipient_npub, key_version),
    FOREIGN KEY (brain_id, folder_id) REFERENCES folders(brain_id, id)
        ON DELETE CASCADE
);

CREATE INDEX brain_pending_grant_wraps_by_brain
    ON brain_pending_grant_wraps(brain_id, folder_id);
"#;

const SCHEMA_V26: &str = r#"
-- Folder-scoped Invitation Plans: one row per mailbox+Folder preview for
-- cohort Folder invitations. NULL keeps a plan Brain-scoped (membership);
-- a non-NULL value scopes its commit to per-principal Folder share links
-- for exactly that Folder.
ALTER TABLE brain_invitation_plans ADD COLUMN folder_id TEXT;
"#;

const SCHEMA_V27: &str = r#"
ALTER TABLE brain_approval_requests ADD COLUMN result_invitations_json TEXT;
"#;

const SCHEMA_V24: &str = r#"
-- ADR-0046 Approval artifacts: npub-bound invitations remember whether they
-- were committed directly by an admin ('invitation') or through a signed
-- finite-brain-approval-v1 Approval Card ('approval'). Existing rows stay
-- 'invitation', matching the provenance their accepts already stamp.
ALTER TABLE brain_invitations
ADD COLUMN origin_kind TEXT NOT NULL DEFAULT 'invitation'
CHECK (origin_kind IN ('invitation', 'approval'));

-- Pending human Approval requests: an agent or the UI submits the action
-- payload, the server mints the nonce and expiry, and a human key holder
-- signs exactly this payload. The request row is the durable link from the
-- signed approval event id back to the plan it committed.
CREATE TABLE brain_approval_requests (
    id TEXT PRIMARY KEY NOT NULL,
    brain_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('invite-commit', 'delegation-grant')),
    payload_json TEXT NOT NULL,
    nonce TEXT NOT NULL,
    expires_at_unix INTEGER NOT NULL CHECK (expires_at_unix > 0),
    requested_by_npub TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'denied')),
    approval_event_id TEXT,
    resolved_by_npub TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (brain_id, nonce),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);

CREATE INDEX brain_approval_requests_by_event
    ON brain_approval_requests(brain_id, approval_event_id);

-- Replay protection for applied Approvals: one row per consumed
-- (brain_id, nonce), recorded in the same critical section as the mutation
-- the approval authorized.
CREATE TABLE brain_approval_nonces (
    brain_id TEXT NOT NULL,
    nonce TEXT NOT NULL,
    approval_event_id TEXT NOT NULL,
    signer_npub TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('invite-commit', 'delegation-grant')),
    applied_at TEXT NOT NULL,
    PRIMARY KEY (brain_id, nonce),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);
"#;

const SCHEMA_V23: &str = r#"
-- ADR-0046 Permanent Departure Facts. Brain consumes Core's durable,
-- monotonic departure log from a last-applied-revision cursor. The cursor
-- advances only inside the per-fact revocation transaction, so after any
-- crash it always trails the facts whose revocations actually committed.
CREATE TABLE brain_departure_fact_cursor (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    last_applied_revision INTEGER NOT NULL DEFAULT 0
        CHECK (last_applied_revision >= 0),
    updated_at TEXT NOT NULL
);

-- Revocation ledger: the departure fact is the authority record for every
-- service-issued revocation, with the fact revision recorded. One row per
-- affected Brain per fact; ids are deterministic so replay never duplicates.
CREATE TABLE brain_principal_revocations (
    id TEXT PRIMARY KEY NOT NULL,
    brain_id TEXT NOT NULL,
    departed_npub TEXT,
    principal_kind TEXT NOT NULL CHECK (principal_kind IN ('human', 'agent')),
    principal_ref TEXT NOT NULL,
    account_id TEXT NOT NULL,
    fact_revision INTEGER NOT NULL CHECK (fact_revision > 0),
    origin_kind TEXT NOT NULL DEFAULT 'departure'
        CHECK (origin_kind IN ('direct', 'invitation', 'approval', 'bootstrap', 'departure')),
    origin_ref TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    UNIQUE (brain_id, fact_revision, principal_ref),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);

-- Folders the departed Principal could read keep their current key until a
-- remaining admin's client re-wraps; the server never sees plaintext keys.
-- A marker tracks the key version that still needs rotation-on-replay.
CREATE TABLE brain_departure_pending_rotations (
    brain_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    marked_at_revision INTEGER NOT NULL CHECK (marked_at_revision > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    updated_at TEXT NOT NULL,
    PRIMARY KEY (brain_id, folder_id),
    FOREIGN KEY (brain_id, folder_id) REFERENCES folders(brain_id, id)
        ON DELETE CASCADE
);

-- Extend the V22 provenance origin CHECKs with 'departure' so rotation grants
-- written during departure replay can be stamped with their true origin.
-- SQLite cannot alter a column CHECK, so both tables are rebuilt following
-- the V6/V9 pattern; existing rows are copied verbatim and stay valid. The
-- migration runner applies this batch with legacy_alter_table=ON and
-- foreign_keys=OFF (neither pragma can change inside a transaction), which
-- stops the RENAMEs from rewriting folder_access and brain_admins foreign
-- keys toward the retired table names and stops the DROPs from cascading.
ALTER TABLE folder_key_grants RENAME TO folder_key_grants_v22;
ALTER TABLE brain_members RENAME TO brain_members_v22;

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
    delegated_by_npub TEXT,
    origin_kind TEXT NOT NULL DEFAULT 'direct'
        CHECK (origin_kind IN ('direct', 'invitation', 'approval', 'bootstrap', 'departure')),
    origin_ref TEXT,
    roster_revision INTEGER
        CHECK (roster_revision IS NULL OR roster_revision >= 0),
    UNIQUE (brain_id, folder_id, key_version, recipient_npub),
    FOREIGN KEY (brain_id, folder_id) REFERENCES folders(brain_id, id)
        ON DELETE CASCADE
);

INSERT INTO folder_key_grants (
    id, brain_id, folder_id, key_version, issuer_npub, recipient_npub, format,
    wrapped_event_json, access_change_event_json, created_at,
    delegated_by_npub, origin_kind, origin_ref, roster_revision
)
SELECT
    id, brain_id, folder_id, key_version, issuer_npub, recipient_npub, format,
    wrapped_event_json, access_change_event_json, created_at,
    delegated_by_npub, origin_kind, origin_ref, roster_revision
FROM folder_key_grants_v22;

DROP TABLE folder_key_grants_v22;

CREATE TABLE brain_members (
    brain_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    delegated_by_npub TEXT,
    origin_kind TEXT NOT NULL DEFAULT 'direct'
        CHECK (origin_kind IN ('direct', 'invitation', 'approval', 'bootstrap', 'departure')),
    origin_ref TEXT,
    PRIMARY KEY (brain_id, user_id),
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);

INSERT INTO brain_members (
    brain_id, user_id, delegated_by_npub, origin_kind, origin_ref
)
SELECT
    brain_id, user_id, delegated_by_npub, origin_kind, origin_ref
FROM brain_members_v22;

DROP TABLE brain_members_v22;
"#;

const SCHEMA_V22: &str = r#"
-- ADR-0046 Grant Provenance: every access record carries who delegated it,
-- which invitation or approval it came from, and the account roster state at
-- write time. Defaults keep pre-existing rows valid as 'direct' origins.
ALTER TABLE folder_key_grants
ADD COLUMN delegated_by_npub TEXT;

ALTER TABLE folder_key_grants
ADD COLUMN origin_kind TEXT NOT NULL DEFAULT 'direct'
CHECK (origin_kind IN ('direct', 'invitation', 'approval', 'bootstrap'));

ALTER TABLE folder_key_grants
ADD COLUMN origin_ref TEXT;

ALTER TABLE folder_key_grants
ADD COLUMN roster_revision INTEGER
CHECK (roster_revision IS NULL OR roster_revision >= 0);

ALTER TABLE brain_members
ADD COLUMN delegated_by_npub TEXT;

ALTER TABLE brain_members
ADD COLUMN origin_kind TEXT NOT NULL DEFAULT 'direct'
CHECK (origin_kind IN ('direct', 'invitation', 'approval', 'bootstrap'));

ALTER TABLE brain_members
ADD COLUMN origin_ref TEXT;

-- Plan-linked invitations remember which Invitation Plan resolved them and at
-- what account roster revision, so acceptance can re-check and narrow only.
ALTER TABLE brain_invitations
ADD COLUMN origin_ref TEXT;

ALTER TABLE brain_invitations
ADD COLUMN roster_revision INTEGER
CHECK (roster_revision IS NULL OR roster_revision >= 0);

-- Immutable Invitation Plans: the resolved invite set previewed at preflight
-- and committed as per-principal Brain Invitations. A plan is not a stored
-- cohort entity; it grants nothing by itself and only records the resolution
-- the inviter committed to.
CREATE TABLE brain_invitation_plans (
    id TEXT PRIMARY KEY NOT NULL,
    brain_id TEXT NOT NULL,
    plan_hash TEXT NOT NULL,
    inviter_npub TEXT NOT NULL,
    workos_user_id TEXT,
    human_email TEXT NOT NULL,
    human_npub TEXT,
    agents_json TEXT NOT NULL,
    exclusions_json TEXT NOT NULL,
    roster_revision INTEGER
        CHECK (roster_revision IS NULL OR roster_revision >= 0),
    status TEXT NOT NULL CHECK (status IN ('pending', 'committed')),
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (brain_id) REFERENCES brains(id) ON DELETE CASCADE
);
"#;

const SCHEMA_V21: &str = r#"
-- V6 created this index before V10 renamed vault_invitations to
-- brain_invitations. SQLite preserved the old index name through the rename.
DROP INDEX IF EXISTS vault_invitations_pending_email_target;
"#;

const SCHEMA_V20: &str = r#"
ALTER TABLE organization_folder_mounts RENAME TO folder_mounts;
ALTER TABLE folder_mounts RENAME COLUMN organization_brain_id TO destination_brain_id;

DROP TRIGGER IF EXISTS capacity_organization_mounts;
DROP TRIGGER IF EXISTS capacity_personal_mounts;

CREATE TABLE legacy_personal_mount_migrations (
    legacy_mount_id TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('migrated', 'repair')),
    destination_brain_id TEXT,
    connection_id TEXT,
    reason TEXT,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (destination_brain_id) REFERENCES brains(id) ON DELETE SET NULL,
    FOREIGN KEY (connection_id) REFERENCES shared_folder_connections(id) ON DELETE SET NULL
);
"#;

#[derive(Debug)]
struct LegacyPersonalMountRow {
    id: String,
    owner_npub: String,
    source_brain_id: String,
    source_folder_id: String,
    display_name: String,
    display_parent_folder_id: Option<String>,
    created_at: String,
    updated_at: String,
}

pub(crate) fn migrate_legacy_personal_mounts(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let rows = {
        let mut statement = tx.prepare(
            r#"
            SELECT id, owner_npub, source_brain_id, source_folder_id, display_name,
                   display_parent_folder_id, created_at, updated_at
            FROM personal_folder_mounts
            ORDER BY id
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok(LegacyPersonalMountRow {
                id: row.get(0)?,
                owner_npub: row.get(1)?,
                source_brain_id: row.get(2)?,
                source_folder_id: row.get(3)?,
                display_name: row.get(4)?,
                display_parent_folder_id: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    for row in rows {
        let destination_brains = {
            let mut statement = tx.prepare(
                "SELECT id FROM brains
                 WHERE kind = 'personal' AND owner_user_id = ?1
                 ORDER BY id",
            )?;
            let rows = statement.query_map(params![row.owner_npub], |result| result.get(0))?;
            rows.collect::<rusqlite::Result<Vec<String>>>()?
        };
        if destination_brains.len() != 1 {
            let reason = if destination_brains.is_empty() {
                "legacy Personal Mount owner has no Personal Brain"
            } else {
                "legacy Personal Mount owner resolves to multiple Personal Brains"
            };
            record_legacy_personal_mount_migration(
                tx,
                &row.id,
                "repair",
                None,
                None,
                Some(reason),
                &row.updated_at,
            )?;
            continue;
        }
        let destination_brain_id = &destination_brains[0];
        let source_exists = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM folders WHERE brain_id = ?1 AND id = ?2
             )",
            params![row.source_brain_id, row.source_folder_id],
            |result| result.get::<_, bool>(0),
        )?;
        if !source_exists {
            record_legacy_personal_mount_migration(
                tx,
                &row.id,
                "repair",
                Some(destination_brain_id),
                None,
                Some("legacy Personal Mount source Folder no longer exists"),
                &row.updated_at,
            )?;
            continue;
        }

        let existing_connection = tx
            .query_row(
                "SELECT id FROM shared_folder_connections
                 WHERE source_brain_id = ?1 AND source_folder_id = ?2
                   AND destination_brain_id = ?3",
                params![
                    row.source_brain_id,
                    row.source_folder_id,
                    destination_brain_id
                ],
                |result| result.get::<_, String>(0),
            )
            .optional()?;
        let connection_id =
            existing_connection.unwrap_or_else(|| format!("legacy-personal-connection-{}", row.id));
        let connection_id_collision = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM shared_folder_connections
                WHERE id = ?1
                  AND NOT (
                    source_brain_id = ?2 AND source_folder_id = ?3
                    AND destination_brain_id = ?4
                  )
             )",
            params![
                connection_id,
                row.source_brain_id,
                row.source_folder_id,
                destination_brain_id
            ],
            |result| result.get::<_, bool>(0),
        )?;
        if connection_id_collision {
            record_legacy_personal_mount_migration(
                tx,
                &row.id,
                "repair",
                Some(destination_brain_id),
                None,
                Some("legacy Personal Mount connection id collides with unrelated state"),
                &row.updated_at,
            )?;
            continue;
        }

        tx.execute(
            r#"
            INSERT OR IGNORE INTO shared_folder_connections (
                id, source_brain_id, source_folder_id, destination_brain_id,
                destination_admin_npub, status, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7)
            "#,
            params![
                connection_id,
                row.source_brain_id,
                row.source_folder_id,
                destination_brain_id,
                row.owner_npub,
                row.created_at,
                row.updated_at
            ],
        )?;

        let mount_id_collision = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM folder_mounts
                WHERE id = ?1
                  AND NOT (
                    destination_brain_id = ?2 AND source_brain_id = ?3
                    AND source_folder_id = ?4
                  )
             )",
            params![
                row.id,
                destination_brain_id,
                row.source_brain_id,
                row.source_folder_id
            ],
            |result| result.get::<_, bool>(0),
        )?;
        if mount_id_collision {
            return Err(StoreError::BrokenInvariant {
                reason: format!(
                    "legacy Personal Mount {} collides with an unrelated universal Mount",
                    row.id
                ),
            });
        }
        tx.execute(
            r#"
            INSERT OR IGNORE INTO folder_mounts (
                id, destination_brain_id, source_brain_id, source_folder_id, connection_id,
                display_name, display_parent_folder_id, created_by_npub, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                row.id,
                destination_brain_id,
                row.source_brain_id,
                row.source_folder_id,
                connection_id,
                row.display_name,
                row.display_parent_folder_id,
                row.owner_npub,
                row.created_at,
                row.updated_at
            ],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO folder_access (brain_id, folder_id, user_id)
             VALUES (?1, ?2, ?3)",
            params![row.source_brain_id, row.source_folder_id, row.owner_npub],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO folder_access_sources (
                brain_id, folder_id, user_id, source_kind, source_id, created_at
             ) VALUES (?1, ?2, ?3, 'mount', ?4, ?5)",
            params![
                row.source_brain_id,
                row.source_folder_id,
                row.owner_npub,
                connection_id,
                row.created_at
            ],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO shared_folder_connection_members (
                connection_id, member_npub, created_at, manages_folder_access
             ) VALUES (?1, ?2, ?3, 0)",
            params![connection_id, row.owner_npub, row.created_at],
        )?;
        record_legacy_personal_mount_migration(
            tx,
            &row.id,
            "migrated",
            Some(destination_brain_id),
            Some(&connection_id),
            None,
            &row.updated_at,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_legacy_personal_mount_migration(
    tx: &Transaction<'_>,
    legacy_mount_id: &str,
    status: &str,
    destination_brain_id: Option<&str>,
    connection_id: Option<&str>,
    reason: Option<&str>,
    updated_at: &str,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT INTO legacy_personal_mount_migrations (
            legacy_mount_id, status, destination_brain_id, connection_id, reason, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(legacy_mount_id) DO UPDATE SET
            status = excluded.status,
            destination_brain_id = excluded.destination_brain_id,
            connection_id = excluded.connection_id,
            reason = excluded.reason,
            updated_at = excluded.updated_at
        "#,
        params![
            legacy_mount_id,
            status,
            destination_brain_id,
            connection_id,
            reason,
            updated_at
        ],
    )?;
    Ok(())
}

const SCHEMA_V19: &str = r#"
CREATE TABLE folder_access_sources (
    brain_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('direct', 'invitation', 'mount')),
    source_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (brain_id, folder_id, user_id, source_kind, source_id),
    FOREIGN KEY (brain_id, folder_id, user_id)
        REFERENCES folder_access(brain_id, folder_id, user_id) ON DELETE CASCADE
);

CREATE INDEX folder_access_sources_by_source
    ON folder_access_sources(source_kind, source_id);

CREATE TABLE legacy_folder_access_source_repairs (
    connection_id TEXT NOT NULL,
    brain_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('repair', 'resolved')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (connection_id, user_id),
    FOREIGN KEY (connection_id)
        REFERENCES shared_folder_connections(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO folder_access_sources (
    brain_id, folder_id, user_id, source_kind, source_id, created_at
)
SELECT connections.source_brain_id,
       connections.source_folder_id,
       members.member_npub,
       'mount',
       connections.id,
       members.created_at
FROM shared_folder_connection_members members
JOIN shared_folder_connections connections ON connections.id = members.connection_id
JOIN folder_access access
  ON access.brain_id = connections.source_brain_id
 AND access.folder_id = connections.source_folder_id
 AND access.user_id = members.member_npub
WHERE connections.status = 'active';

-- `manages_folder_access = 0` is authoritative for rows created after V17:
-- access predated that Mount. It is ambiguous for rows that V17 migrated,
-- because the new column defaulted every historical row to zero. Preserve
-- availability by retaining a direct source and surface the ambiguity for
-- deliberate repair instead of guessing that the Mount owns the only access.
INSERT OR IGNORE INTO folder_access_sources (
    brain_id, folder_id, user_id, source_kind, source_id, created_at
)
SELECT connections.source_brain_id,
       connections.source_folder_id,
       members.member_npub,
       'direct',
       'legacy-manages-false:' || connections.id,
       members.created_at
FROM shared_folder_connection_members members
JOIN shared_folder_connections connections ON connections.id = members.connection_id
JOIN folder_access access
  ON access.brain_id = connections.source_brain_id
 AND access.folder_id = connections.source_folder_id
 AND access.user_id = members.member_npub
WHERE connections.status = 'active'
  AND members.manages_folder_access = 0;

INSERT OR IGNORE INTO legacy_folder_access_source_repairs (
    connection_id, brain_id, folder_id, user_id, reason, status, created_at
)
SELECT connections.id,
       connections.source_brain_id,
       connections.source_folder_id,
       members.member_npub,
       'legacy manages_folder_access=false cannot distinguish preexisting direct access from a pre-V17 Mount-owned grant',
       'repair',
       members.created_at
FROM shared_folder_connection_members members
JOIN shared_folder_connections connections ON connections.id = members.connection_id
JOIN folder_access access
  ON access.brain_id = connections.source_brain_id
 AND access.folder_id = connections.source_folder_id
 AND access.user_id = members.member_npub
WHERE connections.status = 'active'
  AND members.manages_folder_access = 0;

INSERT OR IGNORE INTO folder_access_sources (
    brain_id, folder_id, user_id, source_kind, source_id, created_at
)
SELECT links.brain_id,
       links.folder_id,
       links.recipient_npub,
       'invitation',
       links.id,
       links.accepted_at
FROM share_links links
JOIN folder_access access
  ON access.brain_id = links.brain_id
 AND access.folder_id = links.folder_id
 AND access.user_id = links.recipient_npub
WHERE links.status = 'accepted'
  AND links.accepted_at IS NOT NULL;

INSERT OR IGNORE INTO folder_access_sources (
    brain_id, folder_id, user_id, source_kind, source_id, created_at
)
SELECT invitations.brain_id,
       json_each.value,
       COALESCE(invitations.claimed_by_npub, invitations.user_id),
       'invitation',
       invitations.id,
       invitations.accepted_at
FROM brain_invitations invitations, json_each(invitations.initial_folder_access_json)
JOIN folder_access access
  ON access.brain_id = invitations.brain_id
 AND access.folder_id = json_each.value
 AND access.user_id = COALESCE(invitations.claimed_by_npub, invitations.user_id)
WHERE invitations.status = 'accepted'
  AND invitations.accepted_at IS NOT NULL
  AND COALESCE(invitations.claimed_by_npub, invitations.user_id) IS NOT NULL;

INSERT OR IGNORE INTO folder_access_sources (
    brain_id, folder_id, user_id, source_kind, source_id, created_at
)
SELECT access.brain_id,
       access.folder_id,
       access.user_id,
       'direct',
       'migration-v19',
       '2026-07-26T00:00:00Z'
FROM folder_access access
WHERE NOT EXISTS (
    SELECT 1
    FROM folder_access_sources sources
    WHERE sources.brain_id = access.brain_id
      AND sources.folder_id = access.folder_id
      AND sources.user_id = access.user_id
);
"#;

const SCHEMA_V18: &str = r#"
ALTER TABLE brain_invitations
ADD COLUMN folder_only INTEGER NOT NULL DEFAULT 0
CHECK (folder_only IN (0, 1));

DROP INDEX IF EXISTS brain_invitations_pending_email_target;

CREATE UNIQUE INDEX brain_invitations_pending_email_brain_target
    ON brain_invitations(brain_id, invited_email)
    WHERE status = 'pending'
      AND target_kind = 'email_bootstrap'
      AND folder_only = 0;

CREATE UNIQUE INDEX brain_invitations_pending_email_folder_target
    ON brain_invitations(brain_id, invited_email, initial_folder_access_json)
    WHERE status = 'pending'
      AND target_kind = 'email_bootstrap'
      AND folder_only = 1;
"#;

const SCHEMA_V17: &str = r#"
ALTER TABLE shared_folder_connection_members
ADD COLUMN manages_folder_access INTEGER NOT NULL DEFAULT 0
CHECK (manages_folder_access IN (0, 1));
"#;

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

const SCHEMA_V15: &str = r#"
ALTER TABLE folder_access RENAME TO folder_access_legacy;

CREATE TABLE folder_access (
    brain_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    PRIMARY KEY (brain_id, folder_id, user_id),
    FOREIGN KEY (brain_id, folder_id) REFERENCES folders(brain_id, id)
        ON DELETE CASCADE
);

INSERT INTO folder_access (brain_id, folder_id, user_id)
SELECT brain_id, folder_id, user_id FROM folder_access_legacy;

DROP TABLE folder_access_legacy;

DELETE FROM brain_members
WHERE brain_id IN (SELECT id FROM brains WHERE kind = 'personal')
  AND NOT EXISTS (
      SELECT 1 FROM brains personal
      WHERE personal.id = brain_members.brain_id
        AND personal.owner_user_id = brain_members.user_id
  )
  AND NOT EXISTS (
      SELECT 1 FROM personal_agents agents
      WHERE agents.brain_id = brain_members.brain_id
        AND agents.agent_npub = brain_members.user_id
  )
  AND NOT EXISTS (
      SELECT 1 FROM brain_invitations invitations
      WHERE invitations.brain_id = brain_members.brain_id
        AND invitations.status = 'accepted'
        AND COALESCE(invitations.claimed_by_npub, invitations.user_id) = brain_members.user_id
  );

DELETE FROM brain_members
WHERE EXISTS (
    SELECT 1
    FROM shared_folder_connection_members participants
    JOIN shared_folder_connections connections
      ON connections.id = participants.connection_id
    WHERE connections.source_brain_id = brain_members.brain_id
      AND participants.member_npub = brain_members.user_id
)
AND NOT EXISTS (
    SELECT 1 FROM brain_admins admins
    WHERE admins.brain_id = brain_members.brain_id
      AND admins.user_id = brain_members.user_id
)
AND NOT EXISTS (
    SELECT 1 FROM brain_invitations invitations
    WHERE invitations.brain_id = brain_members.brain_id
      AND invitations.status = 'accepted'
      AND COALESCE(invitations.claimed_by_npub, invitations.user_id) = brain_members.user_id
);
"#;

const SCHEMA_V16: &str = r#"
ALTER TABLE shared_folder_invitations ADD COLUMN expires_at TEXT;

UPDATE shared_folder_invitations
SET expires_at = strftime('%Y-%m-%dT%H:%M:%SZ', created_at, '+7 days')
WHERE expires_at IS NULL;

UPDATE folders SET shared_folder_source = 0;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_adds_provenance_columns_with_direct_defaults() {
        let store = BrainStore::open_in_memory().unwrap();

        for (table, expected) in [
            (
                "folder_key_grants",
                vec![
                    "delegated_by_npub",
                    "origin_kind",
                    "origin_ref",
                    "roster_revision",
                ],
            ),
            (
                "brain_members",
                vec!["delegated_by_npub", "origin_kind", "origin_ref"],
            ),
            ("brain_invitations", vec!["origin_ref", "roster_revision"]),
        ] {
            let mut statement = store
                .conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            for column in expected {
                assert!(
                    columns.iter().any(|existing| existing == column),
                    "{table} is missing provenance column {column}"
                );
            }
        }

        let plans_table_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'brain_invitation_plans'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(plans_table_count, 1);
    }

    #[test]
    fn migration_v23_adds_departure_tables_and_extends_origin_checks() {
        let store = BrainStore::open_in_memory().unwrap();

        for table in [
            "brain_departure_fact_cursor",
            "brain_principal_revocations",
            "brain_departure_pending_rotations",
        ] {
            let count: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "missing V23 table {table}");
        }
        let cursor: i64 = store
            .conn
            .query_row(
                "SELECT last_applied_revision FROM brain_departure_fact_cursor WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cursor, 0);

        // The rebuilt V22 tables accept the new 'departure' origin kind and
        // keep rejecting unknown kinds.
        store
            .conn
            .execute_batch(
                r#"
                INSERT INTO brains (id, kind, name, owner_user_id, created_at)
                VALUES ('origin-check', 'organization', 'Origin Check', NULL, '2026-06-23T00:00:00Z');
                INSERT INTO folders (
                    brain_id, id, name, role, access, parent_folder_id, parent_folder_key,
                    path, current_key_version, shared_folder_source, setup_incomplete, created_at
                ) VALUES (
                    'origin-check', 'ops', 'Ops', 'vault_ops', 'admin_only', NULL, '',
                    'ops', 1, 0, 0, '2026-06-23T00:00:00Z'
                );
                INSERT INTO brain_members (brain_id, user_id, origin_kind)
                VALUES ('origin-check', 'npub-admin', 'departure');
                INSERT INTO folder_key_grants (
                    id, brain_id, folder_id, key_version, issuer_npub, recipient_npub,
                    format, wrapped_event_json, created_at, origin_kind
                ) VALUES (
                    'grant-departure-origin', 'origin-check', 'ops', 1, 'npub-admin', 'npub-admin',
                    'NIP-59', '{}', '2026-06-23T00:00:00Z', 'departure'
                );
                "#,
            )
            .unwrap();
        for statement in [
            "INSERT INTO brain_members (brain_id, user_id, origin_kind)
             VALUES ('origin-check', 'npub-other', 'bogus')",
            "INSERT INTO folder_key_grants (
                id, brain_id, folder_id, key_version, issuer_npub, recipient_npub,
                format, wrapped_event_json, created_at, origin_kind
             ) VALUES (
                'grant-bogus-origin', 'origin-check', 'ops', 1, 'npub-admin', 'npub-admin',
                'NIP-59', '{}', '2026-06-23T00:00:00Z', 'bogus'
             )",
        ] {
            assert!(
                store.conn.execute(statement, []).is_err(),
                "origin CHECK must reject bogus kinds: {statement}"
            );
        }
    }

    #[test]
    fn migration_v25_adds_pending_grant_wraps_table() {
        let store = BrainStore::open_in_memory().unwrap();

        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'brain_pending_grant_wraps'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "missing V25 table brain_pending_grant_wraps");

        store
            .conn
            .execute_batch(
                r#"
                INSERT INTO brains (id, kind, name, owner_user_id, created_at)
                VALUES ('wraps-check', 'organization', 'Wraps Check', NULL, '2026-06-23T00:00:00Z');
                INSERT INTO folders (
                    brain_id, id, name, role, access, parent_folder_id, parent_folder_key,
                    path, current_key_version, shared_folder_source, setup_incomplete, created_at
                ) VALUES (
                    'wraps-check', 'ops', 'Ops', 'vault_ops', 'admin_only', NULL, '',
                    'ops', 1, 0, 0, '2026-06-23T00:00:00Z'
                );
                INSERT INTO brain_pending_grant_wraps (
                    brain_id, folder_id, recipient_npub, key_version, reason, created_at
                ) VALUES (
                    'wraps-check', 'ops', 'npub-member', 1, 'invitation', '2026-06-23T00:00:00Z'
                );
                "#,
            )
            .unwrap();
        // Duplicate (brain, folder, recipient, key version) markers are ignored
        // by the marker write path and rejected by the table itself.
        assert!(
            store
                .conn
                .execute(
                    "INSERT INTO brain_pending_grant_wraps (
                        brain_id, folder_id, recipient_npub, key_version, reason, created_at
                     ) VALUES (
                        'wraps-check', 'ops', 'npub-member', 1, 'accept', '2026-06-24T00:00:00Z'
                     )",
                    [],
                )
                .is_err(),
            "the UNIQUE constraint must reject duplicate markers"
        );
        assert!(
            store
                .conn
                .execute(
                    "INSERT INTO brain_pending_grant_wraps (
                        brain_id, folder_id, recipient_npub, key_version, reason, created_at
                     ) VALUES (
                        'wraps-check', 'ops', 'npub-member', 2, 'bogus', '2026-06-24T00:00:00Z'
                     )",
                    [],
                )
                .is_err(),
            "the reason CHECK must reject unknown kinds"
        );
    }

    #[test]
    fn migration_removes_legacy_pending_email_invitation_index() {
        let mut store = BrainStore::open_in_memory().unwrap();
        store
            .conn
            .execute_batch(
                r#"
                DELETE FROM schema_migrations WHERE version = 21;
                DROP INDEX IF EXISTS vault_invitations_pending_email_target;
                CREATE UNIQUE INDEX vault_invitations_pending_email_target
                    ON brain_invitations(brain_id, invited_email)
                    WHERE status = 'pending' AND target_kind = 'email_bootstrap';
                "#,
            )
            .unwrap();

        store.apply_migrations().unwrap();

        let stale_index_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND name = 'vault_invitations_pending_email_target'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stale_index_count, 0);
    }

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
        assert_eq!(latest_version, 27);

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

    #[test]
    fn migration_15_preserves_folder_access_while_converting_limited_members_to_guests() {
        let mut store = BrainStore::open_in_memory().unwrap();
        store
            .conn
            .execute(
                "INSERT INTO brains (id, kind, name, owner_user_id, created_at)
                 VALUES ('personal', 'personal', 'Personal', 'npub-owner', ?1)",
                params![MIGRATION_TIMESTAMP],
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO brain_members (brain_id, user_id)
                 VALUES ('personal', 'npub-limited')",
                [],
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO folders (
                    brain_id, id, name, role, access, parent_folder_id,
                    parent_folder_key, path, current_key_version,
                    shared_folder_source, setup_incomplete, created_at
                 ) VALUES (
                    'personal', 'notes', 'Notes', 'folder', 'restricted', NULL,
                    '', 'Notes', 1, 0, 0, ?1
                 )",
                params![MIGRATION_TIMESTAMP],
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO folder_access (brain_id, folder_id, user_id)
                 VALUES ('personal', 'notes', 'npub-limited')",
                [],
            )
            .unwrap();
        store
            .conn
            .execute("DELETE FROM schema_migrations WHERE version = 15", [])
            .unwrap();

        store.apply_migrations().unwrap();

        let membership: bool = store
            .conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM brain_members
                    WHERE brain_id = 'personal' AND user_id = 'npub-limited'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let folder_access: bool = store
            .conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM folder_access
                    WHERE brain_id = 'personal'
                      AND folder_id = 'notes'
                      AND user_id = 'npub-limited'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!membership);
        assert!(folder_access);
    }
}
