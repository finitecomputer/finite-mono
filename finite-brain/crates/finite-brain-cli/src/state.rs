use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use finite_brain_core::portability::VaultWorkingTreeStateManifest;
use serde::Deserialize;

use crate::{
    AccessExplanation, AgentState, AuthStatus, CliEnvironment, CliError, ConflictState,
    DaemonRunState, DaemonStatus, StatusReport, SyncStatus, identity_paths, load_identity_optional,
    option_value, timestamp, write_json_file,
};

/// Report the shared Finite identity without touching it: status never mints
/// (finite-identity CLI-CONVENTIONS.md).
pub(crate) fn auth_status(env: &CliEnvironment) -> Result<AuthStatus, CliError> {
    let identity_file = identity_paths(env)?.identity_file().display().to_string();
    Ok(match load_identity_optional(env)? {
        Some(identity) => AuthStatus {
            state: "authenticated".to_owned(),
            npub: Some(identity.npub()),
            identity_file,
            created_by: Some(identity.created_by().to_owned()),
            created_at: Some(identity.created_at().to_owned()),
            signer: "finite-identity".to_owned(),
            config_dir: env.config_dir.display().to_string(),
        },
        None => AuthStatus {
            state: "missing".to_owned(),
            npub: None,
            identity_file,
            created_by: None,
            created_at: None,
            signer: "none".to_owned(),
            config_dir: env.config_dir.display().to_string(),
        },
    })
}

pub(crate) fn daemon_status(env: &CliEnvironment) -> Result<DaemonStatus, CliError> {
    let state = load_current_agent_state(env)?;
    Ok(daemon_status_from_state(&state))
}

pub(crate) fn status_report(env: &CliEnvironment) -> Result<StatusReport, CliError> {
    let auth = auth_status(env)?;
    let root = find_agent_state(&env.cwd)?;
    let Some(root) = root else {
        return Ok(StatusReport {
            vault_id: None,
            working_tree_path: None,
            auth,
            daemon: DaemonStatus {
                state: "missing".to_owned(),
                sync_mode: "automatic".to_owned(),
                last_started_at: None,
                last_tick_at: None,
                last_error: None,
                tick_count: 0,
                failure_count: 0,
                retry_backoff_millis: 0,
                watch_strategy: None,
                last_local_change_count: None,
            },
            sync: SyncStatus {
                mode: "automatic".to_owned(),
                status: "no-working-tree".to_owned(),
                latest_sequence: 0,
            },
            unlocked_folders: Vec::new(),
            conflicts: Vec::new(),
            blocked: vec!["no Vault Working Tree found".to_owned()],
        });
    };
    let state = read_agent_state(&root)?;
    let tree_state = read_working_tree_state(&root)?;
    let open_conflicts = state
        .conflicts
        .iter()
        .filter(|conflict| conflict.state == ConflictState::Open)
        .cloned()
        .collect::<Vec<_>>();
    let mut blocked = Vec::new();
    if auth.state != "authenticated" {
        blocked.push(
            "no Finite identity yet (minted on first signing use, or fbrain auth import)"
                .to_owned(),
        );
    }
    if state.daemon.state != DaemonRunState::Running {
        blocked.push("daemon not running".to_owned());
    }
    if !open_conflicts.is_empty() {
        blocked.push("unresolved conflicts".to_owned());
    }
    Ok(StatusReport {
        vault_id: Some(state.vault_id.clone()),
        working_tree_path: Some(root.display().to_string()),
        auth,
        daemon: daemon_status_from_state(&state),
        sync: SyncStatus {
            mode: state.sync.mode,
            status: state.sync.status,
            latest_sequence: tree_state.sync.latest_sequence,
        },
        unlocked_folders: state.unlocked_folders,
        conflicts: open_conflicts,
        blocked,
    })
}

fn daemon_status_from_state(state: &AgentState) -> DaemonStatus {
    DaemonStatus {
        state: state.daemon.state.to_string(),
        sync_mode: state.sync.mode.clone(),
        last_started_at: state.daemon.last_started_at.clone(),
        last_tick_at: state.daemon.last_tick_at.clone(),
        last_error: state.daemon.last_error.clone(),
        tick_count: state.daemon.tick_count,
        failure_count: state.daemon.failure_count,
        retry_backoff_millis: state.daemon.retry_backoff_millis,
        watch_strategy: state.daemon.watch_strategy.clone(),
        last_local_change_count: state.daemon.last_local_change_count,
    }
}

pub(crate) fn explain_access(
    folder: &str,
    tree: &VaultWorkingTreeStateManifest,
) -> AccessExplanation {
    if let Some(root) = tree
        .folder_roots
        .iter()
        .find(|root| root.folder_id == folder || root.path == folder)
    {
        if root.can_read {
            AccessExplanation {
                folder: folder.to_owned(),
                state: "readable".to_owned(),
                reason: "Folder is materialized and readable in this Vault Working Tree".to_owned(),
            }
        } else if root.metadata_only {
            AccessExplanation {
                folder: folder.to_owned(),
                state: "locked".to_owned(),
                reason: "Folder is metadata-only; Folder Access or an open Folder Key is missing"
                    .to_owned(),
            }
        } else {
            AccessExplanation {
                folder: folder.to_owned(),
                state: "unavailable".to_owned(),
                reason: "Folder is present but not readable".to_owned(),
            }
        }
    } else {
        AccessExplanation {
            folder: folder.to_owned(),
            state: "unknown".to_owned(),
            reason: "Folder is not listed in working-tree state".to_owned(),
        }
    }
}

pub(crate) fn current_tree_root(env: &CliEnvironment) -> Result<PathBuf, CliError> {
    find_agent_state(&env.cwd)?.ok_or(CliError::MissingWorkingTree)
}

pub(crate) fn load_current_agent_state(env: &CliEnvironment) -> Result<AgentState, CliError> {
    let root = current_tree_root(env)?;
    read_agent_state(&root)
}

pub(crate) fn mutate_agent_state<F>(env: &CliEnvironment, f: F) -> Result<(), CliError>
where
    F: FnOnce(&mut AgentState, String) -> Result<(), CliError>,
{
    let root = current_tree_root(env)?;
    let mut state = read_agent_state(&root)?;
    f(&mut state, timestamp(env))?;
    write_agent_state(&root, &state)
}

pub(crate) fn find_agent_state(start: &Path) -> Result<Option<PathBuf>, CliError> {
    let mut cursor = start.to_path_buf();
    loop {
        if cursor.join(".finitebrain/agent-state.json").exists() {
            return Ok(Some(cursor));
        }
        if !cursor.pop() {
            return Ok(None);
        }
    }
}

pub(crate) fn read_agent_state(root: &Path) -> Result<AgentState, CliError> {
    read_json_file(&root.join(".finitebrain/agent-state.json"))
}

pub(crate) fn write_agent_state(root: &Path, state: &AgentState) -> Result<(), CliError> {
    write_json_file(&root.join(".finitebrain/agent-state.json"), state)
}

pub(crate) fn read_working_tree_state(
    root: &Path,
) -> Result<VaultWorkingTreeStateManifest, CliError> {
    read_json_file(&root.join(".finitebrain/working-tree-state.json"))
}

pub(crate) fn read_json_file<T>(path: &Path) -> Result<T, CliError>
where
    T: for<'de> Deserialize<'de>,
{
    let mut body = String::new();
    fs::File::open(path)?.read_to_string(&mut body)?;
    serde_json::from_str(&body).map_err(CliError::from)
}

pub(crate) fn command_vault_id(args: &[String], env: &CliEnvironment) -> Result<String, CliError> {
    option_value(args, "--vault")
        .or_else(|| current_vault_id(env))
        .ok_or(CliError::MissingArgument("vault-id or --vault"))
}

pub(crate) fn current_vault_id(env: &CliEnvironment) -> Option<String> {
    find_agent_state(&env.cwd)
        .ok()
        .flatten()
        .and_then(|root| read_agent_state(&root).ok())
        .map(|state| state.vault_id)
}
