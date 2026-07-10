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
}
