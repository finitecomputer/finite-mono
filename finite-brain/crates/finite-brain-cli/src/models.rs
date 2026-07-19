use std::collections::BTreeMap;
use std::fmt;

use finite_brain_core::FolderKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AGENT_STATE_VERSION;

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentState {
    pub(crate) version: String,
    pub(crate) vault_id: String,
    pub(crate) server_url: Option<String>,
    pub(crate) auth_npub: Option<String>,
    pub(crate) daemon: DaemonState,
    pub(crate) sync: AgentSyncState,
    pub(crate) conflicts: Vec<ConflictEntry>,
    pub(crate) activity: Vec<ActivityEntry>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

impl AgentState {
    pub(crate) fn new(vault_id: &str, now: &str) -> Self {
        Self {
            version: AGENT_STATE_VERSION.to_owned(),
            vault_id: vault_id.to_owned(),
            server_url: None,
            auth_npub: None,
            daemon: DaemonState {
                state: DaemonRunState::Stopped,
                last_started_at: None,
                last_tick_at: None,
                last_error: None,
                tick_count: 0,
                failure_count: 0,
                retry_backoff_millis: 0,
                watch_strategy: None,
                last_local_change_count: None,
            },
            sync: AgentSyncState {
                mode: "automatic".to_owned(),
                status: "idle".to_owned(),
            },
            conflicts: Vec::new(),
            activity: Vec::new(),
            created_at: now.to_owned(),
            updated_at: now.to_owned(),
        }
    }

    pub(crate) fn add_activity(
        &mut self,
        at: String,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) {
        let kind = kind.into();
        let id = activity_id(&at, self.activity.len() + 1, &kind);
        self.activity.push(ActivityEntry {
            id,
            at: at.clone(),
            kind,
            message: message.into(),
        });
        self.updated_at = at;
    }
}

fn activity_id(at: &str, index: usize, kind: &str) -> String {
    let digest = Sha256::digest(format!("{at}\n{index}\n{kind}").as_bytes());
    format!(
        "activity-{}",
        digest
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DaemonState {
    pub(crate) state: DaemonRunState,
    pub(crate) last_started_at: Option<String>,
    #[serde(default)]
    pub(crate) last_tick_at: Option<String>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
    #[serde(default)]
    pub(crate) tick_count: u64,
    #[serde(default)]
    pub(crate) failure_count: u64,
    #[serde(default)]
    pub(crate) retry_backoff_millis: u64,
    #[serde(default)]
    pub(crate) watch_strategy: Option<String>,
    #[serde(default)]
    pub(crate) last_local_change_count: Option<usize>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DaemonRunState {
    Running,
    Stopped,
    Missing,
}

impl fmt::Display for DaemonRunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => f.write_str("running"),
            Self::Stopped => f.write_str("stopped"),
            Self::Missing => f.write_str("missing"),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSyncState {
    pub(crate) mode: String,
    pub(crate) status: String,
}

#[derive(Default)]
pub(crate) struct SessionFolderKeyring {
    keys: BTreeMap<(String, String, u32), FolderKey>,
}

impl SessionFolderKeyring {
    pub(crate) fn insert(
        &mut self,
        vault_id: impl Into<String>,
        folder_id: impl Into<String>,
        key_version: u32,
        folder_key: FolderKey,
    ) -> bool {
        self.keys
            .insert((vault_id.into(), folder_id.into(), key_version), folder_key)
            .is_none()
    }

    pub(crate) fn get(
        &self,
        vault_id: &str,
        folder_id: &str,
        key_version: u32,
    ) -> Option<&FolderKey> {
        self.keys
            .get(&(vault_id.to_owned(), folder_id.to_owned(), key_version))
    }

    pub(crate) fn contains(&self, vault_id: &str, folder_id: &str, key_version: u32) -> bool {
        self.get(vault_id, folder_id, key_version).is_some()
    }

    pub(crate) fn len(&self) -> usize {
        self.keys.len()
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictEntry {
    pub id: String,
    pub folder_id: Option<String>,
    pub path: Option<String>,
    pub reason: String,
    pub state: ConflictState,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictState {
    Open,
    Resolved,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub id: String,
    pub at: String,
    pub kind: String,
    pub message: String,
}

/// `fbrain auth status`: the shared Finite identity (never minted here),
/// plus fbrain-specific context (signer type and config dir).
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthStatus {
    pub(crate) state: String,
    pub(crate) npub: Option<String>,
    pub(crate) identity_file: String,
    pub(crate) created_by: Option<String>,
    pub(crate) created_at: Option<String>,
    pub(crate) signer: String,
    pub(crate) config_dir: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DaemonStatus {
    pub(crate) state: String,
    pub(crate) sync_mode: String,
    pub(crate) last_started_at: Option<String>,
    pub(crate) last_tick_at: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) tick_count: u64,
    pub(crate) failure_count: u64,
    pub(crate) retry_backoff_millis: u64,
    pub(crate) watch_strategy: Option<String>,
    pub(crate) last_local_change_count: Option<usize>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncStatus {
    pub(crate) mode: String,
    pub(crate) status: String,
    pub(crate) latest_sequence: u64,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncOnceReport {
    pub(crate) status: String,
    pub(crate) latest_sequence: u64,
    pub(crate) record_count: usize,
    pub(crate) server_url: String,
    pub(crate) local_changes: Vec<SyncChangeReport>,
    pub(crate) remote_changes: Vec<SyncChangeReport>,
    pub(crate) conflicts: Vec<SyncChangeReport>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncChangeReport {
    pub(crate) status: String,
    pub(crate) action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) actor_npub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sequence: Option<u64>,
    pub(crate) path: Option<String>,
    pub(crate) from_path: Option<String>,
    pub(crate) folder_id: Option<String>,
    pub(crate) source_vault_id: Option<String>,
    pub(crate) object_id: Option<String>,
    pub(crate) route: String,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatusReport {
    pub(crate) vault_id: Option<String>,
    pub(crate) working_tree_path: Option<String>,
    pub(crate) auth: AuthStatus,
    pub(crate) daemon: DaemonStatus,
    pub(crate) sync: SyncStatus,
    pub(crate) conflicts: Vec<ConflictEntry>,
    pub(crate) blocked: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckState {
    pub(crate) state: String,
    pub(crate) message: String,
}

impl CheckState {
    pub(crate) fn ok(message: impl Into<String>) -> Self {
        Self {
            state: "ok".to_owned(),
            message: message.into(),
        }
    }

    pub(crate) fn warn(message: impl Into<String>) -> Self {
        Self {
            state: "warn".to_owned(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HealthCheck {
    pub(crate) state: String,
    pub(crate) message: String,
}

impl HealthCheck {
    pub(crate) fn ok(message: impl Into<String>) -> Self {
        Self {
            state: "ok".to_owned(),
            message: message.into(),
        }
    }

    pub(crate) fn warn(message: impl Into<String>) -> Self {
        Self {
            state: "warn".to_owned(),
            message: message.into(),
        }
    }

    pub(crate) fn skipped(message: impl Into<String>) -> Self {
        Self {
            state: "skipped".to_owned(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DoctorReport {
    pub(crate) cli: CheckState,
    pub(crate) auth: CheckState,
    pub(crate) working_tree: CheckState,
    pub(crate) daemon: CheckState,
    pub(crate) server: HealthCheck,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccessExplanation {
    pub(crate) folder: String,
    pub(crate) state: String,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccessRemovalBlockedReport {
    pub(crate) state: String,
    pub(crate) operation: String,
    pub(crate) vault_id: String,
    pub(crate) folder_id: String,
    pub(crate) target_npub: String,
    pub(crate) route: String,
    pub(crate) reason: String,
    pub(crate) required: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccessSummaryReport {
    pub(crate) vault_id: String,
    pub(crate) members: Vec<String>,
    pub(crate) admins: Vec<String>,
    pub(crate) folders: Vec<FolderAccessSummary>,
    pub(crate) mounted_folders: Vec<MountedFolderMetadataView>,
    pub(crate) grant_count: usize,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FolderAccessSummary {
    #[serde(flatten)]
    pub(crate) metadata: FolderMetadataView,
    pub(crate) explicit_access_user_ids: Vec<String>,
    pub(crate) effective_access_user_ids: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) body: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VaultMetadataView {
    pub(crate) vault_id: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) owner_user_id: Option<String>,
    #[serde(default)]
    pub(crate) personal_agent: Option<PersonalAgentView>,
    pub(crate) members: Vec<String>,
    pub(crate) admins: Vec<String>,
    pub(crate) folders: Vec<FolderMetadataView>,
    #[serde(default)]
    pub(crate) mounted_folders: Vec<MountedFolderMetadataView>,
    #[serde(default)]
    pub(crate) grant_count: usize,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersonalAgentView {
    pub(crate) agent_npub: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FolderMetadataView {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default = "default_folder_role")]
    pub(crate) role: String,
    pub(crate) access: String,
    #[serde(default)]
    pub(crate) parent_folder_id: Option<String>,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) shared_folder_source: bool,
    pub(crate) access_user_ids: Vec<String>,
    pub(crate) current_key_version: u32,
    #[serde(default)]
    pub(crate) setup_incomplete: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MountedFolderMetadataView {
    pub(crate) mount_id: String,
    pub(crate) organization_vault_id: String,
    pub(crate) source_vault_id: String,
    pub(crate) source_folder_id: String,
    pub(crate) connection_id: String,
    pub(crate) display_name: String,
    pub(crate) display_parent_folder_id: Option<String>,
    pub(crate) state: String,
}

fn default_folder_role() -> String {
    "folder".to_owned()
}
