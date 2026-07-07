use rusqlite::params;

use crate::{AuthStore, AuthStoreError, MIGRATION_TIMESTAMP};

impl AuthStore {
    pub(crate) fn apply_migrations(&mut self) -> Result<(), AuthStoreError> {
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

        tx.commit()?;
        Ok(())
    }
}

const SCHEMA_V1: &str = r#"
CREATE TABLE auth_challenges (
    nonce TEXT PRIMARY KEY NOT NULL,
    method TEXT NOT NULL,
    url TEXT NOT NULL,
    issued_at_unix_seconds INTEGER NOT NULL CHECK (issued_at_unix_seconds >= 0),
    expires_at_unix_seconds INTEGER NOT NULL CHECK (
        expires_at_unix_seconds > issued_at_unix_seconds
    ),
    expected_public_key_hex TEXT,
    consumed_at_unix_seconds INTEGER CHECK (
        consumed_at_unix_seconds IS NULL OR
        consumed_at_unix_seconds >= issued_at_unix_seconds
    ),
    consumed_by_public_key_hex TEXT,
    CHECK (
        (consumed_at_unix_seconds IS NULL AND consumed_by_public_key_hex IS NULL) OR
        (consumed_at_unix_seconds IS NOT NULL AND consumed_by_public_key_hex IS NOT NULL)
    )
);

CREATE TABLE auth_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    token_hash_hex TEXT NOT NULL UNIQUE,
    public_key_hex TEXT NOT NULL,
    nip05_identifier TEXT,
    authenticated_at_unix_seconds INTEGER NOT NULL CHECK (
        authenticated_at_unix_seconds >= 0
    ),
    issued_at_unix_seconds INTEGER NOT NULL CHECK (issued_at_unix_seconds >= 0),
    expires_at_unix_seconds INTEGER NOT NULL CHECK (
        expires_at_unix_seconds > issued_at_unix_seconds
    ),
    revoked_at_unix_seconds INTEGER CHECK (
        revoked_at_unix_seconds IS NULL OR
        revoked_at_unix_seconds >= issued_at_unix_seconds
    )
);

CREATE INDEX auth_sessions_by_token_hash
    ON auth_sessions(token_hash_hex);

CREATE TABLE nip05_bindings (
    public_key_hex TEXT NOT NULL,
    identifier TEXT NOT NULL,
    relays_json TEXT NOT NULL,
    verified_at_unix_seconds INTEGER NOT NULL CHECK (verified_at_unix_seconds >= 0),
    PRIMARY KEY (public_key_hex, identifier)
);

CREATE TABLE frostr_keysets (
    group_public_key_hex TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'active', 'rotating', 'disabled')),
    threshold INTEGER NOT NULL CHECK (threshold = 2),
    member_count INTEGER NOT NULL CHECK (member_count = 3),
    server_member_index INTEGER NOT NULL CHECK (server_member_index BETWEEN 1 AND 3),
    user_client_member_index INTEGER NOT NULL CHECK (user_client_member_index BETWEEN 1 AND 3),
    native_secure_storage_member_index INTEGER NOT NULL CHECK (
        native_secure_storage_member_index BETWEEN 1 AND 3
    ),
    server_share_package_ref TEXT NOT NULL,
    user_client_share_package_ref TEXT NOT NULL,
    native_secure_storage_share_package_ref TEXT NOT NULL,
    created_at_unix_seconds INTEGER NOT NULL CHECK (created_at_unix_seconds >= 0),
    activated_at_unix_seconds INTEGER CHECK (
        activated_at_unix_seconds IS NULL OR
        activated_at_unix_seconds >= created_at_unix_seconds
    ),
    CHECK (
        server_member_index != user_client_member_index AND
        server_member_index != native_secure_storage_member_index AND
        user_client_member_index != native_secure_storage_member_index
    ),
    CHECK (
        (status = 'pending' AND activated_at_unix_seconds IS NULL) OR
        (status IN ('active', 'rotating', 'disabled') AND
            activated_at_unix_seconds IS NOT NULL)
    )
);

CREATE TABLE agent_nostr_keys (
    agent_public_key_hex TEXT PRIMARY KEY NOT NULL,
    user_public_key_hex TEXT NOT NULL,
    created_at_unix_seconds INTEGER NOT NULL CHECK (created_at_unix_seconds >= 0),
    revoked_at_unix_seconds INTEGER CHECK (
        revoked_at_unix_seconds IS NULL OR
        revoked_at_unix_seconds >= created_at_unix_seconds
    ),
    CHECK (agent_public_key_hex != user_public_key_hex)
);

CREATE INDEX agent_nostr_keys_by_user
    ON agent_nostr_keys(user_public_key_hex);
"#;

fn migration_applied(tx: &rusqlite::Transaction<'_>, version: i64) -> Result<bool, AuthStoreError> {
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE version = ?1",
        params![version],
        |row| row.get(0),
    )?;
    Ok(count == 1)
}
