//! Finite auth SQLite store and transaction boundary.

use std::error::Error;
use std::fmt;
use std::path::Path;

use finite_auth_core::{
    AgentNostrKeyBinding, AuthChallenge, AuthError, AuthNonce, AuthPrincipal, AuthSessionRecord,
    FrostrKeysetPlan, FrostrKeysetRecord, FrostrKeysetStatus, FrostrSharePackageRef,
    FrostrSharePlacement, FrostrShareRole, Nip05Identifier, SessionId, SessionTokenHash,
    VerifiedNip05,
};
use finite_nostr::NostrPublicKey;
use rusqlite::{Connection, OptionalExtension, params};

mod schema;

const MIGRATION_TIMESTAMP: &str = "2026-07-01T00:00:00.000Z";

/// Returns the crate name used in workspace status surfaces.
pub fn crate_name() -> &'static str {
    "finite-auth-store"
}

/// Store-level auth errors.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AuthStoreError {
    /// Core auth validation failed.
    Core(AuthError),
    /// SQLite returned an error.
    Database { message: String },
    /// A requested auth challenge does not exist.
    MissingChallenge { nonce: String },
    /// A challenge was already consumed.
    ChallengeReplayed { nonce: String },
    /// A challenge expired before use.
    ChallengeExpired {
        nonce: String,
        expires_at: u64,
        now: u64,
    },
    /// A challenge was bound to another public key.
    ChallengeSignerMismatch { expected: String, actual: String },
    /// A requested auth session does not exist.
    MissingSession { session_id: String },
    /// No session exists for the supplied token hash.
    MissingSessionForToken,
    /// Session is expired.
    SessionExpired { session_id: String },
    /// Session is revoked.
    SessionRevoked { session_id: String },
    /// Stored rows violate finite-auth invariants.
    BrokenInvariant { reason: String },
}

impl fmt::Display for AuthStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => write!(f, "{error}"),
            Self::Database { message } => write!(f, "database error: {message}"),
            Self::MissingChallenge { nonce } => write!(f, "missing auth challenge: {nonce}"),
            Self::ChallengeReplayed { nonce } => {
                write!(f, "replayed auth challenge: {nonce}")
            }
            Self::ChallengeExpired {
                nonce,
                expires_at,
                now,
            } => write!(
                f,
                "expired auth challenge: {nonce}; expires_at {expires_at}, now {now}"
            ),
            Self::ChallengeSignerMismatch { expected, actual } => write!(
                f,
                "auth challenge signer mismatch: expected {expected}, got {actual}"
            ),
            Self::MissingSession { session_id } => write!(f, "missing auth session: {session_id}"),
            Self::MissingSessionForToken => f.write_str("missing auth session for token"),
            Self::SessionExpired { session_id } => write!(f, "expired auth session: {session_id}"),
            Self::SessionRevoked { session_id } => write!(f, "revoked auth session: {session_id}"),
            Self::BrokenInvariant { reason } => write!(f, "broken auth invariant: {reason}"),
        }
    }
}

impl Error for AuthStoreError {}

impl From<AuthError> for AuthStoreError {
    fn from(value: AuthError) -> Self {
        Self::Core(value)
    }
}

impl From<rusqlite::Error> for AuthStoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database {
            message: value.to_string(),
        }
    }
}

/// SQLite-backed auth store.
pub struct AuthStore {
    conn: Connection,
}

impl AuthStore {
    /// Open an auth store at `path` and apply migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AuthStoreError> {
        let mut store = Self {
            conn: Connection::open(path)?,
        };
        store.apply_migrations()?;
        Ok(store)
    }

    /// Open an in-memory auth store for tests and local prototypes.
    pub fn in_memory() -> Result<Self, AuthStoreError> {
        let mut store = Self {
            conn: Connection::open_in_memory()?,
        };
        store.apply_migrations()?;
        Ok(store)
    }

    /// Insert a server-issued challenge.
    pub fn insert_challenge(&mut self, challenge: &AuthChallenge) -> Result<(), AuthStoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO auth_challenges (
                nonce,
                method,
                url,
                issued_at_unix_seconds,
                expires_at_unix_seconds,
                expected_public_key_hex
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                challenge.nonce().as_str(),
                challenge.method(),
                challenge.url(),
                to_i64("issued_at_unix_seconds", challenge.issued_at_unix_seconds())?,
                to_i64(
                    "expires_at_unix_seconds",
                    challenge.expires_at_unix_seconds()
                )?,
                challenge.expected_public_key().map(|key| key.to_hex())
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Consume a challenge once for the authenticated signer.
    pub fn consume_challenge(
        &mut self,
        nonce: &AuthNonce,
        signer: NostrPublicKey,
        now_unix_seconds: u64,
    ) -> Result<AuthChallenge, AuthStoreError> {
        let tx = self.conn.transaction()?;
        let row = tx
            .query_row(
                r#"
                SELECT
                    method,
                    url,
                    issued_at_unix_seconds,
                    expires_at_unix_seconds,
                    expected_public_key_hex,
                    consumed_at_unix_seconds
                FROM auth_challenges
                WHERE nonce = ?1
                "#,
                params![nonce.as_str()],
                |row| {
                    Ok(ChallengeRow {
                        method: row.get(0)?,
                        url: row.get(1)?,
                        issued_at_unix_seconds: row.get(2)?,
                        expires_at_unix_seconds: row.get(3)?,
                        expected_public_key_hex: row.get(4)?,
                        consumed_at_unix_seconds: row.get(5)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| AuthStoreError::MissingChallenge {
                nonce: nonce.as_str().to_string(),
            })?;

        if row.consumed_at_unix_seconds.is_some() {
            return Err(AuthStoreError::ChallengeReplayed {
                nonce: nonce.as_str().to_string(),
            });
        }

        let expires_at = from_i64("expires_at_unix_seconds", row.expires_at_unix_seconds)?;
        if now_unix_seconds >= expires_at {
            return Err(AuthStoreError::ChallengeExpired {
                nonce: nonce.as_str().to_string(),
                expires_at,
                now: now_unix_seconds,
            });
        }

        if let Some(expected) = &row.expected_public_key_hex {
            let actual = signer.to_hex();
            if expected != &actual {
                return Err(AuthStoreError::ChallengeSignerMismatch {
                    expected: expected.clone(),
                    actual,
                });
            }
        }

        let changed = tx.execute(
            r#"
            UPDATE auth_challenges
            SET consumed_at_unix_seconds = ?2,
                consumed_by_public_key_hex = ?3
            WHERE nonce = ?1
              AND consumed_at_unix_seconds IS NULL
            "#,
            params![
                nonce.as_str(),
                to_i64("consumed_at_unix_seconds", now_unix_seconds)?,
                signer.to_hex()
            ],
        )?;
        if changed != 1 {
            return Err(AuthStoreError::ChallengeReplayed {
                nonce: nonce.as_str().to_string(),
            });
        }

        let challenge = row.into_challenge(nonce.clone())?;
        tx.commit()?;
        Ok(challenge)
    }

    /// Insert a durable session. Bearer tokens must already be hashed.
    pub fn insert_session(&mut self, session: &AuthSessionRecord) -> Result<(), AuthStoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO auth_sessions (
                id,
                token_hash_hex,
                public_key_hex,
                nip05_identifier,
                authenticated_at_unix_seconds,
                issued_at_unix_seconds,
                expires_at_unix_seconds,
                revoked_at_unix_seconds
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                session.session_id().as_str(),
                session.token_hash().as_str(),
                session.principal().public_key().to_hex(),
                session
                    .principal()
                    .nip05()
                    .map(|identifier| identifier.as_str()),
                to_i64(
                    "authenticated_at_unix_seconds",
                    session.principal().authenticated_at_unix_seconds()
                )?,
                to_i64("issued_at_unix_seconds", session.issued_at_unix_seconds())?,
                to_i64("expires_at_unix_seconds", session.expires_at_unix_seconds())?,
                session
                    .revoked_at_unix_seconds()
                    .map(|value| to_i64("revoked_at_unix_seconds", value))
                    .transpose()?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Load an active session by token hash.
    pub fn load_active_session(
        &self,
        token_hash: &SessionTokenHash,
        now_unix_seconds: u64,
    ) -> Result<AuthSessionRecord, AuthStoreError> {
        let row = self
            .conn
            .query_row(
                r#"
                SELECT
                    id,
                    public_key_hex,
                    nip05_identifier,
                    authenticated_at_unix_seconds,
                    issued_at_unix_seconds,
                    expires_at_unix_seconds,
                    revoked_at_unix_seconds
                FROM auth_sessions
                WHERE token_hash_hex = ?1
                "#,
                params![token_hash.as_str()],
                |row| {
                    Ok(SessionRow {
                        session_id: row.get(0)?,
                        token_hash_hex: token_hash.as_str().to_string(),
                        public_key_hex: row.get(1)?,
                        nip05_identifier: row.get(2)?,
                        authenticated_at_unix_seconds: row.get(3)?,
                        issued_at_unix_seconds: row.get(4)?,
                        expires_at_unix_seconds: row.get(5)?,
                        revoked_at_unix_seconds: row.get(6)?,
                    })
                },
            )
            .optional()?
            .ok_or(AuthStoreError::MissingSessionForToken)?;
        let session = row.into_session()?;

        if session.is_revoked() {
            return Err(AuthStoreError::SessionRevoked {
                session_id: session.session_id().as_str().to_string(),
            });
        }
        if session.is_expired_at(now_unix_seconds) {
            return Err(AuthStoreError::SessionExpired {
                session_id: session.session_id().as_str().to_string(),
            });
        }

        Ok(session)
    }

    /// Revoke a session once.
    pub fn revoke_session(
        &mut self,
        session_id: &SessionId,
        revoked_at_unix_seconds: u64,
    ) -> Result<(), AuthStoreError> {
        let tx = self.conn.transaction()?;
        let revoked_at = tx
            .query_row(
                "SELECT revoked_at_unix_seconds FROM auth_sessions WHERE id = ?1",
                params![session_id.as_str()],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .ok_or_else(|| AuthStoreError::MissingSession {
                session_id: session_id.as_str().to_string(),
            })?;

        if revoked_at.is_some() {
            return Err(AuthStoreError::SessionRevoked {
                session_id: session_id.as_str().to_string(),
            });
        }

        tx.execute(
            "UPDATE auth_sessions SET revoked_at_unix_seconds = ?2 WHERE id = ?1",
            params![
                session_id.as_str(),
                to_i64("revoked_at_unix_seconds", revoked_at_unix_seconds)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Upsert a verified NIP-05 binding.
    pub fn record_nip05_binding(
        &mut self,
        verified: &VerifiedNip05,
        verified_at_unix_seconds: u64,
    ) -> Result<(), AuthStoreError> {
        let relays_json = serde_json::to_string(verified.relays()).map_err(|error| {
            AuthStoreError::BrokenInvariant {
                reason: error.to_string(),
            }
        })?;
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO nip05_bindings (
                public_key_hex,
                identifier,
                relays_json,
                verified_at_unix_seconds
            ) VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(public_key_hex, identifier) DO UPDATE SET
                relays_json = excluded.relays_json,
                verified_at_unix_seconds = excluded.verified_at_unix_seconds
            "#,
            params![
                verified.public_key().to_hex(),
                verified.identifier().as_str(),
                relays_json,
                to_i64("verified_at_unix_seconds", verified_at_unix_seconds)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Load a stored NIP-05 binding if present.
    pub fn load_nip05_binding(
        &self,
        public_key: NostrPublicKey,
        identifier: &Nip05Identifier,
    ) -> Result<Option<StoredNip05Binding>, AuthStoreError> {
        self.conn
            .query_row(
                r#"
                SELECT relays_json, verified_at_unix_seconds
                FROM nip05_bindings
                WHERE public_key_hex = ?1
                  AND identifier = ?2
                "#,
                params![public_key.to_hex(), identifier.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .map(|(relays_json, verified_at)| {
                let relays =
                    serde_json::from_str::<Vec<String>>(&relays_json).map_err(|error| {
                        AuthStoreError::BrokenInvariant {
                            reason: error.to_string(),
                        }
                    })?;
                Ok(StoredNip05Binding {
                    public_key,
                    identifier: identifier.clone(),
                    relays,
                    verified_at_unix_seconds: from_i64("verified_at_unix_seconds", verified_at)?,
                })
            })
            .transpose()
    }

    /// Insert Frostr keyset metadata. Share package material stays outside this layer.
    pub fn insert_frostr_keyset(
        &mut self,
        record: &FrostrKeysetRecord,
    ) -> Result<(), AuthStoreError> {
        validate_frostr_record_for_storage(record)?;
        let server_share = required_share(record.plan(), FrostrShareRole::Server)?;
        let user_client_share = required_share(record.plan(), FrostrShareRole::UserClient)?;
        let native_share = required_share(record.plan(), FrostrShareRole::NativeSecureStorage)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO frostr_keysets (
                group_public_key_hex,
                status,
                threshold,
                member_count,
                server_member_index,
                user_client_member_index,
                native_secure_storage_member_index,
                server_share_package_ref,
                user_client_share_package_ref,
                native_secure_storage_share_package_ref,
                created_at_unix_seconds,
                activated_at_unix_seconds
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                record.plan().group_public_key().to_hex(),
                record.status().as_str(),
                i64::from(record.plan().threshold()),
                i64::from(record.plan().member_count()),
                i64::from(server_share.member_index()),
                i64::from(user_client_share.member_index()),
                i64::from(native_share.member_index()),
                server_share.package_ref().as_str(),
                user_client_share.package_ref().as_str(),
                native_share.package_ref().as_str(),
                to_i64("created_at_unix_seconds", record.created_at_unix_seconds())?,
                record
                    .activated_at_unix_seconds()
                    .map(|value| to_i64("activated_at_unix_seconds", value))
                    .transpose()?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Load Frostr keyset metadata for a group public key.
    pub fn load_frostr_keyset(
        &self,
        group_public_key: NostrPublicKey,
    ) -> Result<Option<FrostrKeysetRecord>, AuthStoreError> {
        self.conn
            .query_row(
                r#"
                SELECT
                    status,
                    server_member_index,
                    user_client_member_index,
                    native_secure_storage_member_index,
                    server_share_package_ref,
                    user_client_share_package_ref,
                    native_secure_storage_share_package_ref,
                    created_at_unix_seconds,
                    activated_at_unix_seconds
                FROM frostr_keysets
                WHERE group_public_key_hex = ?1
                "#,
                params![group_public_key.to_hex()],
                |row| {
                    Ok(FrostrKeysetRow {
                        group_public_key,
                        status: row.get(0)?,
                        server_member_index: row.get(1)?,
                        user_client_member_index: row.get(2)?,
                        native_secure_storage_member_index: row.get(3)?,
                        server_share_package_ref: row.get(4)?,
                        user_client_share_package_ref: row.get(5)?,
                        native_secure_storage_share_package_ref: row.get(6)?,
                        created_at_unix_seconds: row.get(7)?,
                        activated_at_unix_seconds: row.get(8)?,
                    })
                },
            )
            .optional()?
            .map(FrostrKeysetRow::into_record)
            .transpose()
    }

    /// Insert a delegated agent Nostr key binding.
    pub fn insert_agent_key_binding(
        &mut self,
        binding: &AgentNostrKeyBinding,
    ) -> Result<(), AuthStoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO agent_nostr_keys (
                agent_public_key_hex,
                user_public_key_hex,
                created_at_unix_seconds,
                revoked_at_unix_seconds
            ) VALUES (?1, ?2, ?3, ?4)
            "#,
            params![
                binding.agent_public_key().to_hex(),
                binding.user_public_key().to_hex(),
                to_i64("created_at_unix_seconds", binding.created_at_unix_seconds())?,
                binding
                    .revoked_at_unix_seconds()
                    .map(|value| to_i64("revoked_at_unix_seconds", value))
                    .transpose()?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Load a delegated agent Nostr key binding.
    pub fn load_agent_key_binding(
        &self,
        agent_public_key: NostrPublicKey,
    ) -> Result<Option<AgentNostrKeyBinding>, AuthStoreError> {
        self.conn
            .query_row(
                r#"
                SELECT
                    user_public_key_hex,
                    created_at_unix_seconds,
                    revoked_at_unix_seconds
                FROM agent_nostr_keys
                WHERE agent_public_key_hex = ?1
                "#,
                params![agent_public_key.to_hex()],
                |row| {
                    Ok(AgentKeyRow {
                        agent_public_key,
                        user_public_key_hex: row.get(0)?,
                        created_at_unix_seconds: row.get(1)?,
                        revoked_at_unix_seconds: row.get(2)?,
                    })
                },
            )
            .optional()?
            .map(AgentKeyRow::into_binding)
            .transpose()
    }
}

/// Stored NIP-05 binding snapshot.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredNip05Binding {
    /// Public key the identifier mapped to.
    pub public_key: NostrPublicKey,
    /// NIP-05 identifier.
    pub identifier: Nip05Identifier,
    /// Relay hints observed with the binding.
    pub relays: Vec<String>,
    /// Verification time.
    pub verified_at_unix_seconds: u64,
}

#[derive(Debug)]
struct FrostrKeysetRow {
    group_public_key: NostrPublicKey,
    status: String,
    server_member_index: i64,
    user_client_member_index: i64,
    native_secure_storage_member_index: i64,
    server_share_package_ref: String,
    user_client_share_package_ref: String,
    native_secure_storage_share_package_ref: String,
    created_at_unix_seconds: i64,
    activated_at_unix_seconds: Option<i64>,
}

impl FrostrKeysetRow {
    fn into_record(self) -> Result<FrostrKeysetRecord, AuthStoreError> {
        let plan = FrostrKeysetPlan::new(
            self.group_public_key,
            vec![
                FrostrSharePlacement::new(
                    FrostrShareRole::Server,
                    to_u8("server_member_index", self.server_member_index)?,
                    FrostrSharePackageRef::new(self.server_share_package_ref)?,
                )?,
                FrostrSharePlacement::new(
                    FrostrShareRole::UserClient,
                    to_u8("user_client_member_index", self.user_client_member_index)?,
                    FrostrSharePackageRef::new(self.user_client_share_package_ref)?,
                )?,
                FrostrSharePlacement::new(
                    FrostrShareRole::NativeSecureStorage,
                    to_u8(
                        "native_secure_storage_member_index",
                        self.native_secure_storage_member_index,
                    )?,
                    FrostrSharePackageRef::new(self.native_secure_storage_share_package_ref)?,
                )?,
            ],
        )?;
        let status = FrostrKeysetStatus::parse(&self.status)?;
        let created_at = from_i64("created_at_unix_seconds", self.created_at_unix_seconds)?;
        let record = FrostrKeysetRecord::new(plan, FrostrKeysetStatus::Pending, created_at)?;

        match self.activated_at_unix_seconds {
            Some(activated_at) => {
                let active_record =
                    record.activated_at(from_i64("activated_at_unix_seconds", activated_at)?)?;
                Ok(active_record.with_status(status)?)
            }
            None if status == FrostrKeysetStatus::Pending => Ok(record),
            None => Err(AuthStoreError::BrokenInvariant {
                reason: "non-pending Frostr keyset is missing activated_at".to_string(),
            }),
        }
    }
}

#[derive(Debug)]
struct AgentKeyRow {
    agent_public_key: NostrPublicKey,
    user_public_key_hex: String,
    created_at_unix_seconds: i64,
    revoked_at_unix_seconds: Option<i64>,
}

impl AgentKeyRow {
    fn into_binding(self) -> Result<AgentNostrKeyBinding, AuthStoreError> {
        let user_public_key =
            NostrPublicKey::from_hex(&self.user_public_key_hex).map_err(AuthError::from)?;
        let binding = AgentNostrKeyBinding::new(
            user_public_key,
            self.agent_public_key,
            from_i64("created_at_unix_seconds", self.created_at_unix_seconds)?,
        )?;
        if let Some(revoked_at) = self.revoked_at_unix_seconds {
            Ok(binding.revoked_at(from_i64("revoked_at_unix_seconds", revoked_at)?)?)
        } else {
            Ok(binding)
        }
    }
}

#[derive(Debug)]
struct ChallengeRow {
    method: String,
    url: String,
    issued_at_unix_seconds: i64,
    expires_at_unix_seconds: i64,
    expected_public_key_hex: Option<String>,
    consumed_at_unix_seconds: Option<i64>,
}

impl ChallengeRow {
    fn into_challenge(self, nonce: AuthNonce) -> Result<AuthChallenge, AuthStoreError> {
        let mut challenge = AuthChallenge::new(
            nonce,
            self.method,
            self.url,
            from_i64("issued_at_unix_seconds", self.issued_at_unix_seconds)?,
            from_i64("expires_at_unix_seconds", self.expires_at_unix_seconds)?,
        )?;
        if let Some(public_key_hex) = self.expected_public_key_hex {
            let public_key = NostrPublicKey::from_hex(&public_key_hex).map_err(AuthError::from)?;
            challenge = challenge.with_expected_public_key(public_key);
        }
        Ok(challenge)
    }
}

#[derive(Debug)]
struct SessionRow {
    session_id: String,
    token_hash_hex: String,
    public_key_hex: String,
    nip05_identifier: Option<String>,
    authenticated_at_unix_seconds: i64,
    issued_at_unix_seconds: i64,
    expires_at_unix_seconds: i64,
    revoked_at_unix_seconds: Option<i64>,
}

impl SessionRow {
    fn into_session(self) -> Result<AuthSessionRecord, AuthStoreError> {
        let public_key = NostrPublicKey::from_hex(&self.public_key_hex).map_err(AuthError::from)?;
        let mut principal = AuthPrincipal::new(
            public_key,
            from_i64(
                "authenticated_at_unix_seconds",
                self.authenticated_at_unix_seconds,
            )?,
        );
        if let Some(identifier) = self.nip05_identifier {
            principal = principal.with_nip05(Nip05Identifier::parse(&identifier)?);
        }

        let mut session = AuthSessionRecord::new(
            SessionId::new(self.session_id)?,
            SessionTokenHash::parse(self.token_hash_hex)?,
            principal,
            from_i64("issued_at_unix_seconds", self.issued_at_unix_seconds)?,
            from_i64("expires_at_unix_seconds", self.expires_at_unix_seconds)?,
        )?;
        if let Some(revoked_at) = self.revoked_at_unix_seconds {
            session = session.revoked_at(from_i64("revoked_at_unix_seconds", revoked_at)?);
        }
        Ok(session)
    }
}

fn to_i64(field: &'static str, value: u64) -> Result<i64, AuthStoreError> {
    i64::try_from(value).map_err(|_| AuthStoreError::BrokenInvariant {
        reason: format!("{field} does not fit in SQLite INTEGER"),
    })
}

fn from_i64(field: &'static str, value: i64) -> Result<u64, AuthStoreError> {
    u64::try_from(value).map_err(|_| AuthStoreError::BrokenInvariant {
        reason: format!("{field} is negative"),
    })
}

fn to_u8(field: &'static str, value: i64) -> Result<u8, AuthStoreError> {
    u8::try_from(value).map_err(|_| AuthStoreError::BrokenInvariant {
        reason: format!("{field} does not fit in u8"),
    })
}

fn required_share(
    plan: &FrostrKeysetPlan,
    role: FrostrShareRole,
) -> Result<&FrostrSharePlacement, AuthStoreError> {
    plan.share_for_role(role)
        .ok_or_else(|| AuthStoreError::BrokenInvariant {
            reason: format!("missing Frostr {role} share"),
        })
}

fn validate_frostr_record_for_storage(record: &FrostrKeysetRecord) -> Result<(), AuthStoreError> {
    match (record.status(), record.activated_at_unix_seconds()) {
        (FrostrKeysetStatus::Pending, None) => Ok(()),
        (FrostrKeysetStatus::Pending, Some(_)) => Err(AuthStoreError::BrokenInvariant {
            reason: "pending Frostr keyset has activated_at".to_string(),
        }),
        (_, Some(_)) => Ok(()),
        (_, None) => Err(AuthStoreError::BrokenInvariant {
            reason: "non-pending Frostr keyset is missing activated_at".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use finite_auth_core::{
        AgentNostrKeyBinding, FrostrKeysetPlan, FrostrKeysetRecord, FrostrKeysetStatus,
        FrostrSharePackageRef, FrostrSharePlacement, FrostrShareRole, Nip05WellKnownDocument,
        SessionToken, SessionTokenHash,
    };
    use nostr::Keys;
    use tempfile::tempdir;

    const SECRET_KEY_HEX: &str = "6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";
    const AGENT_SECRET_KEY_HEX: &str =
        "5b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";
    const URL: &str = "https://api.finite.test/v1/auth/session";
    const NOW: u64 = 1_760_000_000;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "finite-auth-store");
    }

    #[test]
    fn consumes_challenge_once() {
        let mut store = AuthStore::in_memory().unwrap();
        let signer = test_public_key();
        let nonce = AuthNonce::new("auth-nonce-00001").unwrap();
        let challenge = AuthChallenge::new(nonce.clone(), "POST", URL, NOW, NOW + 60)
            .unwrap()
            .with_expected_public_key(signer);
        store.insert_challenge(&challenge).unwrap();

        let consumed = store.consume_challenge(&nonce, signer, NOW + 1).unwrap();

        assert_eq!(consumed.nonce(), &nonce);
        assert_eq!(
            store
                .consume_challenge(&nonce, signer, NOW + 2)
                .unwrap_err(),
            AuthStoreError::ChallengeReplayed {
                nonce: nonce.as_str().to_string()
            }
        );
    }

    #[test]
    fn rejects_expired_challenge() {
        let mut store = AuthStore::in_memory().unwrap();
        let signer = test_public_key();
        let nonce = AuthNonce::new("auth-nonce-00002").unwrap();
        let challenge = AuthChallenge::new(nonce.clone(), "GET", URL, NOW, NOW + 10).unwrap();
        store.insert_challenge(&challenge).unwrap();

        assert_eq!(
            store
                .consume_challenge(&nonce, signer, NOW + 10)
                .unwrap_err(),
            AuthStoreError::ChallengeExpired {
                nonce: nonce.as_str().to_string(),
                expires_at: NOW + 10,
                now: NOW + 10
            }
        );
    }

    #[test]
    fn loads_session_across_restart() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("auth.sqlite");
        let signer = test_public_key();
        let token = SessionToken::new("session-token-00000000000000000001").unwrap();
        let token_hash = SessionTokenHash::from_token(&token);
        let session_id = SessionId::new("session-0001").unwrap();

        {
            let mut store = AuthStore::open(&path).unwrap();
            let principal = AuthPrincipal::new(signer, NOW);
            let session = AuthSessionRecord::new(
                session_id.clone(),
                token_hash.clone(),
                principal,
                NOW,
                NOW + 3_600,
            )
            .unwrap();
            store.insert_session(&session).unwrap();
        }

        let store = AuthStore::open(&path).unwrap();
        let loaded = store.load_active_session(&token_hash, NOW + 10).unwrap();

        assert_eq!(loaded.session_id(), &session_id);
        assert_eq!(loaded.principal().public_key(), signer);
    }

    #[test]
    fn rejects_revoked_session() {
        let mut store = AuthStore::in_memory().unwrap();
        let signer = test_public_key();
        let token = SessionToken::new("session-token-00000000000000000002").unwrap();
        let token_hash = SessionTokenHash::from_token(&token);
        let session_id = SessionId::new("session-0002").unwrap();
        let principal = AuthPrincipal::new(signer, NOW);
        let session = AuthSessionRecord::new(
            session_id.clone(),
            token_hash.clone(),
            principal,
            NOW,
            NOW + 3_600,
        )
        .unwrap();
        store.insert_session(&session).unwrap();
        store.revoke_session(&session_id, NOW + 30).unwrap();

        assert_eq!(
            store
                .load_active_session(&token_hash, NOW + 31)
                .unwrap_err(),
            AuthStoreError::SessionRevoked {
                session_id: session_id.as_str().to_string()
            }
        );
    }

    #[test]
    fn records_nip05_binding() {
        let mut store = AuthStore::in_memory().unwrap();
        let public_key = test_public_key();
        let identifier = Nip05Identifier::parse("alice@example.com").unwrap();
        let document = format!(
            r#"{{
                "names": {{"alice": "{}"}},
                "relays": {{"{}": ["wss://relay.example.com"]}}
            }}"#,
            public_key.to_hex(),
            public_key.to_hex()
        );
        let verified = Nip05WellKnownDocument::from_json(document.as_bytes())
            .unwrap()
            .verify(&identifier, public_key)
            .unwrap();

        store.record_nip05_binding(&verified, NOW + 5).unwrap();
        let loaded = store
            .load_nip05_binding(public_key, &identifier)
            .unwrap()
            .unwrap();

        assert_eq!(loaded.public_key, public_key);
        assert_eq!(loaded.identifier, identifier);
        assert_eq!(loaded.relays, vec!["wss://relay.example.com".to_string()]);
        assert_eq!(loaded.verified_at_unix_seconds, NOW + 5);
    }

    #[test]
    fn loads_frostr_keyset_across_restart() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("auth.sqlite");
        let group_key = test_public_key();
        let record = FrostrKeysetRecord::new(
            test_frostr_plan(group_key),
            FrostrKeysetStatus::Pending,
            NOW,
        )
        .unwrap()
        .activated_at(NOW + 5)
        .unwrap()
        .with_status(FrostrKeysetStatus::Rotating)
        .unwrap();

        {
            let mut store = AuthStore::open(&path).unwrap();
            store.insert_frostr_keyset(&record).unwrap();
        }

        let store = AuthStore::open(&path).unwrap();
        let loaded = store.load_frostr_keyset(group_key).unwrap().unwrap();

        assert_eq!(loaded, record);
    }

    #[test]
    fn loads_agent_key_binding_across_restart() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("auth.sqlite");
        let user_key = test_public_key();
        let agent_key = agent_public_key();
        let binding = AgentNostrKeyBinding::new(user_key, agent_key, NOW).unwrap();

        {
            let mut store = AuthStore::open(&path).unwrap();
            store.insert_agent_key_binding(&binding).unwrap();
        }

        let store = AuthStore::open(&path).unwrap();
        let loaded = store.load_agent_key_binding(agent_key).unwrap().unwrap();

        assert_eq!(loaded, binding);
    }

    fn test_public_key() -> NostrPublicKey {
        let keys = Keys::parse(SECRET_KEY_HEX).unwrap();
        NostrPublicKey::from_protocol(keys.public_key())
    }

    fn agent_public_key() -> NostrPublicKey {
        let keys = Keys::parse(AGENT_SECRET_KEY_HEX).unwrap();
        NostrPublicKey::from_protocol(keys.public_key())
    }

    fn test_frostr_plan(group_key: NostrPublicKey) -> FrostrKeysetPlan {
        FrostrKeysetPlan::new(
            group_key,
            vec![
                FrostrSharePlacement::new(
                    FrostrShareRole::Server,
                    1,
                    FrostrSharePackageRef::new("server-share-ref-0001").unwrap(),
                )
                .unwrap(),
                FrostrSharePlacement::new(
                    FrostrShareRole::UserClient,
                    2,
                    FrostrSharePackageRef::new("client-share-ref-0001").unwrap(),
                )
                .unwrap(),
                FrostrSharePlacement::new(
                    FrostrShareRole::NativeSecureStorage,
                    3,
                    FrostrSharePackageRef::new("native-share-ref-0001").unwrap(),
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }
}
