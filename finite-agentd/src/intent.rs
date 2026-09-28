//! The single-slot inference intent record (`agentd/inference-intent.json`) and
//! the one admission function every command calls before doing any work.
//!
//! The record never holds a key, token, code, or verifier.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::AgentdError;

pub(crate) const INTENT_FILE_NAME: &str = "inference-intent.json";
const INTENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct IntentRecord {
    pub v: u32,
    pub id: String,
    pub kind: IntentKind,
    pub route: IntentRoute,
    pub model: Option<String>,
    pub phase: IntentPhase,
    pub state: IntentState,
    pub error_code: Option<String>,
    pub attempts: u32,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum IntentKind {
    Select,
    Activate,
    Disconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum IntentRoute {
    FinitePrivate,
    Openrouter,
    OpenaiCodex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum IntentPhase {
    Accepted,
    ConfigWritten,
    Restarting,
    Verifying,
    LoginCancelled,
    RouteSwitched,
    CredentialRemoved,
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum IntentState {
    Running,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdmitCommand {
    Status,
    OpenRouterUsage,
    CodexModels,
    CodexLoginCancel,
    CodexLoginStart,
    V1Apply,
    Select,
    Connect,
    Disconnect(IntentRoute),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Proceed,
    ReplaceFailed,
    ResumeFailed,
}

const SELECT_PHASES: &[IntentPhase] = &[
    IntentPhase::Accepted,
    IntentPhase::ConfigWritten,
    IntentPhase::Restarting,
    IntentPhase::Verifying,
];

const DISCONNECT_PHASES: &[IntentPhase] = &[
    IntentPhase::Accepted,
    IntentPhase::LoginCancelled,
    IntentPhase::RouteSwitched,
    IntentPhase::CredentialRemoved,
    IntentPhase::Cleanup,
    IntentPhase::Verifying,
];

impl IntentKind {
    /// The kind's phases, in their only permitted (forward) order.
    pub(crate) fn phases(self) -> &'static [IntentPhase] {
        match self {
            Self::Select | Self::Activate => SELECT_PHASES,
            Self::Disconnect => DISCONNECT_PHASES,
        }
    }
}

impl IntentRecord {
    /// A fresh `running` record at phase `accepted` with a new `op_<32 hex>` id.
    pub(crate) fn new(
        kind: IntentKind,
        route: IntentRoute,
        model: Option<String>,
    ) -> Result<Self, AgentdError> {
        let now = now_ms();
        Ok(Self {
            v: INTENT_VERSION,
            id: new_operation_id()?,
            kind,
            route,
            model,
            phase: IntentPhase::Accepted,
            state: IntentState::Running,
            error_code: None,
            attempts: 0,
            created_at_ms: now,
            updated_at_ms: now,
        })
    }

    fn check(&self) -> Result<(), String> {
        if self.v != INTENT_VERSION {
            return Err(format!("unsupported version {}", self.v));
        }
        if !is_operation_id(&self.id) {
            return Err("malformed id".to_owned());
        }
        if !self.kind.phases().contains(&self.phase) {
            return Err("phase does not belong to kind".to_owned());
        }
        Ok(())
    }
}

/// The intent path for an agent home (`$FINITECHAT_HOME/agentd/inference-intent.json`).
pub(crate) fn intent_path(agent_home: &Path) -> PathBuf {
    agent_home.join("agentd").join(INTENT_FILE_NAME)
}

/// Reads the record. A missing file is `Ok(None)`. A file that does not parse
/// as a version-1 record is renamed to `<name>.corrupt-<unix ms>`, logged, and
/// treated as absent.
pub(crate) fn load(path: &Path) -> Result<Option<IntentRecord>, AgentdError> {
    let mut bytes = Vec::new();
    match File::open(path) {
        Ok(mut file) => {
            file.read_to_end(&mut bytes)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let problem = match serde_json::from_slice::<IntentRecord>(&bytes) {
        Ok(record) => match record.check() {
            Ok(()) => return Ok(Some(record)),
            Err(problem) => problem,
        },
        // serde's message can quote the offending value; log only its position.
        Err(error) => format!(
            "not a valid record (line {}, column {})",
            error.line(),
            error.column()
        ),
    };
    let quarantine = quarantine_path(path, now_ms());
    fs::rename(path, &quarantine)?;
    eprintln!(
        "finite-agentd: warning: quarantined unreadable inference intent as {}: {problem}",
        quarantine.display()
    );
    Ok(None)
}

/// Writes the record atomically: temp file, fsync, rename, fsync the directory. Mode 0600.
pub(crate) fn store(path: &Path, record: &IntentRecord) -> Result<(), AgentdError> {
    record.check().map_err(|problem| {
        AgentdError::Config(format!("refusing to write an invalid intent: {problem}"))
    })?;
    let parent = path
        .parent()
        .ok_or_else(|| AgentdError::Config("intent path has no parent".to_owned()))?;
    fs::create_dir_all(parent)?;
    let mut bytes = serde_json::to_vec_pretty(record)?;
    bytes.push(b'\n');
    let mut temporary = NamedTempFile::new_in(parent)?;
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o600))?;
    temporary.write_all(&bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

/// Deletes the record. A missing file is not an error.
pub(crate) fn clear(path: &Path) -> Result<(), AgentdError> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// The §3.11 admission table. Every command calls this after its schema check
/// and before any other work.
pub(crate) fn admit(
    record: Option<&IntentRecord>,
    command: AdmitCommand,
) -> Result<Admission, AgentdError> {
    let Some(record) = record else {
        return Ok(Admission::Proceed);
    };
    let running = record.state == IntentState::Running;
    let failed_disconnect = !running && record.kind == IntentKind::Disconnect;
    match command {
        AdmitCommand::Status
        | AdmitCommand::OpenRouterUsage
        | AdmitCommand::CodexModels
        | AdmitCommand::CodexLoginCancel => Ok(Admission::Proceed),
        AdmitCommand::CodexLoginStart => {
            if record.kind == IntentKind::Disconnect && record.route == IntentRoute::OpenaiCodex {
                Err(AgentdError::DisconnectInProgress)
            } else if failed_disconnect {
                Err(AgentdError::OperationInProgress)
            } else {
                Ok(Admission::Proceed)
            }
        }
        AdmitCommand::V1Apply | AdmitCommand::Select | AdmitCommand::Connect => {
            if running || failed_disconnect {
                Err(AgentdError::OperationInProgress)
            } else {
                Ok(Admission::ReplaceFailed)
            }
        }
        AdmitCommand::Disconnect(route) => {
            if running {
                Err(AgentdError::OperationInProgress)
            } else if failed_disconnect {
                if route == record.route {
                    Ok(Admission::ResumeFailed)
                } else {
                    Err(AgentdError::OperationInProgress)
                }
            } else {
                Ok(Admission::ReplaceFailed)
            }
        }
    }
}

fn new_operation_id() -> Result<String, AgentdError> {
    let mut entropy = [0_u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut entropy)?;
    Ok(format!(
        "op_{}",
        entropy
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

fn is_operation_id(value: &str) -> bool {
    value.strip_prefix("op_").is_some_and(|hex| {
        hex.len() == 32
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn quarantine_path(path: &Path, now_ms: u64) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".corrupt-{now_ms}"));
    path.with_file_name(name)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    const KINDS: [IntentKind; 3] = [
        IntentKind::Select,
        IntentKind::Activate,
        IntentKind::Disconnect,
    ];
    const ROUTES: [IntentRoute; 3] = [
        IntentRoute::FinitePrivate,
        IntentRoute::Openrouter,
        IntentRoute::OpenaiCodex,
    ];

    fn record(kind: IntentKind, route: IntentRoute, state: IntentState) -> IntentRecord {
        IntentRecord {
            state,
            ..IntentRecord::new(kind, route, None).unwrap()
        }
    }

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn every_kind_and_phase_round_trips_through_an_atomic_private_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = intent_path(temp.path());
        assert!(load(&path).unwrap().is_none());
        for kind in KINDS {
            for phase in kind.phases() {
                for route in ROUTES {
                    let mut written =
                        IntentRecord::new(kind, route, Some("model/a".to_owned())).unwrap();
                    written.phase = *phase;
                    written.state = IntentState::Failed;
                    written.error_code = Some("verify_failed".to_owned());
                    written.attempts = 3;
                    store(&path, &written).unwrap();
                    assert_eq!(load(&path).unwrap(), Some(written));
                    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
                    assert_eq!(mode, 0o600);
                }
            }
        }
        // No temp file is left beside the record.
        assert_eq!(file_names(path.parent().unwrap()), vec![INTENT_FILE_NAME]);
        clear(&path).unwrap();
        assert!(load(&path).unwrap().is_none());
        clear(&path).unwrap();
    }

    #[test]
    fn the_wire_names_are_the_documented_ones() {
        let mut written =
            IntentRecord::new(IntentKind::Disconnect, IntentRoute::OpenaiCodex, None).unwrap();
        written.phase = IntentPhase::CredentialRemoved;
        let value = serde_json::to_value(&written).unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                "attempts",
                "created_at_ms",
                "error_code",
                "id",
                "kind",
                "model",
                "phase",
                "route",
                "state",
                "updated_at_ms",
                "v",
            ]
        );
        assert_eq!(value["v"], 1);
        assert_eq!(value["kind"], "disconnect");
        assert_eq!(value["route"], "openai_codex");
        assert_eq!(value["phase"], "credential_removed");
        assert_eq!(value["state"], "running");
        assert!(value["model"].is_null());
        assert!(value["error_code"].is_null());
        assert!(is_operation_id(value["id"].as_str().unwrap()));
        let phases = [
            IntentPhase::Accepted,
            IntentPhase::ConfigWritten,
            IntentPhase::Restarting,
            IntentPhase::Verifying,
            IntentPhase::LoginCancelled,
            IntentPhase::RouteSwitched,
            IntentPhase::CredentialRemoved,
            IntentPhase::Cleanup,
        ]
        .map(|phase| serde_json::to_value(phase).unwrap());
        assert_eq!(
            phases,
            [
                "accepted",
                "config_written",
                "restarting",
                "verifying",
                "login_cancelled",
                "route_switched",
                "credential_removed",
                "cleanup",
            ]
            .map(Value::from)
        );
    }

    #[test]
    fn the_schema_has_no_field_that_can_hold_a_secret() {
        // Only `id`, `model`, and `error_code` are free strings. `id` is
        // `op_<32 hex>` minted here; `model` is a model id; `error_code` is a
        // background code. Anything else is rejected as an unknown field.
        let base = serde_json::to_value(
            IntentRecord::new(IntentKind::Select, IntentRoute::Openrouter, None).unwrap(),
        )
        .unwrap();
        for field in [
            "api_key",
            "key",
            "token",
            "access_token",
            "refresh_token",
            "code",
            "code_verifier",
            "secret",
        ] {
            let mut value = base.clone();
            value[field] = json!("sk-or-v1-synthetic");
            assert!(
                serde_json::from_value::<IntentRecord>(value).is_err(),
                "{field} must not be accepted"
            );
        }
    }

    #[test]
    fn a_record_that_does_not_parse_is_quarantined_and_treated_as_absent() {
        let valid = serde_json::to_value(
            IntentRecord::new(IntentKind::Select, IntentRoute::Openrouter, None).unwrap(),
        )
        .unwrap();
        let mut extra_field = valid.clone();
        extra_field["api_key"] = json!("sk-or-v1-synthetic");
        let mut unknown_kind = valid.clone();
        unknown_kind["kind"] = json!("rename");
        let mut unknown_route = valid.clone();
        unknown_route["route"] = json!("anthropic");
        let mut unknown_phase = valid.clone();
        unknown_phase["phase"] = json!("paused");
        let mut unknown_state = valid.clone();
        unknown_state["state"] = json!("succeeded");
        let mut version_two = valid.clone();
        version_two["v"] = json!(2);
        let mut foreign_phase = valid.clone();
        foreign_phase["phase"] = json!("cleanup");
        let mut bad_id = valid.clone();
        bad_id["id"] = json!("op_not-hex");
        let mut missing_field = valid.clone();
        missing_field.as_object_mut().unwrap().remove("attempts");
        let cases = [
            serde_json::to_vec(&extra_field).unwrap(),
            serde_json::to_vec(&unknown_kind).unwrap(),
            serde_json::to_vec(&unknown_route).unwrap(),
            serde_json::to_vec(&unknown_phase).unwrap(),
            serde_json::to_vec(&unknown_state).unwrap(),
            serde_json::to_vec(&version_two).unwrap(),
            serde_json::to_vec(&foreign_phase).unwrap(),
            serde_json::to_vec(&bad_id).unwrap(),
            serde_json::to_vec(&missing_field).unwrap(),
            b"{\"v\": 1, \"id\": ".to_vec(),
            Vec::new(),
        ];
        for bytes in cases {
            let temp = tempfile::tempdir().unwrap();
            let path = intent_path(temp.path());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &bytes).unwrap();
            assert!(load(&path).unwrap().is_none());
            assert!(!path.exists());
            let names = file_names(path.parent().unwrap());
            assert_eq!(names.len(), 1);
            let suffix = names[0]
                .strip_prefix("inference-intent.json.corrupt-")
                .unwrap();
            assert!(suffix.parse::<u64>().is_ok());
            assert_eq!(
                fs::read(path.parent().unwrap().join(&names[0])).unwrap(),
                bytes
            );
            // The next write starts clean.
            let fresh =
                IntentRecord::new(IntentKind::Select, IntentRoute::Openrouter, None).unwrap();
            store(&path, &fresh).unwrap();
            assert_eq!(load(&path).unwrap(), Some(fresh));
        }
    }

    #[test]
    fn store_refuses_a_record_it_could_not_read_back() {
        let temp = tempfile::tempdir().unwrap();
        let path = intent_path(temp.path());
        let mut invalid =
            IntentRecord::new(IntentKind::Select, IntentRoute::Openrouter, None).unwrap();
        invalid.phase = IntentPhase::Cleanup;
        assert!(store(&path, &invalid).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn operation_ids_are_fresh() {
        let first = new_operation_id().unwrap();
        let second = new_operation_id().unwrap();
        assert!(is_operation_id(&first));
        assert_ne!(first, second);
    }

    fn code(result: Result<Admission, AgentdError>) -> Result<Admission, &'static str> {
        result.map_err(|error| error.public_code())
    }

    const NON_MUTATING: [AdmitCommand; 4] = [
        AdmitCommand::Status,
        AdmitCommand::OpenRouterUsage,
        AdmitCommand::CodexModels,
        AdmitCommand::CodexLoginCancel,
    ];
    const MUTATING: [AdmitCommand; 3] = [
        AdmitCommand::V1Apply,
        AdmitCommand::Select,
        AdmitCommand::Connect,
    ];

    fn all_commands() -> Vec<AdmitCommand> {
        let mut commands = NON_MUTATING.to_vec();
        commands.push(AdmitCommand::CodexLoginStart);
        commands.extend(MUTATING);
        commands.extend(ROUTES.map(AdmitCommand::Disconnect));
        commands
    }

    #[test]
    fn admission_with_no_record_always_proceeds() {
        for command in all_commands() {
            assert_eq!(code(admit(None, command)), Ok(Admission::Proceed));
        }
    }

    #[test]
    fn admission_with_a_running_record() {
        for kind in KINDS {
            for route in ROUTES {
                let record = record(kind, route, IntentState::Running);
                for command in NON_MUTATING {
                    assert_eq!(code(admit(Some(&record), command)), Ok(Admission::Proceed));
                }
                let codex_disconnect =
                    kind == IntentKind::Disconnect && route == IntentRoute::OpenaiCodex;
                assert_eq!(
                    code(admit(Some(&record), AdmitCommand::CodexLoginStart)),
                    if codex_disconnect {
                        Err("disconnect_in_progress")
                    } else {
                        Ok(Admission::Proceed)
                    },
                    "{kind:?} {route:?}"
                );
                for command in MUTATING {
                    assert_eq!(
                        code(admit(Some(&record), command)),
                        Err("operation_in_progress")
                    );
                }
                for target in ROUTES {
                    assert_eq!(
                        code(admit(Some(&record), AdmitCommand::Disconnect(target))),
                        Err("operation_in_progress")
                    );
                }
            }
        }
    }

    #[test]
    fn admission_with_a_failed_disconnect() {
        for route in ROUTES {
            let record = record(IntentKind::Disconnect, route, IntentState::Failed);
            for command in NON_MUTATING {
                assert_eq!(code(admit(Some(&record), command)), Ok(Admission::Proceed));
            }
            // disconnect_in_progress wins over operation_in_progress.
            assert_eq!(
                code(admit(Some(&record), AdmitCommand::CodexLoginStart)),
                if route == IntentRoute::OpenaiCodex {
                    Err("disconnect_in_progress")
                } else {
                    Err("operation_in_progress")
                }
            );
            for command in MUTATING {
                assert_eq!(
                    code(admit(Some(&record), command)),
                    Err("operation_in_progress")
                );
            }
            for target in ROUTES {
                assert_eq!(
                    code(admit(Some(&record), AdmitCommand::Disconnect(target))),
                    if target == route {
                        Ok(Admission::ResumeFailed)
                    } else {
                        Err("operation_in_progress")
                    }
                );
            }
        }
    }

    #[test]
    fn admission_with_a_failed_select_or_activate() {
        for kind in [IntentKind::Select, IntentKind::Activate] {
            for route in ROUTES {
                let record = record(kind, route, IntentState::Failed);
                for command in NON_MUTATING {
                    assert_eq!(code(admit(Some(&record), command)), Ok(Admission::Proceed));
                }
                assert_eq!(
                    code(admit(Some(&record), AdmitCommand::CodexLoginStart)),
                    Ok(Admission::Proceed)
                );
                for command in MUTATING {
                    assert_eq!(
                        code(admit(Some(&record), command)),
                        Ok(Admission::ReplaceFailed)
                    );
                }
                for target in ROUTES {
                    assert_eq!(
                        code(admit(Some(&record), AdmitCommand::Disconnect(target))),
                        Ok(Admission::ReplaceFailed)
                    );
                }
            }
        }
    }

    #[test]
    fn admission_errors_carry_the_documented_copy() {
        let codex = record(
            IntentKind::Disconnect,
            IntentRoute::OpenaiCodex,
            IntentState::Failed,
        );
        let error = admit(Some(&codex), AdmitCommand::CodexLoginStart).unwrap_err();
        assert_eq!(
            error.public_message(),
            "ChatGPT is being removed from this agent. Wait for that to finish, or try the removal again."
        );
        let error = admit(Some(&codex), AdmitCommand::V1Apply).unwrap_err();
        assert_eq!(
            error.public_message(),
            "Another connection change is still finishing. Try again in a moment."
        );
    }
}
