use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use finitechat_proto::{RuntimeCommandRequestV1, RuntimeCommandResultV1};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::AgentdError;

#[derive(Debug, Clone)]
pub struct Ledger {
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandDecision {
    Execute,
    Resume,
    Replay(RuntimeCommandResultV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigOwnership {
    pub proposal_id: String,
    pub applied_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigHistory {
    pub proposal_id: String,
    pub path: String,
    pub before_bytes: Vec<u8>,
    pub applied_hash: String,
    pub rolled_back: bool,
}

#[derive(Clone, PartialEq)]
pub struct StartupSpecializationState {
    pub phase: StartupSpecializationPhase,
    pub proposal_id: String,
    pub before_bytes: Vec<u8>,
    pub vision_before: Option<serde_json::Value>,
    pub video_toolset_added: bool,
    pub exact_restore_eligible: bool,
    pub applied_bytes: Vec<u8>,
    pub transition_before_bytes: Vec<u8>,
    pub previous_applied_bytes: Option<Vec<u8>>,
    pub previous_proposal_id: Option<String>,
    pub previous_video_toolset_added: Option<bool>,
    pub vision_applied_hash: String,
    pub previous_vision_applied_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupSpecializationPhase {
    Applying,
    Active,
    Removing,
    SemanticRollback,
}

impl StartupSpecializationPhase {
    pub fn parse(value: &str) -> Result<Self, AgentdError> {
        match value {
            "applying" => Ok(Self::Applying),
            "active" => Ok(Self::Active),
            "removing" => Ok(Self::Removing),
            "semantic_rollback" => Ok(Self::SemanticRollback),
            _ => Err(AgentdError::Ledger(format!(
                "startup specialization state has invalid phase {value:?}"
            ))),
        }
    }
}

impl StartupSpecializationState {
    fn validate(self) -> Result<Self, AgentdError> {
        let previous_fields = [
            self.previous_applied_bytes.is_some(),
            self.previous_proposal_id.is_some(),
            self.previous_video_toolset_added.is_some(),
            self.previous_vision_applied_hash.is_some(),
        ];
        if previous_fields.iter().any(|present| *present)
            && !previous_fields.iter().all(|present| *present)
        {
            return Err(AgentdError::Ledger(
                "startup specialization prior-generation snapshot is incomplete".to_owned(),
            ));
        }
        if self.phase == StartupSpecializationPhase::SemanticRollback
            && !previous_fields.iter().all(|present| *present)
        {
            return Err(AgentdError::Ledger(
                "startup specialization semantic rollback has no complete prior generation"
                    .to_owned(),
            ));
        }
        if previous_fields.iter().all(|present| *present)
            && (self
                .previous_proposal_id
                .as_deref()
                .is_none_or(str::is_empty)
                || self
                    .previous_vision_applied_hash
                    .as_deref()
                    .is_none_or(str::is_empty))
        {
            return Err(AgentdError::Ledger(
                "startup specialization prior-generation identity metadata is empty".to_owned(),
            ));
        }
        if self.proposal_id.is_empty() || self.vision_applied_hash.is_empty() {
            return Err(AgentdError::Ledger(
                "startup specialization state has empty identity metadata".to_owned(),
            ));
        }
        Ok(self)
    }
}

impl PartialEq<&str> for StartupSpecializationPhase {
    fn eq(&self, other: &&str) -> bool {
        matches!(
            (self, *other),
            (Self::Applying, "applying")
                | (Self::Active, "active")
                | (Self::Removing, "removing")
                | (Self::SemanticRollback, "semantic_rollback")
        )
    }
}

impl std::fmt::Debug for StartupSpecializationState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StartupSpecializationState")
            .field("phase", &self.phase)
            .field("proposal_id", &self.proposal_id)
            .field("before_bytes", &"<redacted>")
            .field("vision_before", &"<redacted>")
            .field("video_toolset_added", &self.video_toolset_added)
            .field("exact_restore_eligible", &self.exact_restore_eligible)
            .field("applied_bytes", &"<redacted>")
            .field("transition_before_bytes", &"<redacted>")
            .field("previous_applied_bytes", &"<redacted>")
            .field("previous_proposal_id", &self.previous_proposal_id)
            .field(
                "previous_video_toolset_added",
                &self.previous_video_toolset_added,
            )
            .field("vision_applied_hash", &self.vision_applied_hash)
            .field(
                "previous_vision_applied_hash",
                &self.previous_vision_applied_hash,
            )
            .finish()
    }
}

pub struct StartupSpecializationApplyIntent<'a> {
    pub proposal_id: &'a str,
    pub before_bytes: &'a [u8],
    pub transition_before_bytes: &'a [u8],
    pub vision_before: Option<&'a serde_json::Value>,
    pub video_toolset_added: bool,
    pub exact_restore_eligible: bool,
    pub applied_bytes: &'a [u8],
    pub vision_applied_hash: &'a str,
}

impl Ledger {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, AgentdError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        }
        let ledger = Self { path };
        let connection = ledger.connection()?;
        connection.execute_batch(
            "
            PRAGMA journal_mode = DELETE;
            PRAGMA synchronous = FULL;
            CREATE TABLE IF NOT EXISTS command_ledger (
                request_id TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL,
                state TEXT NOT NULL CHECK (state IN ('pending', 'terminal')),
                result_json TEXT,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS config_history (
                proposal_id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                before_bytes BLOB NOT NULL,
                applied_hash TEXT NOT NULL,
                rolled_back INTEGER NOT NULL DEFAULT 0,
                applied_at_ms INTEGER NOT NULL,
                rolled_back_at_ms INTEGER
            );
            CREATE TABLE IF NOT EXISTS config_ownership (
                path TEXT PRIMARY KEY,
                proposal_id TEXT NOT NULL,
                applied_hash TEXT NOT NULL,
                FOREIGN KEY(proposal_id) REFERENCES config_history(proposal_id)
            );
            CREATE TABLE IF NOT EXISTS startup_specialization_state (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                phase TEXT NOT NULL CHECK (
                    phase IN ('applying', 'active', 'removing', 'semantic_rollback')
                ),
                proposal_id TEXT NOT NULL,
                before_bytes BLOB NOT NULL,
                vision_before_json TEXT,
                video_toolset_added INTEGER NOT NULL,
                exact_restore_eligible INTEGER NOT NULL DEFAULT 1,
                applied_bytes BLOB NOT NULL,
                transition_before_bytes BLOB NOT NULL,
                previous_applied_bytes BLOB,
                previous_proposal_id TEXT,
                previous_video_toolset_added INTEGER,
                vision_applied_hash TEXT NOT NULL,
                previous_vision_applied_hash TEXT,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS authorized_principals (
                account_id TEXT PRIMARY KEY,
                authorized_at_ms INTEGER NOT NULL
            );
            ",
        )?;
        drop(connection);
        fs::set_permissions(&ledger.path, fs::Permissions::from_mode(0o600))?;
        Ok(ledger)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn authorize_principal(&self, account_id: &str) -> Result<(), AgentdError> {
        if account_id.trim().is_empty() || account_id.len() > 256 {
            return Err(AgentdError::Ledger(
                "authorized Principal account id is invalid".to_owned(),
            ));
        }
        self.connection()?.execute(
            "INSERT OR IGNORE INTO authorized_principals(account_id, authorized_at_ms)
             VALUES (?1, ?2)",
            params![account_id, now_ms()],
        )?;
        Ok(())
    }

    pub fn principal_is_authorized(&self, account_id: &str) -> Result<bool, AgentdError> {
        self.connection()?
            .query_row(
                "SELECT 1 FROM authorized_principals WHERE account_id = ?1",
                [account_id],
                |_| Ok(true),
            )
            .optional()
            .map(|value| value.unwrap_or(false))
            .map_err(AgentdError::from)
    }

    pub fn authorized_principal_count(&self) -> Result<usize, AgentdError> {
        let count = self.connection()?.query_row(
            "SELECT COUNT(*) FROM authorized_principals",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        usize::try_from(count)
            .map_err(|_| AgentdError::Ledger("authorized Principal count is invalid".to_owned()))
    }

    pub fn begin_command(
        &self,
        request: &RuntimeCommandRequestV1,
    ) -> Result<CommandDecision, AgentdError> {
        let fingerprint = request_fingerprint(request)?;
        let connection = self.connection()?;
        let existing = connection
            .query_row(
                "SELECT fingerprint, state, result_json FROM command_ledger WHERE request_id = ?1",
                [&request.request_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((recorded_fingerprint, state, result_json)) = existing {
            if recorded_fingerprint != fingerprint {
                return Err(AgentdError::ConflictingRequestId(
                    request.request_id.clone(),
                ));
            }
            if state == "terminal" {
                let payload = result_json.ok_or_else(|| {
                    AgentdError::Ledger("terminal command is missing its result".to_owned())
                })?;
                return Ok(CommandDecision::Replay(serde_json::from_str(&payload)?));
            }
            return Ok(CommandDecision::Resume);
        }

        connection.execute(
            "INSERT INTO command_ledger(request_id, fingerprint, state, updated_at_ms)
             VALUES (?1, ?2, 'pending', ?3)",
            params![&request.request_id, fingerprint, now_ms()],
        )?;
        Ok(CommandDecision::Execute)
    }

    pub fn finish_command(
        &self,
        request_id: &str,
        result: &RuntimeCommandResultV1,
    ) -> Result<(), AgentdError> {
        let result_json = serde_json::to_string(result)?;
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE command_ledger
             SET state = 'terminal', result_json = ?2, updated_at_ms = ?3
             WHERE request_id = ?1",
            params![request_id, result_json, now_ms()],
        )?;
        if changed != 1 {
            return Err(AgentdError::Ledger(format!(
                "command {request_id} was not recorded before completion"
            )));
        }
        Ok(())
    }

    pub fn config_ownership(&self, path: &str) -> Result<Option<ConfigOwnership>, AgentdError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT proposal_id, applied_hash FROM config_ownership WHERE path = ?1",
                [path],
                |row| {
                    Ok(ConfigOwnership {
                        proposal_id: row.get(0)?,
                        applied_hash: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(AgentdError::from)
    }

    pub fn config_history(&self, proposal_id: &str) -> Result<Option<ConfigHistory>, AgentdError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT proposal_id, path, before_bytes, applied_hash, rolled_back
                 FROM config_history WHERE proposal_id = ?1",
                [proposal_id],
                |row| {
                    Ok(ConfigHistory {
                        proposal_id: row.get(0)?,
                        path: row.get(1)?,
                        before_bytes: row.get(2)?,
                        applied_hash: row.get(3)?,
                        rolled_back: row.get::<_, i64>(4)? != 0,
                    })
                },
            )
            .optional()
            .map_err(AgentdError::from)
    }

    pub fn record_config_apply(
        &self,
        proposal_id: &str,
        path: &str,
        before_bytes: &[u8],
        applied_hash: &str,
    ) -> Result<(), AgentdError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO config_history(
                proposal_id, path, before_bytes, applied_hash, applied_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![proposal_id, path, before_bytes, applied_hash, now_ms()],
        )?;
        transaction.execute(
            "INSERT INTO config_ownership(path, proposal_id, applied_hash)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET
                proposal_id = excluded.proposal_id,
                applied_hash = excluded.applied_hash",
            params![path, proposal_id, applied_hash],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn record_config_rollback(&self, proposal_id: &str, path: &str) -> Result<(), AgentdError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE config_history
             SET rolled_back = 1, rolled_back_at_ms = ?2
             WHERE proposal_id = ?1 AND rolled_back = 0",
            params![proposal_id, now_ms()],
        )?;
        if changed != 1 {
            return Err(AgentdError::Ledger(format!(
                "configuration proposal {proposal_id} is unavailable for rollback"
            )));
        }
        transaction.execute(
            "DELETE FROM config_ownership WHERE path = ?1 AND proposal_id = ?2",
            params![path, proposal_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn startup_specialization_state(
        &self,
    ) -> Result<Option<StartupSpecializationState>, AgentdError> {
        let connection = self.connection()?;
        let record = connection
            .query_row(
                "SELECT phase, proposal_id, before_bytes, vision_before_json,
                        video_toolset_added, exact_restore_eligible,
                        applied_bytes, transition_before_bytes, previous_applied_bytes,
                        previous_proposal_id, previous_video_toolset_added,
                        vision_applied_hash, previous_vision_applied_hash
                 FROM startup_specialization_state WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)? != 0,
                        row.get::<_, i64>(5)? != 0,
                        row.get::<_, Vec<u8>>(6)?,
                        row.get::<_, Vec<u8>>(7)?,
                        row.get::<_, Option<Vec<u8>>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<i64>>(10)?.map(|value| value != 0),
                        row.get::<_, String>(11)?,
                        row.get::<_, Option<String>>(12)?,
                    ))
                },
            )
            .optional()?;
        record
            .map(
                |(
                    phase,
                    proposal_id,
                    before_bytes,
                    vision_before,
                    video_toolset_added,
                    exact_restore_eligible,
                    applied_bytes,
                    transition_before_bytes,
                    previous_applied_bytes,
                    previous_proposal_id,
                    previous_video_toolset_added,
                    vision_applied_hash,
                    previous_vision_applied_hash,
                )| {
                    StartupSpecializationState {
                        phase: StartupSpecializationPhase::parse(&phase)?,
                        proposal_id,
                        before_bytes,
                        vision_before: vision_before
                            .map(|value| serde_json::from_str(&value))
                            .transpose()?,
                        video_toolset_added,
                        exact_restore_eligible,
                        applied_bytes,
                        transition_before_bytes,
                        previous_applied_bytes,
                        previous_proposal_id,
                        previous_video_toolset_added,
                        vision_applied_hash,
                        previous_vision_applied_hash,
                    }
                    .validate()
                },
            )
            .transpose()
    }

    pub fn begin_startup_specialization_apply(
        &self,
        intent: &StartupSpecializationApplyIntent<'_>,
    ) -> Result<(), AgentdError> {
        let vision_before = intent
            .vision_before
            .map(serde_json::to_string)
            .transpose()?;
        self.connection()?.execute(
            "INSERT INTO startup_specialization_state(
                singleton, phase, proposal_id, before_bytes, vision_before_json,
                video_toolset_added, exact_restore_eligible, applied_bytes,
                transition_before_bytes, vision_applied_hash, updated_at_ms
             ) VALUES (1, 'applying', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(singleton) DO UPDATE SET
                phase = 'applying',
                previous_applied_bytes = excluded.transition_before_bytes,
                previous_proposal_id = startup_specialization_state.proposal_id,
                previous_video_toolset_added =
                    startup_specialization_state.video_toolset_added,
                previous_vision_applied_hash =
                    startup_specialization_state.vision_applied_hash,
                proposal_id = excluded.proposal_id,
                video_toolset_added =
                    startup_specialization_state.video_toolset_added
                    OR excluded.video_toolset_added,
                exact_restore_eligible =
                    startup_specialization_state.exact_restore_eligible
                    AND excluded.transition_before_bytes =
                        startup_specialization_state.applied_bytes,
                applied_bytes = excluded.applied_bytes,
                transition_before_bytes = excluded.transition_before_bytes,
                vision_applied_hash = excluded.vision_applied_hash,
                updated_at_ms = excluded.updated_at_ms",
            params![
                intent.proposal_id,
                intent.before_bytes,
                vision_before,
                intent.video_toolset_added,
                intent.exact_restore_eligible,
                intent.applied_bytes,
                intent.transition_before_bytes,
                intent.vision_applied_hash,
                now_ms(),
            ],
        )?;
        Ok(())
    }

    pub fn finish_startup_specialization_apply(&self) -> Result<(), AgentdError> {
        let changed = self.connection()?.execute(
            "UPDATE startup_specialization_state
             SET phase = 'active', updated_at_ms = ?1
             WHERE singleton = 1 AND phase = 'applying'",
            [now_ms()],
        )?;
        if changed != 1 {
            return Err(AgentdError::Ledger(
                "startup specialization activation intent is unavailable".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn update_interrupted_startup_specialization_apply(
        &self,
        applied_bytes: &[u8],
        rollback_bytes: &[u8],
    ) -> Result<(), AgentdError> {
        let changed = self.connection()?.execute(
            "UPDATE startup_specialization_state
             SET applied_bytes = ?1,
                 transition_before_bytes = ?2,
                 previous_applied_bytes =
                     CASE WHEN previous_applied_bytes IS NULL
                          THEN NULL ELSE ?2 END,
                 exact_restore_eligible = 0,
                 updated_at_ms = ?3
             WHERE singleton = 1 AND phase = 'applying'",
            params![applied_bytes, rollback_bytes, now_ms()],
        )?;
        if changed != 1 {
            return Err(AgentdError::Ledger(
                "startup specialization activation intent is unavailable".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn confirm_startup_specialization_semantics(&self) -> Result<(), AgentdError> {
        let changed = self.connection()?.execute(
            "UPDATE startup_specialization_state
             SET transition_before_bytes = applied_bytes,
                 previous_applied_bytes = NULL,
                 previous_proposal_id = NULL,
                 previous_video_toolset_added = NULL,
                 previous_vision_applied_hash = NULL,
                 updated_at_ms = ?1
             WHERE singleton = 1 AND phase = 'active'",
            [now_ms()],
        )?;
        if changed != 1 {
            return Err(AgentdError::Ledger(
                "active startup specialization is unavailable for semantic confirmation".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn begin_unverified_startup_specialization_rollback(&self) -> Result<(), AgentdError> {
        let changed = self.connection()?.execute(
            "UPDATE startup_specialization_state
             SET phase = 'semantic_rollback', updated_at_ms = ?1
             WHERE singleton = 1 AND phase = 'active'
               AND previous_applied_bytes IS NOT NULL",
            [now_ms()],
        )?;
        if changed != 1 {
            return Err(AgentdError::Ledger(
                "startup specialization has no unverified rotation to roll back".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn finish_unverified_startup_specialization_rollback(
        &self,
        applied_bytes: &[u8],
    ) -> Result<(), AgentdError> {
        let changed = self.connection()?.execute(
            "UPDATE startup_specialization_state
             SET phase = 'active',
                 exact_restore_eligible =
                     exact_restore_eligible AND previous_applied_bytes = ?1,
                 applied_bytes = ?1,
                 transition_before_bytes = ?1,
                 proposal_id = previous_proposal_id,
                 video_toolset_added = previous_video_toolset_added,
                 vision_applied_hash = previous_vision_applied_hash,
                 previous_applied_bytes = NULL,
                 previous_proposal_id = NULL,
                 previous_video_toolset_added = NULL,
                 previous_vision_applied_hash = NULL,
                 updated_at_ms = ?2
             WHERE singleton = 1 AND phase = 'semantic_rollback'
               AND previous_applied_bytes IS NOT NULL",
            params![applied_bytes, now_ms()],
        )?;
        if changed != 1 {
            return Err(AgentdError::Ledger(
                "startup specialization has no unverified rotation to roll back".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn cancel_unverified_startup_specialization_rollback(&self) -> Result<(), AgentdError> {
        let changed = self.connection()?.execute(
            "UPDATE startup_specialization_state
             SET phase = 'active', updated_at_ms = ?1
             WHERE singleton = 1 AND phase = 'semantic_rollback'",
            [now_ms()],
        )?;
        if changed != 1 {
            return Err(AgentdError::Ledger(
                "startup specialization semantic rollback intent is unavailable".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn cancel_startup_specialization_apply(&self) -> Result<(), AgentdError> {
        let state = self.startup_specialization_state()?.ok_or_else(|| {
            AgentdError::Ledger(
                "startup specialization activation intent is unavailable".to_owned(),
            )
        })?;
        if state.previous_applied_bytes.is_some() {
            self.connection()?.execute(
                "UPDATE startup_specialization_state
                 SET phase = 'active',
                     applied_bytes = previous_applied_bytes,
                     proposal_id = previous_proposal_id,
                     video_toolset_added = previous_video_toolset_added,
                     vision_applied_hash = previous_vision_applied_hash,
                     previous_applied_bytes = NULL,
                     previous_proposal_id = NULL,
                     previous_video_toolset_added = NULL,
                     transition_before_bytes = previous_applied_bytes,
                     previous_vision_applied_hash = NULL,
                     updated_at_ms = ?1
                 WHERE singleton = 1 AND phase = 'applying'",
                [now_ms()],
            )?;
        } else {
            self.clear_startup_specialization()?;
        }
        Ok(())
    }

    pub fn begin_startup_specialization_removal(
        &self,
        transition_before_bytes: &[u8],
    ) -> Result<(), AgentdError> {
        let changed = self.connection()?.execute(
            "UPDATE startup_specialization_state
             SET phase = 'removing', transition_before_bytes = ?1, updated_at_ms = ?2
             WHERE singleton = 1 AND phase = 'active'",
            params![transition_before_bytes, now_ms()],
        )?;
        if changed != 1 {
            return Err(AgentdError::Ledger(
                "active startup specialization is unavailable for removal".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn cancel_startup_specialization_removal(&self) -> Result<(), AgentdError> {
        let changed = self.connection()?.execute(
            "UPDATE startup_specialization_state
             SET phase = 'active', updated_at_ms = ?1
             WHERE singleton = 1 AND phase = 'removing'",
            [now_ms()],
        )?;
        if changed != 1 {
            return Err(AgentdError::Ledger(
                "startup specialization removal intent is unavailable".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn clear_startup_specialization(&self) -> Result<(), AgentdError> {
        self.connection()?.execute(
            "DELETE FROM startup_specialization_state WHERE singleton = 1",
            [],
        )?;
        Ok(())
    }

    fn connection(&self) -> Result<Connection, AgentdError> {
        Connection::open(&self.path).map_err(AgentdError::from)
    }
}

pub fn request_fingerprint(request: &RuntimeCommandRequestV1) -> Result<String, AgentdError> {
    let encoded = serde_json::to_vec(request)?;
    Ok(hex_digest(&encoded))
}

pub fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use finitechat_proto::{
        RuntimeCommandJsonPayloadV1, RuntimeCommandPayloadKindV1, RuntimeCommandTargetV1,
        RuntimeCommandTerminalStatusV1,
    };

    use super::*;

    fn request(request_id: &str, body: &[u8]) -> RuntimeCommandRequestV1 {
        RuntimeCommandRequestV1 {
            payload_kind: RuntimeCommandPayloadKindV1::Request,
            request_id: request_id.to_owned(),
            command: "agent.status.inspect".to_owned(),
            target: RuntimeCommandTargetV1 {
                account_id: "agent-account".to_owned(),
                device_id: Some("agent-device".to_owned()),
            },
            resource_key: None,
            body: RuntimeCommandJsonPayloadV1 {
                schema: "finite.agent.status.request.v1".to_owned(),
                json_payload: body.to_vec(),
            },
        }
    }

    fn result(request_id: &str) -> RuntimeCommandResultV1 {
        RuntimeCommandResultV1 {
            payload_kind: RuntimeCommandPayloadKindV1::Result,
            request_id: request_id.to_owned(),
            status: RuntimeCommandTerminalStatusV1::Succeeded,
            body: Some(RuntimeCommandJsonPayloadV1 {
                schema: "finite.agent.command.result.v1".to_owned(),
                json_payload: br#"{"ok":true}"#.to_vec(),
            }),
            error: None,
            clears_activity: Vec::new(),
        }
    }

    #[test]
    fn command_ledger_resumes_pending_and_replays_terminal_results() {
        let directory = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(directory.path().join("agentd.sqlite3")).unwrap();
        let request = request("request-1", br#"{}"#);

        assert_eq!(
            ledger.begin_command(&request).unwrap(),
            CommandDecision::Execute
        );
        assert_eq!(
            ledger.begin_command(&request).unwrap(),
            CommandDecision::Resume,
            "a daemon restart must resume the durable pending command"
        );

        let result = result("request-1");
        ledger.finish_command("request-1", &result).unwrap();
        assert_eq!(
            ledger.begin_command(&request).unwrap(),
            CommandDecision::Replay(result),
            "redelivery after a sent-but-unacked result must not execute again"
        );
    }

    #[test]
    fn command_ledger_rejects_request_id_reuse_with_different_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(directory.path().join("agentd.sqlite3")).unwrap();
        ledger
            .begin_command(&request("request-1", br#"{}"#))
            .unwrap();

        let error = ledger
            .begin_command(&request("request-1", br#"{"changed":true}"#))
            .unwrap_err();
        assert!(matches!(error, AgentdError::ConflictingRequestId(_)));
    }

    #[test]
    fn authorized_principals_are_durable_and_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agentd.sqlite3");
        let ledger = Ledger::open(&path).unwrap();
        assert_eq!(ledger.authorized_principal_count().unwrap(), 0);
        ledger.authorize_principal("user-account").unwrap();
        ledger.authorize_principal("user-account").unwrap();
        assert!(ledger.principal_is_authorized("user-account").unwrap());
        assert_eq!(ledger.authorized_principal_count().unwrap(), 1);

        let reopened = Ledger::open(path).unwrap();
        assert!(reopened.principal_is_authorized("user-account").unwrap());
    }

    #[test]
    fn startup_specialization_state_rejects_incomplete_prior_generation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agentd.sqlite3");
        let ledger = Ledger::open(&path).unwrap();
        Connection::open(&path)
            .unwrap()
            .execute(
                "INSERT INTO startup_specialization_state(
                    singleton, phase, proposal_id, before_bytes, vision_before_json,
                    video_toolset_added, exact_restore_eligible, applied_bytes,
                    transition_before_bytes, previous_applied_bytes,
                    vision_applied_hash, updated_at_ms
                 ) VALUES (1, 'active', 'proposal', X'01', NULL, 0, 1, X'02',
                           X'02', X'03', 'hash', 1)",
                [],
            )
            .unwrap();

        assert!(matches!(
            ledger.startup_specialization_state(),
            Err(AgentdError::Ledger(_))
        ));

        let connection = Connection::open(&path).unwrap();
        connection
            .execute("DELETE FROM startup_specialization_state", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO startup_specialization_state(
                    singleton, phase, proposal_id, before_bytes, vision_before_json,
                    video_toolset_added, exact_restore_eligible, applied_bytes,
                    transition_before_bytes, previous_applied_bytes,
                    previous_proposal_id, previous_video_toolset_added,
                    vision_applied_hash, previous_vision_applied_hash, updated_at_ms
                 ) VALUES (1, 'active', 'current', X'01', NULL, 0, 1, X'02',
                           X'02', X'03', '', 0, 'current-hash', '', 1)",
                [],
            )
            .unwrap();
        assert!(matches!(
            ledger.startup_specialization_state(),
            Err(AgentdError::Ledger(_))
        ));
    }
}
