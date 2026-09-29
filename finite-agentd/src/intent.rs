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

/// The admission table. Every command calls this after its schema check
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
    use serde_json::Value;

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
    fn every_shared_fixture_is_loaded_or_quarantined_as_it_states() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/intent");
        let mut cases = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        cases.sort();
        assert!(cases.len() >= 32, "intent fixtures are missing");
        for case in cases {
            let name = case.file_name().unwrap().to_string_lossy().into_owned();
            let case = serde_json::from_slice::<Value>(&fs::read(&case).unwrap()).unwrap();
            let bytes = serde_json::to_vec_pretty(&case["record"]).unwrap();
            let temp = tempfile::tempdir().unwrap();
            let path = intent_path(temp.path());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &bytes).unwrap();
            let loaded = load(&path).unwrap();
            if case["valid"].as_bool().unwrap() {
                let loaded = loaded.unwrap_or_else(|| panic!("{name}: a valid record was refused"));
                assert_eq!(
                    serde_json::to_value(&loaded).unwrap()["id"],
                    case["record"]["id"],
                    "{name}"
                );
                assert_eq!(fs::read(&path).unwrap(), bytes, "{name}: the record moved");
            } else {
                assert!(loaded.is_none(), "{name}: {}", case["description"]);
                assert!(
                    !path.exists(),
                    "{name}: the refused record was not moved aside"
                );
                let names = file_names(path.parent().unwrap());
                assert_eq!(names.len(), 1, "{name}");
                assert!(
                    names[0].starts_with("inference-intent.json.corrupt-"),
                    "{name}: {names:?}"
                );
                assert_eq!(
                    fs::read(path.parent().unwrap().join(&names[0])).unwrap(),
                    bytes,
                    "{name}"
                );
            }
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

    #[test]
    fn admission_table() {
        let running = record(
            IntentKind::Select,
            IntentRoute::FinitePrivate,
            IntentState::Running,
        );
        let failed_select = record(
            IntentKind::Select,
            IntentRoute::FinitePrivate,
            IntentState::Failed,
        );
        let failed_disconnect = record(
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            IntentState::Failed,
        );
        let codex_disconnect = record(
            IntentKind::Disconnect,
            IntentRoute::OpenaiCodex,
            IntentState::Failed,
        );

        let mut commands = NON_MUTATING.to_vec();
        commands.push(AdmitCommand::CodexLoginStart);
        commands.extend(MUTATING);
        commands.extend(ROUTES.map(AdmitCommand::Disconnect));
        for command in commands {
            assert_eq!(code(admit(None, command)), Ok(Admission::Proceed));
        }
        for command in NON_MUTATING {
            for current in [&running, &failed_select, &failed_disconnect] {
                assert_eq!(code(admit(Some(current), command)), Ok(Admission::Proceed));
            }
        }
        for command in MUTATING {
            assert_eq!(
                code(admit(Some(&running), command)),
                Err("operation_in_progress")
            );
            assert_eq!(
                code(admit(Some(&failed_select), command)),
                Ok(Admission::ReplaceFailed)
            );
            assert_eq!(
                code(admit(Some(&failed_disconnect), command)),
                Err("operation_in_progress")
            );
        }

        let cases = [
            (None, AdmitCommand::Select, Ok(Admission::Proceed)),
            (
                Some(&running),
                AdmitCommand::Disconnect(IntentRoute::FinitePrivate),
                Err("operation_in_progress"),
            ),
            (
                Some(&failed_select),
                AdmitCommand::Disconnect(IntentRoute::Openrouter),
                Ok(Admission::ReplaceFailed),
            ),
            (
                Some(&failed_disconnect),
                AdmitCommand::Disconnect(IntentRoute::Openrouter),
                Ok(Admission::ResumeFailed),
            ),
            (
                Some(&failed_disconnect),
                AdmitCommand::Disconnect(IntentRoute::FinitePrivate),
                Err("operation_in_progress"),
            ),
            (
                Some(&failed_select),
                AdmitCommand::CodexLoginStart,
                Ok(Admission::Proceed),
            ),
            (
                Some(&running),
                AdmitCommand::CodexLoginStart,
                Ok(Admission::Proceed),
            ),
            (
                Some(&failed_disconnect),
                AdmitCommand::CodexLoginStart,
                Err("operation_in_progress"),
            ),
            (
                Some(&codex_disconnect),
                AdmitCommand::CodexLoginStart,
                Err("disconnect_in_progress"),
            ),
        ];
        for (current, command, expected) in cases {
            assert_eq!(code(admit(current, command)), expected, "{command:?}");
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
