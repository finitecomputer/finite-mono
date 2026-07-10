//! Agent-native FiniteBrain CLI surface.

mod admin;
mod args;
mod clock;
mod environment;
mod error;
mod http;
mod identity_authority;
mod models;
mod output;
mod signer;
mod state;
mod sync_engine;

pub use environment::CliEnvironment;
pub use error::CliError;
pub use models::{ActivityEntry, ConflictEntry, ConflictState, UnlockedFolder};

pub(crate) use admin::*;
pub(crate) use args::*;
pub(crate) use clock::*;
pub(crate) use http::*;
pub(crate) use identity_authority::*;
pub(crate) use models::*;
pub(crate) use output::*;
pub(crate) use signer::*;
pub(crate) use state::*;
pub(crate) use sync_engine::*;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use finite_brain_core::portability::{
    VaultDirectoryManifest, VaultDirectoryPath, VaultDirectoryPortability,
    VaultDirectoryVaultSummary, VaultWorkingTreeStateManifest, WorkingTreeFolderRoot,
    WorkingTreeObjectManifestEntry, WorkingTreeSyncState,
};
use finite_brain_core::{
    AdminAccessAction, FolderId, FolderKey, FolderObjectAad, FolderObjectOperation, ObjectId,
    SafeRelativePath, VaultId, VaultKind, bootstrap_organization_vault, bootstrap_personal_vault,
    default_vault_pages, encrypt_folder_object,
};
use finite_nostr::{NostrPublicKey, build_rumor, decrypt_nip44, encrypt_nip44, wrap_rumor};
use nostr::{Keys, Kind};
use sha2::{Digest, Sha256};

pub(crate) const AGENT_STATE_VERSION: &str = "finitebrain-agent-state-v1";
pub(crate) const VAULT_DIRECTORY_VERSION: &str = "finite-vault-directory-v1";
pub(crate) const WORKING_TREE_STATE_VERSION: &str = "finite-vault-working-tree-state-v1";
pub(crate) const APP_SPECIFIC_KIND: u16 = 30_078;
const CIPHER_AES_256_GCM: &str = "AES-256-GCM";

/// Run `fbrain` using process args, stdin, and stdout.
pub fn run_from_process(env: CliEnvironment) -> Result<(), CliError> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut stdout = std::io::stdout();
    run_with_io(args, env, &mut std::io::stdin(), &mut stdout)
}

/// Run `fbrain` with injected args and output. Tests use this public seam.
/// Commands that read a secret from stdin (`auth import`) read from the
/// process stdin; use [`run_with_io`] to inject it.
pub fn run_with_env<I, S, W>(args: I, env: CliEnvironment, output: &mut W) -> Result<(), CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    W: Write,
{
    run_with_io(args, env, &mut std::io::stdin(), output)
}

/// Run `fbrain` with injected args, input, and output. `input` stands in
/// for stdin and is only read by `fbrain auth import` without `--file`.
pub fn run_with_io<I, S, W>(
    args: I,
    env: CliEnvironment,
    input: &mut dyn std::io::Read,
    output: &mut W,
) -> Result<(), CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    W: Write,
{
    let mut args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let mut env = env;
    if let Some(config_dir) = take_option_value(&mut args, "--config-dir")? {
        env.config_dir = expand_cli_path(&config_dir);
    }
    let json = take_flag(&mut args, "--json");
    let command = args.first().cloned().unwrap_or_else(|| "help".to_owned());
    match command.as_str() {
        "help" | "--help" | "-h" => help(output),
        "version" | "--version" | "-V" => version(output),
        "doctor" => doctor(&args[1..], &env, json, output),
        "auth" => auth(&args[1..], &env, json, input, output),
        "signer" => signer(&args[1..], &env, json, output),
        "daemon" => daemon(&args[1..], &env, json, output),
        "sync" => sync(&args[1..], &env, json, output),
        "open" => open_vault(&args[1..], &env, json, output),
        "status" => status(&env, json, output),
        "unlock" => unlock(&args[1..], &env, json, output),
        "conflicts" => conflicts(&env, json, output),
        "resolve" => resolve(&args[1..], &env, json, output),
        "activity" => activity(&env, json, output),
        "access" => access(&args[1..], &env, json, output),
        "vault" => vault(&args[1..], &env, json, output),
        "folder" => folder(&args[1..], &env, json, output),
        "mount" | "mounts" => mount(&args[1..], &env, json, output),
        "permissions" | "permission" | "perms" => permissions(&args[1..], &env, json, output),
        "invites" | "invite" => invites(&args[1..], &env, json, output),
        "share" | "shared" => share(&args[1..], &env, json, output),
        other => Err(CliError::InvalidCommand(other.to_owned())),
    }
}

fn help<W: Write>(output: &mut W) -> Result<(), CliError> {
    writeln!(
        output,
        "fbrain [--config-dir <path>] doctor\nauth status|import [--file <path>]|login <email>|redeem <email> <token>\nsigner status|public-key|sign|encrypt|decrypt\ndaemon status|start|stop|logs|tick|watch\nsync status|now [--summary]\nopen <vault-id> [path]\nstatus [--json]\nunlock [folder|--all]\nconflicts\nresolve <id>\nactivity\naccess explain|list|grant|revoke\nvault create|metadata|export\nfolder create|list\nmount list\npermissions add-member|remove-member|add-admin|remove-admin|grant-folder --target <NIP-05|npub|hex>\ninvites create --target <NIP-05|npub|hex>|show --code invite-...|accept --code invite-...|accept --vault <vault-id> --id invitation-...|revoke\nshare link --target <NIP-05|npub|hex>|accept|revoke|source|folder-invite --destination-admin <NIP-05|npub|hex>|folder-accept"
    )?;
    Ok(())
}

fn version<W: Write>(output: &mut W) -> Result<(), CliError> {
    writeln!(output, "fbrain {}", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

fn expand_cli_path(value: &str) -> PathBuf {
    value
        .strip_prefix("~/")
        .and_then(|suffix| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(suffix)))
        .unwrap_or_else(|| PathBuf::from(value))
}

fn doctor<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let server_url = server_url_for_optional_command(env, args);
    let working_tree = find_agent_state(&env.cwd).ok().flatten();
    let identity = load_identity_optional(env)?;
    let daemon_state = working_tree
        .as_ref()
        .and_then(|root| read_agent_state(root).ok())
        .map(|state| state.daemon.state)
        .unwrap_or(DaemonRunState::Missing);
    let server = server_url
        .as_deref()
        .map(check_http_health)
        .unwrap_or_else(|| HealthCheck::skipped("no server URL configured"));
    let report = DoctorReport {
        cli: CheckState::ok("fbrain CLI is available"),
        auth: identity
            .as_ref()
            .map(|identity| {
                CheckState::ok(format!("acting npub {} (shared Finite identity)", identity.npub()))
            })
            .unwrap_or_else(|| {
                CheckState::warn(
                    "no Finite identity yet; it is minted on first signing use, or adopt an existing secret with fbrain auth import",
                )
            }),
        working_tree: working_tree
            .as_ref()
            .map(|root| CheckState::ok(format!("Vault Working Tree at {}", root.display())))
            .unwrap_or_else(|| CheckState::warn("not inside a Vault Working Tree")),
        daemon: match daemon_state {
            DaemonRunState::Running => CheckState::ok("daemon marked running"),
            DaemonRunState::Stopped => CheckState::warn("daemon marked stopped"),
            DaemonRunState::Missing => CheckState::warn("daemon state missing"),
        },
        server,
    };
    if json {
        write_json(output, &report)
    } else {
        writeln!(output, "fbrain doctor")?;
        writeln!(output, "- cli: {}", report.cli.message)?;
        writeln!(output, "- auth: {}", report.auth.message)?;
        writeln!(output, "- working tree: {}", report.working_tree.message)?;
        writeln!(output, "- daemon: {}", report.daemon.message)?;
        writeln!(output, "- server: {}", report.server.message)?;
        Ok(())
    }
}

fn auth<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    input: &mut dyn std::io::Read,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str).unwrap_or("status") {
        // Shows the shared Finite identity; never mints (CLI-CONVENTIONS.md).
        "status" => {
            let status = auth_status(env)?;
            if json {
                write_json(output, &status)
            } else {
                match status.state.as_str() {
                    "authenticated" => {
                        writeln!(
                            output,
                            "authenticated as {} ({})",
                            status.npub.as_deref().unwrap_or("-"),
                            status.signer
                        )?;
                        writeln!(output, "identity file: {}", status.identity_file)?;
                        writeln!(
                            output,
                            "created by {} at {}",
                            status.created_by.as_deref().unwrap_or("-"),
                            status.created_at.as_deref().unwrap_or("-")
                        )?;
                    }
                    _ => {
                        writeln!(output, "no Finite identity yet")?;
                        writeln!(
                            output,
                            "identity file: {} (not created)",
                            status.identity_file
                        )?;
                        writeln!(
                            output,
                            "mint: run any fbrain command that signs, or bring your own: fbrain auth import"
                        )?;
                    }
                }
                Ok(())
            }
        }
        // Adopts an existing secret as the shared Finite identity. The
        // secret comes from stdin or --file, never argv: flag values leak
        // into ps output and shell history (CLI-CONVENTIONS.md). Refuses to
        // overwrite an existing identity.
        "import" => {
            let secret_text = match option_value(args, "--file") {
                Some(path) => {
                    let path = expand_cli_path(&path);
                    fs::read_to_string(&path)?
                }
                None => read_secret_text(input)?,
            };
            let identity = import_identity(env, &secret_text)?;
            let identity_file = identity_paths(env)?.identity_file();
            if json {
                write_json(
                    output,
                    &serde_json::json!({
                        "npub": identity.npub(),
                        "identityFile": identity_file.display().to_string()
                    }),
                )
            } else {
                writeln!(output, "imported Finite identity {}", identity.npub())?;
                writeln!(output, "identity file: {}", identity_file.display())?;
                Ok(())
            }
        }
        // Hard cut: the plaintext auth.json prototype is gone. Keep explicit
        // guidance for the removed secret-login shape instead of accepting
        // secrets in argv again.
        "login"
            if args[1..]
                .iter()
                .any(|arg| arg == "--nsec" || arg.starts_with("--nsec=")) =>
        {
            Err(CliError::Unsupported(
            "fbrain auth login was replaced by fbrain auth import; pipe the secret via stdin or --file <path> (never argv)".to_owned(),
            ))
        }
        "login" => auth_email_login(&args[1..], env, json, output),
        "redeem" => auth_email_redeem(&args[1..], env, json, output),
        "logout" => Err(CliError::Unsupported(
            "fbrain auth logout was removed: the identity is shared by every Finite tool; move ~/.finite/identity/identity.json aside by hand if you mean it".to_owned(),
        )),
        other => Err(CliError::InvalidCommand(format!("auth {other}"))),
    }
}

fn auth_email_login<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let positionals = positional_values(args);
    let [email] = positionals.as_slice() else {
        return Err(CliError::MissingArgument("email"));
    };
    let identity_authority = IdentityAuthorityClient::from_environment(env)?;
    let report = identity_authority.request_email_challenge(email)?;
    if json {
        write_json(output, &report)
    } else {
        writeln!(output, "sent email challenge for {}", report.email)?;
        writeln!(
            output,
            "run fbrain auth redeem {} TOKEN_FROM_EMAIL",
            report.email
        )?;
        Ok(())
    }
}

fn auth_email_redeem<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let positionals = positional_values(args);
    let [email, token] = positionals.as_slice() else {
        return Err(CliError::MissingArgument("email token"));
    };
    let identity_authority = IdentityAuthorityClient::from_environment(env)?;
    let report = identity_authority.redeem_email(env, email, token)?;
    if json {
        write_json(output, &report)
    } else {
        writeln!(
            output,
            "verified {} as {}",
            report.email, report.principal_kind
        )?;
        writeln!(output, "pubkey: {}", report.pubkey)?;
        if let Some(nip05) = &report.nip05 {
            writeln!(output, "nip05: {nip05}")?;
        }
        if let Some(limitation) = &report.limitation {
            writeln!(output, "note: {limitation}")?;
        }
        Ok(())
    }
}

/// Read the secret for `fbrain auth import` from stdin: one trimmed line
/// (`ImportSecret::parse` owns validation and never echoes the input).
fn read_secret_text(input: &mut dyn std::io::Read) -> Result<String, CliError> {
    let mut text = String::new();
    std::io::BufReader::new(input).read_line(&mut text)?;
    if text.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "expected an nsec1... or 64-char hex secret on stdin (or use --file <path>)".to_owned(),
        ));
    }
    Ok(text)
}

fn signer<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str).unwrap_or("status") {
        "status" => auth(
            &["status".to_owned()],
            env,
            json,
            &mut std::io::empty(),
            output,
        ),
        "public-key" | "get-public-key" => {
            let auth = load_signer(env)?;
            if json {
                write_json(output, &serde_json::json!({ "npub": auth.npub }))
            } else {
                writeln!(output, "{}", auth.npub)?;
                Ok(())
            }
        }
        "sign" | "sign-event" => {
            let keys = signer_keys(env)?;
            let kind = option_value(args, "--kind")
                .as_deref()
                .map(parse_kind)
                .transpose()?
                .unwrap_or(Kind::TextNote);
            let content = option_value(args, "--content")
                .or_else(|| positional_values(args).get(1).cloned())
                .unwrap_or_default();
            let tags = option_values(args, "--tag")
                .into_iter()
                .map(parse_cli_tag)
                .collect::<Result<Vec<_>, _>>()?;
            let event = sign_event(&keys, kind, content, tags, unix_timestamp(), None)?;
            if json {
                write_json(
                    output,
                    &serde_json::json!({
                        "event": event,
                        "eventJson": event.as_json()
                    }),
                )
            } else {
                writeln!(output, "{}", event.as_json())?;
                Ok(())
            }
        }
        "encrypt" => {
            let keys = signer_keys(env)?;
            let recipient = option_value(args, "--to").ok_or(CliError::MissingArgument("--to"))?;
            let plaintext = option_value(args, "--text")
                .or_else(|| positional_values(args).get(1).cloned())
                .ok_or(CliError::MissingArgument("--text"))?;
            let recipient = NostrPublicKey::parse(&recipient)
                .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
            let ciphertext = encrypt_nip44(keys.secret_key(), recipient, plaintext)
                .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
            if json {
                write_json(output, &serde_json::json!({ "ciphertext": ciphertext }))
            } else {
                writeln!(output, "{ciphertext}")?;
                Ok(())
            }
        }
        "decrypt" => {
            let keys = signer_keys(env)?;
            let sender = option_value(args, "--from").ok_or(CliError::MissingArgument("--from"))?;
            let payload = option_value(args, "--payload")
                .or_else(|| positional_values(args).get(1).cloned())
                .ok_or(CliError::MissingArgument("--payload"))?;
            let sender = NostrPublicKey::parse(&sender)
                .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
            let plaintext = decrypt_nip44(keys.secret_key(), sender, payload)
                .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
            if json {
                write_json(output, &serde_json::json!({ "plaintext": plaintext }))
            } else {
                writeln!(output, "{plaintext}")?;
                Ok(())
            }
        }
        other => Err(CliError::InvalidCommand(format!("signer {other}"))),
    }
}

fn daemon<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str).unwrap_or("status") {
        "status" => {
            let report = daemon_status(env)?;
            if json {
                write_json(output, &report)
            } else {
                writeln!(output, "daemon {}", report.state)?;
                Ok(())
            }
        }
        "start" => {
            let sync_result = sync_once(env, args, "daemon.start");
            mutate_agent_state(env, |state, now| {
                state.daemon.state = DaemonRunState::Running;
                state.daemon.last_started_at = Some(now.clone());
                state.daemon.last_tick_at = Some(now.clone());
                state.daemon.tick_count = 1;
                state.daemon.watch_strategy = Some("manual-start".to_owned());
                state.daemon.last_local_change_count = None;
                match &sync_result {
                    Ok(report) => {
                        state.daemon.last_error = None;
                        state.daemon.retry_backoff_millis = 0;
                        state.sync.status = report.status.clone();
                    }
                    Err(error) => {
                        state.daemon.failure_count = state.daemon.failure_count.saturating_add(1);
                        state.daemon.last_error = Some(error.to_string());
                        state.daemon.retry_backoff_millis = 0;
                        state.sync.status = format!("blocked: {error}");
                    }
                }
                state.add_activity(now, "daemon.started", "Agent Sync Daemon marked running");
                Ok(())
            })?;
            daemon(&["status".to_owned()], env, json, output)
        }
        "stop" => {
            mutate_agent_state(env, |state, now| {
                state.daemon.state = DaemonRunState::Stopped;
                state.daemon.retry_backoff_millis = 0;
                state.daemon.watch_strategy = Some("stopped".to_owned());
                state.sync.status = "paused".to_owned();
                state.add_activity(now, "daemon.stopped", "Agent Sync Daemon marked stopped");
                Ok(())
            })?;
            daemon(&["status".to_owned()], env, json, output)
        }
        "logs" => {
            let state = load_current_agent_state(env)?;
            let rows = state
                .activity
                .into_iter()
                .filter(|entry| entry.kind.starts_with("daemon."))
                .collect::<Vec<_>>();
            if json {
                write_json(output, &rows)
            } else {
                write_activity_rows(output, &rows)
            }
        }
        "tick" => {
            let report = sync_once(env, args, "daemon.tick");
            mutate_agent_state(env, |state, now| {
                state.daemon.state = DaemonRunState::Running;
                state.daemon.last_tick_at = Some(now.clone());
                state.daemon.tick_count = state.daemon.tick_count.saturating_add(1);
                state.daemon.watch_strategy = Some("manual-tick".to_owned());
                state.daemon.last_local_change_count = None;
                match &report {
                    Ok(report) => {
                        state.daemon.last_error = None;
                        state.daemon.retry_backoff_millis = 0;
                        state.sync.status = report.status.clone();
                    }
                    Err(error) => {
                        state.daemon.failure_count = state.daemon.failure_count.saturating_add(1);
                        state.daemon.last_error = Some(error.to_string());
                        state.daemon.retry_backoff_millis = 0;
                        state.sync.status = format!("blocked: {error}");
                    }
                }
                Ok(())
            })?;
            let report = report?;
            if json {
                write_json(output, &report)
            } else {
                writeln!(
                    output,
                    "{} latestSequence={}",
                    report.status, report.latest_sequence
                )?;
                Ok(())
            }
        }
        "watch" => daemon_watch(args, env, json, output),
        other => Err(CliError::InvalidCommand(format!("daemon {other}"))),
    }
}

fn daemon_watch<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let max_ticks = daemon_watch_max_ticks(args)?;
    let poll = daemon_watch_poll(args)?;
    let file_aware = !args.iter().any(|arg| arg == "--poll-only");
    let watch_strategy = if file_aware {
        "working-tree-files"
    } else {
        "poll"
    };
    let remote_poll_ticks = daemon_watch_remote_poll_ticks(args)?;
    let root = current_tree_root(env)?;
    mutate_agent_state(env, |state, now| {
        state.daemon.state = DaemonRunState::Running;
        state.daemon.last_started_at = Some(now.clone());
        state.daemon.last_tick_at = None;
        state.daemon.last_error = None;
        state.daemon.tick_count = 0;
        state.daemon.failure_count = 0;
        state.daemon.retry_backoff_millis = 0;
        state.daemon.watch_strategy = Some(watch_strategy.to_owned());
        state.daemon.last_local_change_count = None;
        state.sync.status = "watching".to_owned();
        state.add_activity(
            now,
            "daemon.watch.started",
            format!("Agent Sync Daemon watch loop started strategy={watch_strategy}"),
        );
        Ok(())
    })?;

    let mut ticks = 0_usize;
    let mut failures = 0_usize;
    let mut skipped_ticks = 0_usize;
    let mut consecutive_failures = 0_usize;
    let mut retry_backoff_millis: u64;
    let mut last_status = None::<String>;
    let mut last_error: Option<String>;
    loop {
        ticks += 1;
        let local_change_count = if file_aware {
            Some(pending_working_tree_change_count(&root)?)
        } else {
            None
        };
        let local_changes_due = local_change_count.unwrap_or_default() > 0;
        let remote_poll_due =
            remote_poll_ticks.is_some_and(|interval| ticks.is_multiple_of(interval));
        let should_sync = !file_aware || ticks == 1 || local_changes_due || remote_poll_due;

        if should_sync {
            match sync_once(env, args, "daemon.watch.tick") {
                Ok(report) => {
                    last_status = Some(report.status);
                    last_error = None;
                    consecutive_failures = 0;
                    retry_backoff_millis = 0;
                }
                Err(error) => {
                    failures += 1;
                    consecutive_failures += 1;
                    retry_backoff_millis = daemon_retry_backoff_millis(poll, consecutive_failures);
                    let error = error.to_string();
                    last_error = Some(error.clone());
                    mutate_agent_state(env, |state, now| {
                        state.sync.status = format!("blocked: {error}");
                        state.add_activity(
                            now,
                            "daemon.watch.blocked",
                            format!("Sync blocked during daemon watch: {error}"),
                        );
                        Ok(())
                    })?;
                }
            }
        } else {
            skipped_ticks += 1;
            consecutive_failures = 0;
            retry_backoff_millis = 0;
            last_status = Some("idle-no-local-changes".to_owned());
            last_error = None;
            mutate_agent_state(env, |state, now| {
                state.sync.status = "idle-no-local-changes".to_owned();
                state.add_activity(
                    now,
                    "daemon.watch.idle",
                    "No local Vault Working Tree changes detected",
                );
                Ok(())
            })?;
        }

        mutate_agent_state(env, |state, now| {
            state.daemon.state = DaemonRunState::Running;
            state.daemon.last_tick_at = Some(now.clone());
            state.daemon.tick_count = ticks as u64;
            state.daemon.failure_count = failures as u64;
            state.daemon.retry_backoff_millis = retry_backoff_millis;
            state.daemon.watch_strategy = Some(watch_strategy.to_owned());
            state.daemon.last_error = last_error.clone();
            state.daemon.last_local_change_count = local_change_count;
            if should_sync && local_changes_due {
                state.add_activity(
                    now,
                    "daemon.watch.local_changes_detected",
                    format!(
                        "Detected {count} pending Vault Working Tree change(s)",
                        count = local_change_count.unwrap_or_default()
                    ),
                );
            }
            Ok(())
        })?;

        if max_ticks.is_some_and(|limit| ticks >= limit) {
            break;
        }
        std::thread::sleep(poll + std::time::Duration::from_millis(retry_backoff_millis));
    }

    let final_status = last_status
        .clone()
        .or_else(|| last_error.as_ref().map(|error| format!("blocked: {error}")))
        .unwrap_or_else(|| "idle".to_owned());
    mutate_agent_state(env, |state, now| {
        state.daemon.state = DaemonRunState::Stopped;
        state.daemon.failure_count = failures as u64;
        state.daemon.retry_backoff_millis = retry_backoff_millis;
        state.daemon.last_error = last_error.clone();
        state.sync.status = final_status.clone();
        state.add_activity(
            now,
            "daemon.watch.stopped",
            format!("Agent Sync Daemon watch loop stopped after {ticks} tick(s)"),
        );
        Ok(())
    })?;

    let report = serde_json::json!({
        "state": "stopped",
        "ticks": ticks,
        "skippedTicks": skipped_ticks,
        "failures": failures,
        "lastStatus": last_status,
        "lastError": last_error,
        "watchStrategy": watch_strategy,
        "retryBackoffMillis": retry_backoff_millis,
    });
    if json {
        write_json(output, &report)
    } else {
        writeln!(
            output,
            "daemon watch stopped ticks={ticks} skipped={skipped_ticks} failures={failures} status={final_status}"
        )?;
        Ok(())
    }
}

fn daemon_watch_remote_poll_ticks(args: &[String]) -> Result<Option<usize>, CliError> {
    option_value(args, "--remote-poll-ticks")
        .map(|value| {
            value.parse::<usize>().map_err(|_| {
                CliError::InvalidInput(format!(
                    "--remote-poll-ticks must be a non-negative integer, got {value}"
                ))
            })
        })
        .transpose()?
        .map(|ticks| if ticks == 0 { None } else { Some(ticks) })
        .map(Ok)
        .unwrap_or(Ok(Some(12)))
}

fn daemon_retry_backoff_millis(poll: std::time::Duration, consecutive_failures: usize) -> u64 {
    if consecutive_failures == 0 {
        return 0;
    }
    let multiplier = 1_u128 << consecutive_failures.saturating_sub(1).min(3);
    let millis = poll.as_millis().saturating_mul(multiplier);
    millis.min(60_000) as u64
}

fn daemon_watch_max_ticks(args: &[String]) -> Result<Option<usize>, CliError> {
    if args.iter().any(|arg| arg == "--once") {
        return Ok(Some(1));
    }
    option_value(args, "--max-ticks")
        .map(|value| {
            value.parse::<usize>().map_err(|_| {
                CliError::InvalidInput(format!(
                    "--max-ticks must be a positive integer, got {value}"
                ))
            })
        })
        .transpose()?
        .map(|ticks| {
            if ticks == 0 {
                Err(CliError::InvalidInput(
                    "--max-ticks must be greater than zero".to_owned(),
                ))
            } else {
                Ok(Some(ticks))
            }
        })
        .transpose()
        .map(Option::flatten)
}

fn daemon_watch_poll(args: &[String]) -> Result<std::time::Duration, CliError> {
    if let Some(value) = option_value(args, "--poll-ms") {
        let millis = value.parse::<u64>().map_err(|_| {
            CliError::InvalidInput(format!("--poll-ms must be a positive integer, got {value}"))
        })?;
        if !(10..=300_000).contains(&millis) {
            return Err(CliError::InvalidInput(
                "--poll-ms must be between 10 and 300000".to_owned(),
            ));
        }
        return Ok(std::time::Duration::from_millis(millis));
    }
    let seconds = option_value(args, "--poll-secs")
        .map(|value| {
            value.parse::<u64>().map_err(|_| {
                CliError::InvalidInput(format!(
                    "--poll-secs must be a positive integer, got {value}"
                ))
            })
        })
        .transpose()?
        .unwrap_or(5);
    if !(1..=300).contains(&seconds) {
        return Err(CliError::InvalidInput(
            "--poll-secs must be between 1 and 300".to_owned(),
        ));
    }
    Ok(std::time::Duration::from_secs(seconds))
}

fn sync<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str).unwrap_or("status") {
        "status" => {
            let report = status_report(env)?;
            if json {
                write_json(output, &report.sync)
            } else {
                writeln!(
                    output,
                    "{} ({}) latestSequence={}",
                    report.sync.mode, report.sync.status, report.sync.latest_sequence
                )?;
                Ok(())
            }
        }
        "now" => {
            let report = sync_once(env, args, "sync.now")?;
            if json {
                write_json(output, &report)
            } else {
                writeln!(
                    output,
                    "{} latestSequence={}",
                    report.status, report.latest_sequence
                )?;
                if args
                    .iter()
                    .any(|arg| arg == "--summary" || arg == "--verbose" || arg == "-v")
                {
                    write_sync_change_rows(output, &report)?;
                }
                Ok(())
            }
        }
        other => Err(CliError::InvalidCommand(format!("sync {other}"))),
    }
}

fn write_sync_change_rows<W: Write>(
    output: &mut W,
    report: &SyncOnceReport,
) -> Result<(), CliError> {
    write_sync_change_group(output, "local changes", &report.local_changes)?;
    write_sync_change_group(output, "remote changes", &report.remote_changes)?;
    write_sync_change_group(output, "conflicts", &report.conflicts)
}

fn write_sync_change_group<W: Write>(
    output: &mut W,
    label: &str,
    changes: &[SyncChangeReport],
) -> Result<(), CliError> {
    if changes.is_empty() {
        writeln!(output, "{label}: none")?;
        return Ok(());
    }
    writeln!(output, "{label}:")?;
    for change in changes {
        let path = change
            .path
            .as_deref()
            .or(change.object_id.as_deref())
            .unwrap_or("-");
        let sequence = change
            .sequence
            .map(|sequence| format!(" seq={sequence}"))
            .unwrap_or_default();
        if let Some(from_path) = change.from_path.as_deref() {
            writeln!(
                output,
                "- {} {} {} -> {}{}",
                change.status, change.action, from_path, path, sequence
            )?;
        } else {
            writeln!(
                output,
                "- {} {} {}{}",
                change.status, change.action, path, sequence
            )?;
        }
        if let Some(reason) = change.reason.as_deref() {
            writeln!(output, "  reason: {reason}")?;
        }
    }
    Ok(())
}

fn open_vault<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let vault_id = args.first().ok_or(CliError::MissingArgument("vault-id"))?;
    let path = positional_values(args)
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| env.cwd.join(vault_id));
    let server_url = configured_server_url_for_open(args);
    if let Some(server_url) = server_url.as_deref() {
        validate_http_url(server_url)?;
    }
    fs::create_dir_all(path.join(".finitebrain/encrypted-sync"))?;
    let now = timestamp(env);
    // Opening a Vault Working Tree needs the acting identity (it records the
    // owner npub and immediately attempts a signed sync): mint on use.
    let auth = load_signer(env)?;
    let directory = VaultDirectoryManifest {
        version: VAULT_DIRECTORY_VERSION.to_owned(),
        vault: VaultDirectoryVaultSummary {
            id: vault_id.to_owned(),
            kind: "unknown".to_owned(),
            name: vault_id.to_owned(),
            owner_npub: Some(auth.npub.clone()),
        },
        working_tree: VaultDirectoryPath {
            path: ".".to_owned(),
        },
        encrypted_sync: VaultDirectoryPath {
            path: ".finitebrain/encrypted-sync".to_owned(),
        },
        portability: VaultDirectoryPortability {
            owned_by_agent_runtime: true,
            owned_by_app_surface: false,
        },
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let tree_state = VaultWorkingTreeStateManifest {
        version: WORKING_TREE_STATE_VERSION.to_owned(),
        folder_roots: Vec::<WorkingTreeFolderRoot>::new(),
        objects: Vec::<WorkingTreeObjectManifestEntry>::new(),
        sync: WorkingTreeSyncState { latest_sequence: 0 },
    };
    write_json_file(&path.join(".finitebrain/vault-directory.json"), &directory)?;
    write_json_file(
        &path.join(".finitebrain/working-tree-state.json"),
        &tree_state,
    )?;

    let mut state = AgentState::new(vault_id, &now);
    state.server_url = server_url;
    state.daemon.state = DaemonRunState::Running;
    state.daemon.last_started_at = Some(now.clone());
    state.auth_npub = Some(auth.npub);
    state.add_activity(
        now,
        "working_tree.opened",
        "Vault Working Tree opened for agent use",
    );
    write_agent_state(&path, &state)?;
    let mut opened_env = env.clone();
    opened_env.cwd = path.clone();
    let sync_status = match sync_once(&opened_env, args, "working_tree.opened.sync") {
        Ok(report) => report.status,
        Err(error) => {
            let mut state = read_agent_state(&path)?;
            let now = timestamp(env);
            state.sync.status = format!("blocked: {error}");
            state.add_activity(
                now,
                "sync.blocked",
                format!("Automatic sync blocked: {error}"),
            );
            write_agent_state(&path, &state)?;
            format!("blocked: {error}")
        }
    };

    if json {
        write_json(
            output,
            &serde_json::json!({
                "vaultId": vault_id,
                "path": path,
                "daemon": "running",
                "syncMode": "automatic",
                "syncStatus": sync_status
            }),
        )
    } else {
        writeln!(output, "opened Vault Working Tree {}", path.display())?;
        Ok(())
    }
}

fn status<W: Write>(env: &CliEnvironment, json: bool, output: &mut W) -> Result<(), CliError> {
    let report = status_report(env)?;
    if json {
        write_json(output, &report)
    } else {
        writeln!(
            output,
            "Vault: {}",
            report.vault_id.as_deref().unwrap_or("-")
        )?;
        writeln!(
            output,
            "Tree: {}",
            report.working_tree_path.as_deref().unwrap_or("-")
        )?;
        writeln!(output, "Auth: {}", report.auth.state)?;
        writeln!(output, "Daemon: {}", report.daemon.state)?;
        writeln!(
            output,
            "Sync: {} ({})",
            report.sync.mode, report.sync.status
        )?;
        writeln!(
            output,
            "Unlocked Folders: {}",
            report.unlocked_folders.len()
        )?;
        writeln!(output, "Conflicts: {}", report.conflicts.len())?;
        Ok(())
    }
}

fn unlock<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let all = args.iter().any(|arg| arg == "--all");
    let target = args.iter().find(|arg| !arg.starts_with("--")).cloned();
    let root = current_tree_root(env)?;
    let tree = read_working_tree_state(&root)?;
    let mut opened = Vec::new();
    mutate_agent_state(env, |state, now| {
        let mut known = state
            .unlocked_folders
            .iter()
            .map(|folder| folder.folder_id.clone())
            .collect::<BTreeSet<_>>();
        let candidates = if all {
            tree.folder_roots
                .iter()
                .filter(|root| root.can_read)
                .map(|root| root.folder_id.clone())
                .collect::<Vec<_>>()
        } else {
            vec![
                target
                    .clone()
                    .ok_or(CliError::MissingArgument("folder or --all"))?,
            ]
        };
        for folder_id in candidates {
            if known.insert(folder_id.clone()) {
                state.unlocked_folders.push(UnlockedFolder {
                    vault_id: Some(state.vault_id.clone()),
                    folder_id: folder_id.clone(),
                    key_version: 1,
                    opened_at: now.clone(),
                    source: "prototype-local-signer".to_owned(),
                });
                opened.push(folder_id);
            }
        }
        state.add_activity(
            now,
            "folder_keys.opened",
            "Folder Keys opened in local session",
        );
        Ok(())
    })?;
    if json {
        write_json(output, &serde_json::json!({ "opened": opened }))
    } else if opened.is_empty() {
        writeln!(output, "no new Folder Keys opened")?;
        Ok(())
    } else {
        writeln!(output, "opened {}", opened.join(", "))?;
        Ok(())
    }
}

fn conflicts<W: Write>(env: &CliEnvironment, json: bool, output: &mut W) -> Result<(), CliError> {
    let state = load_current_agent_state(env)?;
    let active = state
        .conflicts
        .into_iter()
        .filter(|conflict| conflict.state == ConflictState::Open)
        .collect::<Vec<_>>();
    if json {
        write_json(output, &active)
    } else if active.is_empty() {
        writeln!(output, "no conflicts")?;
        Ok(())
    } else {
        for conflict in active {
            writeln!(output, "{} {}", conflict.id, conflict.reason)?;
        }
        Ok(())
    }
}

fn resolve<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let conflict_id = args
        .first()
        .ok_or(CliError::MissingArgument("conflict-id"))?;
    let mut found = false;
    mutate_agent_state(env, |state, now| {
        for conflict in &mut state.conflicts {
            if conflict.id == *conflict_id {
                conflict.state = ConflictState::Resolved;
                conflict.resolved_at = Some(now.clone());
                found = true;
            }
        }
        if !found {
            return Err(CliError::NotFound(conflict_id.clone()));
        }
        state.add_activity(
            now,
            "conflict.resolved",
            format!("Conflict {conflict_id} marked resolved"),
        );
        Ok(())
    })?;
    if json {
        write_json(output, &serde_json::json!({ "resolved": conflict_id }))
    } else {
        writeln!(output, "resolved {conflict_id}")?;
        Ok(())
    }
}

fn activity<W: Write>(env: &CliEnvironment, json: bool, output: &mut W) -> Result<(), CliError> {
    let state = load_current_agent_state(env)?;
    if json {
        write_json(output, &state.activity)
    } else {
        write_activity_rows(output, &state.activity)
    }
}

struct VaultCreateBootstrapPlan {
    bootstrap_grants: Vec<serde_json::Value>,
    folder_keys: BTreeMap<(String, u32), FolderKey>,
    vault_kind: VaultKind,
}

fn bootstrap_plan_for_vault_create(
    env: &CliEnvironment,
    vault_id: &str,
    kind: &str,
    name: &str,
) -> Result<VaultCreateBootstrapPlan, CliError> {
    let auth = load_signer(env)?;
    let (vault_kind, output) = match kind {
        "personal" => (
            VaultKind::Personal,
            bootstrap_personal_vault(vault_id, name, auth.npub.clone()),
        ),
        "organization" => (
            VaultKind::Organization,
            bootstrap_organization_vault(vault_id, name, auth.npub.clone()),
        ),
        other => {
            return Err(CliError::InvalidInput(format!(
                "unknown vault kind {other}"
            )));
        }
    };
    let output = output.map_err(|error| CliError::InvalidInput(error.to_string()))?;

    let mut folder_keys = BTreeMap::<(String, u32), FolderKey>::new();
    let bootstrap_grants = output
        .required_key_grants
        .into_iter()
        .map(|required| {
            let folder_id = required.folder_id.to_string();
            let folder_key = folder_keys
                .entry((folder_id.clone(), required.key_version))
                .or_insert_with(FolderKey::generate);
            let recipient = required.recipient_user_id.to_string();
            let grant = folder_key_grant_request(
                &auth,
                vault_id,
                &folder_id,
                required.key_version,
                &recipient,
                folder_key,
                env,
            )?;
            Ok(serde_json::json!({
                "folderId": folder_id,
                "grant": grant
            }))
        })
        .collect::<Result<Vec<_>, CliError>>()?;

    Ok(VaultCreateBootstrapPlan {
        bootstrap_grants,
        folder_keys,
        vault_kind,
    })
}

fn write_default_vault_pages_for_create(
    env: &CliEnvironment,
    server_url: &str,
    vault_id: &str,
    plan: &VaultCreateBootstrapPlan,
) -> Result<(), CliError> {
    let auth = load_signer(env)?;
    let keys = auth.keys.clone();
    let key_version = 1;

    for page in default_vault_pages(plan.vault_kind) {
        let folder_id = FolderId::new(page.folder_id)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let folder_key = plan
            .folder_keys
            .get(&(page.folder_id.to_owned(), key_version))
            .ok_or_else(|| {
                CliError::InvalidInput(format!(
                    "missing generated Folder Key for {}",
                    page.folder_id
                ))
            })?;
        let object_id = ObjectId::new(page.object_id)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let page_path = SafeRelativePath::new("page_path", page.path)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let aad = FolderObjectAad {
            vault_id: VaultId::new(vault_id.to_owned())
                .map_err(|error| CliError::InvalidInput(error.to_string()))?,
            folder_id: folder_id.clone(),
            object_id: object_id.clone(),
            key_version,
        };
        let plaintext = encode_folder_object_page_plaintext(&page_path, page.markdown)?;
        let envelope = encrypt_folder_object(folder_key, &aad, plaintext)
            .map_err(|error| CliError::InvalidInput(error.to_string()))?;
        let envelope_json = envelope.canonical_json();
        let revision_event = signed_revision_event(
            &keys,
            RevisionEventInput {
                actor_npub: &auth.npub,
                vault_id,
                folder_id: &folder_id,
                object_id: &object_id,
                operation: FolderObjectOperation::Create,
                base_revision: None,
                key_version,
                envelope_json: envelope_json.clone(),
            },
        )?;
        let body = serde_json::json!({
            "baseRevision": null,
            "keyVersion": key_version,
            "cipher": CIPHER_AES_256_GCM,
            "ciphertext": envelope_json,
            "revisionEvent": revision_event
        });
        let route = format!(
            "/_admin/vaults/{vault_id}/folders/{}/objects/{}",
            folder_id.as_str(),
            object_id.as_str()
        );
        signed_json_request_to_server(env, server_url, "PUT", &route, Some(body))?;
    }

    Ok(())
}

fn access<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str) {
        Some("explain") => {
            let folder = args.get(1).ok_or(CliError::MissingArgument("folder"))?;
            let root = current_tree_root(env)?;
            let tree = read_working_tree_state(&root)?;
            let explanation = explain_access(folder, &tree);
            if json {
                write_json(output, &explanation)
            } else {
                writeln!(output, "{}: {}", explanation.folder, explanation.reason)?;
                Ok(())
            }
        }
        Some("list") | Some("ls") => {
            let metadata = fetch_vault_metadata_for_command(env, args)?;
            let report = access_summary_report(metadata);
            if json {
                write_json(output, &report)
            } else {
                write_access_summary_rows(output, &report)
            }
        }
        Some("grant") | Some("grant-folder") | Some("folder-grant") => {
            let mut delegated = Vec::with_capacity(args.len());
            delegated.push("grant-folder".to_owned());
            delegated.extend(args.iter().skip(1).cloned());
            permissions(&delegated, env, json, output)
        }
        Some("revoke") | Some("remove") | Some("remove-folder") | Some("revoke-folder") => {
            access_revoke(args, env, json, output)
        }
        Some(other) => Err(CliError::InvalidCommand(format!("access {other}"))),
        None => Err(CliError::MissingArgument("access command")),
    }
}

fn access_revoke<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let vault_id = command_vault_id(args, env)?;
    let folder_id = option_value(args, "--folder").ok_or(CliError::MissingArgument("--folder"))?;
    let target = required_option_or_positional(args, "--target", 1, "target-npub")?;
    let route = format!("/_admin/vaults/{vault_id}/folders/{folder_id}/access/{target}");
    let Some(body) = access_rotation_body(args)? else {
        let report = AccessRemovalBlockedReport {
            state: "blocked".to_owned(),
            operation: "remove-folder-access".to_owned(),
            vault_id,
            folder_id,
            target_npub: target,
            route,
            reason: "Folder access removal requires Folder Key rotation and re-encrypted live Folder objects; refusing unsafe metadata-only removal".to_owned(),
            required: vec![
                "newKeyVersion equal to the next Folder Key version".to_owned(),
                "Folder Key Grants for every remaining recipient".to_owned(),
                "reencryptedRecords for every live readable object in the Folder".to_owned(),
                "admin access-change event with action remove-folder-access".to_owned(),
            ],
        };
        if json {
            return write_json(output, &report);
        }
        writeln!(
            output,
            "blocked remove-folder-access vault={} folder={} target={}",
            report.vault_id, report.folder_id, report.target_npub
        )?;
        for requirement in &report.required {
            writeln!(output, "- requires {requirement}")?;
        }
        return Ok(());
    };

    validate_access_rotation_body(&body)?;
    let response = signed_json_request(env, args, "DELETE", &route, Some(body))?;
    write_command_response(output, json, &response)
}

fn access_rotation_body(args: &[String]) -> Result<Option<serde_json::Value>, CliError> {
    let Some(path) = option_value(args, "--rotation-body").or_else(|| option_value(args, "--body"))
    else {
        return Ok(None);
    };
    let body = fs::read_to_string(&path)?;
    serde_json::from_str(&body)
        .map(Some)
        .map_err(CliError::from)
}

fn validate_access_rotation_body(body: &serde_json::Value) -> Result<(), CliError> {
    let object = body
        .as_object()
        .ok_or_else(|| CliError::InvalidInput("rotation body must be a JSON object".to_owned()))?;
    let new_key_version = object
        .get("newKeyVersion")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    if new_key_version == 0 {
        return Err(CliError::InvalidInput(
            "rotation body needs positive newKeyVersion".to_owned(),
        ));
    }
    let grants = object
        .get("grants")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::InvalidInput("rotation body needs grants array".to_owned()))?;
    if grants.is_empty() {
        return Err(CliError::InvalidInput(
            "rotation body needs at least one Folder Key Grant".to_owned(),
        ));
    }
    if !object
        .get("reencryptedRecords")
        .is_some_and(serde_json::Value::is_array)
    {
        return Err(CliError::InvalidInput(
            "rotation body needs reencryptedRecords array".to_owned(),
        ));
    }
    if !object
        .get("accessChangeEvent")
        .is_some_and(serde_json::Value::is_object)
    {
        return Err(CliError::InvalidInput(
            "rotation body needs accessChangeEvent object".to_owned(),
        ));
    }
    Ok(())
}

fn fetch_vault_metadata_for_command(
    env: &CliEnvironment,
    args: &[String],
) -> Result<VaultMetadataView, CliError> {
    let vault_id = command_vault_id(args, env)?;
    fetch_vault_metadata(env, args, &vault_id)
}

fn access_summary_report(metadata: VaultMetadataView) -> AccessSummaryReport {
    AccessSummaryReport {
        vault_id: metadata.vault_id,
        members: metadata.members,
        admins: metadata.admins,
        folders: metadata.folders,
        mounted_folders: metadata.mounted_folders,
        grant_count: metadata.grant_count,
    }
}

fn write_access_summary_rows<W: Write>(
    output: &mut W,
    report: &AccessSummaryReport,
) -> Result<(), CliError> {
    writeln!(
        output,
        "vault {} admins={} members={} grants={}",
        report.vault_id,
        report.admins.len(),
        report.members.len(),
        report.grant_count
    )?;
    write_folder_rows(output, &report.folders)?;
    write_mount_rows(output, &report.mounted_folders)
}

fn vault<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str).unwrap_or("metadata") {
        "create" => {
            let values = positional_values(args);
            let vault_id = values.get(1).ok_or(CliError::MissingArgument("vault-id"))?;
            let kind = option_value(args, "--kind").unwrap_or_else(|| "personal".to_owned());
            let normalized_kind = normalize_vault_kind(&kind)?;
            let name = option_value(args, "--name").unwrap_or_else(|| vault_id.clone());
            let bootstrap_plan =
                bootstrap_plan_for_vault_create(env, vault_id, normalized_kind, &name)?;
            let body = serde_json::json!({
                "vaultId": vault_id,
                "kind": normalized_kind,
                "name": name,
                "bootstrapGrants": bootstrap_plan.bootstrap_grants
            });
            let server_url = server_url_for_command(env, args)?;
            let response = signed_json_request_to_server(
                env,
                &server_url,
                "POST",
                "/_admin/vaults",
                Some(body),
            )?;
            write_default_vault_pages_for_create(env, &server_url, vault_id, &bootstrap_plan)?;
            write_command_response(output, json, &response)
        }
        "metadata" | "status" => {
            let vault_id = option_value(args, "--vault")
                .or_else(|| positional_values(args).get(1).cloned())
                .or_else(|| current_vault_id(env))
                .ok_or(CliError::MissingArgument("vault-id or --vault"))?;
            let path = format!("/_admin/vaults/{vault_id}/metadata");
            let response = signed_json_request(env, args, "GET", &path, None)?;
            write_command_response(output, json, &response)
        }
        "export" => {
            let vault_id = option_value(args, "--vault")
                .or_else(|| positional_values(args).get(1).cloned())
                .or_else(|| current_vault_id(env))
                .ok_or(CliError::MissingArgument("vault-id or --vault"))?;
            let path = format!("/_admin/vaults/{vault_id}/export");
            let response = signed_json_request(env, args, "GET", &path, None)?;
            write_command_response(output, json, &response)
        }
        other => Err(CliError::InvalidCommand(format!("vault {other}"))),
    }
}

fn folder<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str).unwrap_or("create") {
        "list" | "ls" => {
            let metadata = fetch_vault_metadata_for_command(env, args)?;
            if json {
                write_json(output, &metadata.folders)
            } else {
                write_folder_rows(output, &metadata.folders)
            }
        }
        "create" => {
            let values = positional_values(args);
            let folder_id = values
                .get(1)
                .ok_or(CliError::MissingArgument("folder-id"))?;
            let vault_id = command_vault_id(args, env)?;
            let name = option_value(args, "--name").unwrap_or_else(|| folder_id.clone());
            let path = option_value(args, "--path").unwrap_or_else(|| name.clone());
            let role = option_value(args, "--role").unwrap_or_else(|| "folder".to_owned());
            let metadata = fetch_vault_metadata(env, args, &vault_id)?;
            let access = option_value(args, "--access").unwrap_or_else(|| {
                if metadata.kind == "personal" {
                    "owner".to_owned()
                } else {
                    "restricted".to_owned()
                }
            });
            let access_users = option_values(args, "--member")
                .into_iter()
                .map(|input| resolve_identity_npub(env, args, &input))
                .collect::<Result<Vec<_>, _>>()?;
            let recipients = folder_required_recipients(&metadata, &access, &access_users)?;
            let folder_key = FolderKey::generate();
            let auth = load_signer(env)?;
            let event = admin_access_change_event(
                env,
                &vault_id,
                AdminAccessAction::SetFolderAccessMode,
                Some(folder_id),
                None,
                Some(1),
            )?;
            let grants = recipients
                .iter()
                .map(|recipient| {
                    folder_key_grant_request(
                        &auth,
                        &vault_id,
                        folder_id,
                        1,
                        recipient,
                        &folder_key,
                        env,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let body = serde_json::json!({
                "folderId": folder_id,
                "name": name,
                "role": normalize_folder_role(&role)?,
                "access": normalize_folder_access(&access)?,
                "parentFolderId": option_value(args, "--parent"),
                "path": path,
                "sharedFolderSource": args.iter().any(|arg| arg == "--shared-source"),
                "accessUserIds": access_users,
                "grants": grants,
                "accessChangeEvent": event
            });
            let route = format!("/_admin/vaults/{vault_id}/folders");
            let response = signed_json_request(env, args, "POST", &route, Some(body))?;
            update_local_folder_after_create(env, folder_id, &path)?;
            write_command_response(output, json, &response)
        }
        other => Err(CliError::InvalidCommand(format!("folder {other}"))),
    }
}

fn mount<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str).unwrap_or("list") {
        "list" | "ls" => {
            let metadata = fetch_vault_metadata_for_command(env, args)?;
            if json {
                write_json(output, &metadata.mounted_folders)
            } else {
                write_mount_rows(output, &metadata.mounted_folders)
            }
        }
        other => Err(CliError::InvalidCommand(format!("mount {other}"))),
    }
}

fn write_folder_rows<W: Write>(
    output: &mut W,
    folders: &[FolderMetadataView],
) -> Result<(), CliError> {
    if folders.is_empty() {
        writeln!(output, "no folders")?;
        return Ok(());
    }
    for folder in folders {
        let setup = if folder.setup_incomplete {
            "setup-incomplete"
        } else {
            "ready"
        };
        let source = if folder.shared_folder_source {
            " shared-source"
        } else {
            ""
        };
        writeln!(
            output,
            "folder {} path={} access={} keyVersion={} state={}{}",
            folder.id, folder.path, folder.access, folder.current_key_version, setup, source
        )?;
    }
    Ok(())
}

fn write_mount_rows<W: Write>(
    output: &mut W,
    mounts: &[MountedFolderMetadataView],
) -> Result<(), CliError> {
    if mounts.is_empty() {
        writeln!(output, "no mounted folders")?;
        return Ok(());
    }
    for mount in mounts {
        writeln!(
            output,
            "mount {} name={} source={}/{} state={}",
            mount.mount_id,
            mount.display_name,
            mount.source_vault_id,
            mount.source_folder_id,
            mount.state
        )?;
    }
    Ok(())
}

fn permissions<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str) {
        Some("add-member") | Some("member-add") => {
            let vault_id = command_vault_id(args, env)?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let event = admin_access_change_event(
                env,
                &vault_id,
                AdminAccessAction::AddMember,
                None,
                Some(&target),
                None,
            )?;
            let body = serde_json::json!({
                "targetNpub": target,
                "accessChangeEvent": event
            });
            let route = format!("/_admin/vaults/{vault_id}/members");
            let response = signed_json_request(env, args, "POST", &route, Some(body))?;
            write_command_response(output, json, &response)
        }
        Some("remove-member") | Some("member-remove") => {
            let vault_id = command_vault_id(args, env)?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let event = admin_access_change_event(
                env,
                &vault_id,
                AdminAccessAction::RemoveMember,
                None,
                Some(&target),
                None,
            )?;
            let route = format!("/_admin/vaults/{vault_id}/members/{target}");
            let response = signed_json_request(
                env,
                args,
                "DELETE",
                &route,
                Some(serde_json::json!({ "accessChangeEvent": event })),
            )?;
            write_command_response(output, json, &response)
        }
        Some("add-admin") | Some("admin-add") => {
            let vault_id = command_vault_id(args, env)?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let event = admin_access_change_event(
                env,
                &vault_id,
                AdminAccessAction::AddAdmin,
                None,
                Some(&target),
                None,
            )?;
            let body = serde_json::json!({
                "targetNpub": target,
                "accessChangeEvent": event
            });
            let route = format!("/_admin/vaults/{vault_id}/admins");
            let response = signed_json_request(env, args, "POST", &route, Some(body))?;
            write_command_response(output, json, &response)
        }
        Some("remove-admin") | Some("admin-remove") => {
            let vault_id = command_vault_id(args, env)?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let event = admin_access_change_event(
                env,
                &vault_id,
                AdminAccessAction::RemoveAdmin,
                None,
                Some(&target),
                None,
            )?;
            let route = format!("/_admin/vaults/{vault_id}/admins/{target}");
            let response = signed_json_request(
                env,
                args,
                "DELETE",
                &route,
                Some(serde_json::json!({ "accessChangeEvent": event })),
            )?;
            write_command_response(output, json, &response)
        }
        Some("grant-folder") | Some("folder-grant") => {
            let vault_id = command_vault_id(args, env)?;
            let folder_id =
                option_value(args, "--folder").ok_or(CliError::MissingArgument("--folder"))?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let metadata = fetch_vault_metadata(env, args, &vault_id)?;
            let key_version = metadata
                .folders
                .iter()
                .find(|folder| folder.id == folder_id)
                .map(|folder| folder.current_key_version)
                .ok_or_else(|| CliError::NotFound(format!("folder {folder_id}")))?;
            let session_keys = open_vault_session_folder_keys(env, args, &vault_id)?;
            let folder_key = opened_folder_key(&session_keys, &vault_id, &folder_id, key_version)?;
            let auth = load_signer(env)?;
            let event = admin_access_change_event(
                env,
                &vault_id,
                AdminAccessAction::GrantFolderAccess,
                Some(&folder_id),
                Some(&target),
                Some(key_version),
            )?;
            let grant = folder_key_grant_request(
                &auth,
                &vault_id,
                &folder_id,
                key_version,
                &target,
                &folder_key,
                env,
            )?;
            let route = format!("/_admin/vaults/{vault_id}/folders/{folder_id}/access");
            let body = serde_json::json!({
                "targetNpub": target,
                "grant": grant,
                "accessChangeEvent": event
            });
            let response = signed_json_request(env, args, "POST", &route, Some(body))?;
            write_command_response(output, json, &response)
        }
        Some(other) => Err(CliError::InvalidCommand(format!("permissions {other}"))),
        None => Err(CliError::MissingArgument("permissions command")),
    }
}

fn invites<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str) {
        Some("create") => {
            let vault_id = command_vault_id(args, env)?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let expires_at = option_value(args, "--expires")
                .unwrap_or_else(|| "2099-01-01T00:00:00Z".to_owned());
            let folders = option_values(args, "--folder");
            let route = format!("/_admin/vaults/{vault_id}/invitations");
            if let Ok(public_key) = NostrPublicKey::parse(&raw_target) {
                let target = public_key
                    .to_npub()
                    .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
                write_npub_invite_create(
                    output,
                    json,
                    env,
                    args,
                    &route,
                    &target,
                    &folders,
                    &expires_at,
                )
            } else if invite_finite_vip_email(&raw_target) {
                match resolve_identity_npub(env, args, &raw_target) {
                    Ok(target) => write_npub_invite_create(
                        output,
                        json,
                        env,
                        args,
                        &route,
                        &target,
                        &folders,
                        &expires_at,
                    ),
                    Err(_) => write_email_invite_create(
                        output,
                        json,
                        env,
                        args,
                        &route,
                        &vault_id,
                        &raw_target,
                        &folders,
                        &expires_at,
                    ),
                }
            } else if invite_email_like(&raw_target) {
                write_email_invite_create(
                    output,
                    json,
                    env,
                    args,
                    &route,
                    &vault_id,
                    &raw_target,
                    &folders,
                    &expires_at,
                )
            } else {
                let target = resolve_identity_npub(env, args, &raw_target)?;
                write_npub_invite_create(
                    output,
                    json,
                    env,
                    args,
                    &route,
                    &target,
                    &folders,
                    &expires_at,
                )
            }
        }
        Some("show") => {
            let code = require_invite_code(required_option_or_positional(
                args,
                "--code",
                1,
                "invite-code",
            )?)?;
            let route = format!("/_admin/vault-invitation-links/{code}");
            let response = signed_json_request(env, args, "GET", &route, None)?;
            write_command_response(output, json, &response)
        }
        Some("accept") => {
            let route = if let Some(code) =
                option_value(args, "--code").or_else(|| positional_values(args).get(1).cloned())
            {
                let code = require_invite_code(code)?;
                format!("/_admin/vault-invitation-links/{code}/accept")
            } else {
                let vault_id = command_vault_id(args, env)?;
                let id = option_value(args, "--id")
                    .ok_or(CliError::MissingArgument("--id or --code"))?;
                format!("/_admin/vaults/{vault_id}/invitations/{id}/accept")
            };
            let response = signed_json_request(env, args, "POST", &route, None)?;
            write_command_response(output, json, &response)
        }
        Some("revoke") => {
            let vault_id = command_vault_id(args, env)?;
            let id = required_option_or_positional(args, "--id", 1, "invitation-id")?;
            let route = format!("/_admin/vaults/{vault_id}/invitations/{id}");
            let response = signed_json_request(env, args, "DELETE", &route, None)?;
            write_command_response(output, json, &response)
        }
        Some(other) => Err(CliError::InvalidCommand(format!("invites {other}"))),
        None => Err(CliError::MissingArgument("invites command")),
    }
}

#[allow(clippy::too_many_arguments)]
fn write_npub_invite_create<W: Write>(
    output: &mut W,
    json: bool,
    env: &CliEnvironment,
    args: &[String],
    route: &str,
    target: &str,
    folders: &[String],
    expires_at: &str,
) -> Result<(), CliError> {
    let body = serde_json::json!({
        "targetNpub": target,
        "initialFolderAccess": folders,
        "expiresAt": expires_at
    });
    let response = signed_json_request(env, args, "POST", route, Some(body))?;
    write_command_response(output, json, &response)
}

#[allow(clippy::too_many_arguments)]
fn write_email_invite_create<W: Write>(
    output: &mut W,
    json: bool,
    env: &CliEnvironment,
    args: &[String],
    route: &str,
    vault_id: &str,
    raw_target: &str,
    folders: &[String],
    expires_at: &str,
) -> Result<(), CliError> {
    let (body, invite_secret) =
        email_invite_create_body(env, args, vault_id, raw_target, folders, expires_at)?;
    let server_url = server_url_for_command(env, args)?;
    let mut response = signed_json_request_to_server(env, &server_url, "POST", route, Some(body))?;
    if let Some(object) = response.as_object_mut() {
        object.insert(
            "inviteSecret".to_owned(),
            serde_json::Value::String(invite_secret.clone()),
        );
        if let Some(accept_path) = object
            .get("acceptPath")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
        {
            object.insert(
                "inviteUrl".to_owned(),
                serde_json::Value::String(format!(
                    "{}{}#inviteSecret={}",
                    server_url.trim_end_matches('/'),
                    accept_path,
                    invite_secret
                )),
            );
        }
    }
    if json {
        write_json(output, &response)
    } else {
        write_command_response(output, false, &response)?;
        if let Some(invite_url) = response
            .get("inviteUrl")
            .and_then(serde_json::Value::as_str)
        {
            writeln!(output, "inviteUrl {invite_url}")?;
        }
        writeln!(output, "inviteSecret {invite_secret}")?;
        Ok(())
    }
}

fn require_invite_code(value: String) -> Result<String, CliError> {
    let value = value.trim().to_owned();
    if value.starts_with("invitation-") {
        return Err(CliError::InvalidInput(
            "expected an invite code like invite-...; invitation-... is an invitation id. Use `fbrain invites accept --vault <vault-id> --id <invitation-id>` for by-id accept."
                .to_owned(),
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct EmailInviteScopeItem {
    folder_id: String,
    access: String,
    key_version: u32,
}

fn invite_email_like(value: &str) -> bool {
    let value = value.trim();
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty() && !domain.is_empty() && domain.contains('.')
}

fn invite_finite_vip_email(value: &str) -> bool {
    canonical_invite_email(value)
        .map(|email| email.ends_with("@finite.vip"))
        .unwrap_or(false)
}

fn canonical_invite_email(value: &str) -> Result<String, CliError> {
    let value = value.trim().to_ascii_lowercase();
    let Some((local, domain)) = value.split_once('@') else {
        return Err(CliError::InvalidInput(
            "email invite target must be an email address".to_owned(),
        ));
    };
    if local.is_empty()
        || domain.is_empty()
        || value.len() > 320
        || value.chars().any(|c| c == '\0' || c.is_control())
    {
        return Err(CliError::InvalidInput(
            "email invite target must be a printable email address".to_owned(),
        ));
    }
    Ok(value)
}

fn email_invite_scope(
    metadata: &VaultMetadataView,
    selected_folders: &[String],
) -> Result<Vec<EmailInviteScopeItem>, CliError> {
    let selected = selected_folders.iter().cloned().collect::<BTreeSet<_>>();
    let mut seen_selected = BTreeSet::new();
    let mut included = BTreeSet::new();
    let mut scope = Vec::new();

    for folder in &metadata.folders {
        let selected_folder = selected.contains(&folder.id);
        if selected_folder {
            seen_selected.insert(folder.id.clone());
        }
        let include = match folder.access.as_str() {
            "all_members" => true,
            "restricted" => selected_folder,
            "owner" | "admin_only" => {
                if selected_folder {
                    return Err(CliError::InvalidInput(
                        "email invite bootstrap can include all-members folders and selected restricted folders only"
                            .to_owned(),
                    ));
                }
                false
            }
            other => {
                return Err(CliError::InvalidInput(format!(
                    "unknown folder access mode {other}"
                )));
            }
        };
        if include && included.insert(folder.id.clone()) {
            scope.push(EmailInviteScopeItem {
                folder_id: folder.id.clone(),
                access: folder.access.clone(),
                key_version: folder.current_key_version,
            });
        }
    }

    if seen_selected != selected {
        let missing = selected
            .difference(&seen_selected)
            .next()
            .cloned()
            .unwrap_or_else(|| "unknown".to_owned());
        return Err(CliError::NotFound(format!("folder {missing}")));
    }
    Ok(scope)
}

fn email_invite_scope_json(scope: &[EmailInviteScopeItem]) -> Vec<serde_json::Value> {
    scope
        .iter()
        .map(|item| {
            serde_json::json!({
                "folderId": item.folder_id,
                "access": item.access,
                "keyVersion": item.key_version,
            })
        })
        .collect()
}

fn email_invite_create_body(
    env: &CliEnvironment,
    args: &[String],
    vault_id: &str,
    target_email: &str,
    selected_folders: &[String],
    expires_at: &str,
) -> Result<(serde_json::Value, String), CliError> {
    let invited_email = canonical_invite_email(target_email)?;
    let metadata = fetch_vault_metadata(env, args, vault_id)?;
    let scope = email_invite_scope(&metadata, selected_folders)?;
    let auth = load_signer(env)?;
    let session_keys = open_vault_session_folder_keys(env, args, vault_id)?;
    let unwrap_keys = Keys::generate();
    let invite_unwrap_npub = NostrPublicKey::from_protocol(unwrap_keys.public_key())
        .to_npub()
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    let invite_secret = unwrap_keys.secret_key().to_secret_hex();

    let mut bootstrap_grants = Vec::new();
    for item in &scope {
        let folder_key =
            opened_folder_key(&session_keys, vault_id, &item.folder_id, item.key_version)?;
        bootstrap_grants.push(serde_json::json!({
            "folderId": item.folder_id,
            "grant": folder_key_grant_request(
                &auth,
                vault_id,
                &item.folder_id,
                item.key_version,
                &invite_unwrap_npub,
                &folder_key,
                env,
            )?,
        }));
    }

    let scope_json = email_invite_scope_json(&scope);
    let bootstrap_payload = serde_json::json!({
        "version": "finite-email-invite-bootstrap-payload-v1",
        "vaultId": vault_id,
        "invitedEmail": invited_email,
        "inviteUnwrapNpub": invite_unwrap_npub,
        "folders": scope_json,
        "grants": bootstrap_grants,
    });
    let bootstrap_payload_json = serde_json::to_string(&bootstrap_payload)?;
    let bootstrap_payload_hash = format!(
        "sha256:{}",
        hex_lower(Sha256::digest(bootstrap_payload_json.as_bytes()).as_slice())
    );
    let bootstrap_wrapped_event_json = wrap_email_invite_bootstrap_payload(
        &auth,
        vault_id,
        &invite_unwrap_npub,
        &bootstrap_payload_json,
    )?;
    let bootstrap_authorization_event_json = email_invite_authorization_event(
        &auth,
        vault_id,
        &invited_email,
        &invite_unwrap_npub,
        &bootstrap_payload_hash,
        expires_at,
        &scope,
    )?;

    Ok((
        serde_json::json!({
            "target": invited_email,
            "initialFolderAccess": selected_folders,
            "expiresAt": expires_at,
            "inviteUnwrapNpub": invite_unwrap_npub,
            "bootstrapPayloadHash": bootstrap_payload_hash,
            "bootstrapWrappedEventJson": bootstrap_wrapped_event_json,
            "bootstrapAuthorizationEventJson": bootstrap_authorization_event_json,
        }),
        invite_secret,
    ))
}

fn wrap_email_invite_bootstrap_payload(
    auth: &LocalSigner,
    vault_id: &str,
    invite_unwrap_npub: &str,
    bootstrap_payload_json: &str,
) -> Result<String, CliError> {
    let recipient = NostrPublicKey::parse(invite_unwrap_npub)
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    let rumor = build_rumor(
        NostrPublicKey::from_protocol(auth.keys.public_key()),
        Kind::Custom(APP_SPECIFIC_KIND),
        vec![
            tag_vec(["d", &format!("finite-email-invite-bootstrap:{vault_id}")])?,
            tag_vec(["vault", vault_id])?,
        ],
        bootstrap_payload_json.to_owned(),
        unix_timestamp(),
    );
    let wrapped = wrap_rumor(&auth.keys, recipient, rumor)
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    Ok(wrapped.as_json())
}

fn email_invite_authorization_event(
    auth: &LocalSigner,
    vault_id: &str,
    invited_email: &str,
    invite_unwrap_npub: &str,
    bootstrap_payload_hash: &str,
    expires_at: &str,
    scope: &[EmailInviteScopeItem],
) -> Result<String, CliError> {
    let content = serde_json::json!({
        "version": "finite-email-invite-bootstrap-authorization-v1",
        "vaultId": vault_id,
        "invitedEmail": invited_email,
        "inviteUnwrapNpub": invite_unwrap_npub,
        "bootstrapPayloadHash": bootstrap_payload_hash,
        "expiresAt": expires_at,
        "folders": email_invite_scope_json(scope),
    })
    .to_string();
    let event = sign_event(
        &auth.keys,
        Kind::Custom(APP_SPECIFIC_KIND),
        content,
        vec![
            tag_vec([
                "d",
                &format!("finite-email-invite-bootstrap-authorization:{vault_id}:{invited_email}"),
            ])?,
            tag_vec(["vault", vault_id])?,
            tag_vec(["email", invited_email])?,
        ],
        unix_timestamp(),
        None,
    )?;
    Ok(event.as_json())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("write to string");
    }
    out
}

fn share<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str) {
        Some("link") | Some("create-link") => {
            let vault_id = command_vault_id(args, env)?;
            let folder_id =
                option_value(args, "--folder").ok_or(CliError::MissingArgument("--folder"))?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let expires_at = option_value(args, "--expires")
                .unwrap_or_else(|| "2099-01-01T00:00:00Z".to_owned());
            let metadata = fetch_vault_metadata(env, args, &vault_id)?;
            let key_version = metadata
                .folders
                .iter()
                .find(|folder| folder.id == folder_id)
                .map(|folder| folder.current_key_version)
                .ok_or_else(|| CliError::NotFound(format!("folder {folder_id}")))?;
            let session_keys = open_vault_session_folder_keys(env, args, &vault_id)?;
            let folder_key = opened_folder_key(&session_keys, &vault_id, &folder_id, key_version)?;
            let auth = load_signer(env)?;
            let event = admin_access_change_event(
                env,
                &vault_id,
                AdminAccessAction::GrantFolderAccess,
                Some(&folder_id),
                Some(&target),
                Some(key_version),
            )?;
            let grant = folder_key_grant_request(
                &auth,
                &vault_id,
                &folder_id,
                key_version,
                &target,
                &folder_key,
                env,
            )?;
            let body = serde_json::json!({
                "recipientNpub": target,
                "grant": grant,
                "accessChangeEvent": event,
                "expiresAt": expires_at,
                "createPersonalMount": args.iter().any(|arg| arg == "--personal-mount")
            });
            let route = format!("/_admin/vaults/{vault_id}/folders/{folder_id}/share-links");
            let response = signed_json_request(env, args, "POST", &route, Some(body))?;
            write_command_response(output, json, &response)
        }
        Some("accept") => {
            let id = required_option_or_positional(args, "--id", 1, "share-link-id")?;
            let route = format!("/_admin/share-links/{id}/accept");
            let response = signed_json_request(env, args, "POST", &route, None)?;
            write_command_response(output, json, &response)
        }
        Some("revoke") => {
            let id = required_option_or_positional(args, "--id", 1, "share-link-id")?;
            let route = format!("/_admin/share-links/{id}");
            let response = signed_json_request(env, args, "DELETE", &route, None)?;
            write_command_response(output, json, &response)
        }
        Some("source") => {
            let vault_id = command_vault_id(args, env)?;
            let folder_id =
                option_value(args, "--folder").ok_or(CliError::MissingArgument("--folder"))?;
            let metadata = fetch_vault_metadata(env, args, &vault_id)?;
            let key_version = metadata
                .folders
                .iter()
                .find(|folder| folder.id == folder_id)
                .map(|folder| folder.current_key_version)
                .ok_or_else(|| CliError::NotFound(format!("folder {folder_id}")))?;
            let event = admin_access_change_event(
                env,
                &vault_id,
                AdminAccessAction::SetFolderAccessMode,
                Some(&folder_id),
                None,
                Some(key_version),
            )?;
            let route = format!("/_admin/vaults/{vault_id}/folders/{folder_id}/share-source");
            let response = signed_json_request(
                env,
                args,
                "POST",
                &route,
                Some(serde_json::json!({ "accessChangeEvent": event })),
            )?;
            write_command_response(output, json, &response)
        }
        Some("folder-invite") => {
            let vault_id = command_vault_id(args, env)?;
            let folder_id =
                option_value(args, "--folder").ok_or(CliError::MissingArgument("--folder"))?;
            let destination_vault_id = option_value(args, "--destination-vault")
                .ok_or(CliError::MissingArgument("--destination-vault"))?;
            let raw_destination_admin = option_value(args, "--destination-admin")
                .ok_or(CliError::MissingArgument("--destination-admin"))?;
            let destination_admin = resolve_identity_npub(env, args, &raw_destination_admin)?;
            let metadata = fetch_vault_metadata(env, args, &vault_id)?;
            let key_version = metadata
                .folders
                .iter()
                .find(|folder| folder.id == folder_id)
                .map(|folder| folder.current_key_version)
                .ok_or_else(|| CliError::NotFound(format!("folder {folder_id}")))?;
            let session_keys = open_vault_session_folder_keys(env, args, &vault_id)?;
            let folder_key = opened_folder_key(&session_keys, &vault_id, &folder_id, key_version)?;
            let auth = load_signer(env)?;
            let event = admin_access_change_event(
                env,
                &vault_id,
                AdminAccessAction::GrantFolderAccess,
                Some(&folder_id),
                Some(&destination_admin),
                Some(key_version),
            )?;
            let grant = folder_key_grant_request(
                &auth,
                &vault_id,
                &folder_id,
                key_version,
                &destination_admin,
                &folder_key,
                env,
            )?;
            let route =
                format!("/_admin/vaults/{vault_id}/folders/{folder_id}/shared-folder-invitations");
            let body = serde_json::json!({
                "destinationVaultId": destination_vault_id,
                "destinationAdminNpub": destination_admin,
                "grant": grant,
                "accessChangeEvent": event
            });
            let response = signed_json_request(env, args, "POST", &route, Some(body))?;
            write_command_response(output, json, &response)
        }
        Some("folder-accept") => {
            let id = required_option_or_positional(args, "--id", 1, "shared-folder-invitation-id")?;
            let route = format!("/_admin/shared-folder-invitations/{id}/accept");
            let response = signed_json_request(env, args, "POST", &route, None)?;
            write_command_response(output, json, &response)
        }
        Some(other) => Err(CliError::InvalidCommand(format!("share {other}"))),
        None => Err(CliError::MissingArgument("share command")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finite_brain_core::{EncryptedFolderObjectEnvelope, FolderObjectAad, open_folder_object};
    use finite_nostr::{
        GiftWrapValidation, NostrPublicKey, decode_http_auth_header, open_gift_wrap,
    };
    use nostr::{Event, Keys};
    use serde_json::Value;
    use std::io::{ErrorKind, Read};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    fn env_for(tmp: &TempDir) -> CliEnvironment {
        CliEnvironment {
            cwd: tmp.path().to_path_buf(),
            config_dir: tmp.path().join("config"),
            now: Some("2026-06-24T20:46:36Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(tmp.path().join("finite-home")),
        }
    }

    fn env_with_identity_authority(
        tmp: &TempDir,
        identity_authority_url: String,
    ) -> CliEnvironment {
        let mut env = env_for(tmp);
        env.identity_authority_url = Some(identity_authority_url);
        env
    }

    fn run(tmp: &TempDir, args: &[&str]) -> String {
        let mut output = Vec::new();
        run_with_env(args.iter().copied(), env_for(tmp), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    /// Plant a known secret as the shared Finite identity for this test
    /// environment (setup shorthand; the CLI import path has its own tests).
    fn import_identity_for(env: &CliEnvironment, secret: &str) {
        let paths = identity_paths(env).unwrap();
        finite_identity::FiniteIdentity::import(
            &paths,
            finite_identity::ImportSecret::parse(secret).unwrap(),
            "test-setup/0.0.0",
        )
        .unwrap();
    }

    fn import_identity_secret(tmp: &TempDir, secret: &str) {
        import_identity_for(&env_for(tmp), secret);
    }

    fn start_conflict_sync_server(
        export_grants: Vec<Value>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            while requests.len() < 3 && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, _) = read_http_request(&mut stream);
                requests.push(request_line.clone());
                let (status, body) = if request_line.contains("/export") {
                    (
                        "200 OK",
                        serde_json::json!({
                            "vault": {
                                "id": "vault",
                                "kind": "personal",
                                "name": "Vault",
                                "ownerUserId": null
                            },
                            "folders": [{
                                "id": "general",
                                "path": "General",
                                "access": "owner",
                                "currentKeyVersion": 1,
                                "sharedFolderSource": false,
                                "accessible": true
                            }],
                            "keyGrants": export_grants.clone(),
                            "accessState": {
                                "members": [],
                                "admins": []
                            }
                        })
                        .to_string(),
                    )
                } else if request_line.contains("/sync/bootstrap") {
                    (
                        "200 OK",
                        serde_json::json!({
                            "latestSequence": 0,
                            "objects": []
                        })
                        .to_string(),
                    )
                } else {
                    (
                        "409 Conflict",
                        serde_json::json!({
                            "error": "baseRevision does not match current folder object revision"
                        })
                        .to_string(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn start_empty_sync_server() -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            while requests.len() < 2 && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, _) = read_http_request(&mut stream);
                requests.push(request_line.clone());
                let body = if request_line.contains("/export") {
                    export_body(&[])
                } else if request_line.contains("/sync/records") {
                    serde_json::json!({
                        "vaultId": "vault",
                        "afterSequence": 0,
                        "latestSequence": 0,
                        "records": [],
                        "count": 0,
                        "hasMore": false,
                        "nextSequence": 0
                    })
                    .to_string()
                } else {
                    serde_json::json!({
                        "latestSequence": 0,
                        "objects": []
                    })
                    .to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn export_body(key_grants: &[Value]) -> String {
        serde_json::json!({
            "vault": {
                "id": "vault",
                "kind": "personal",
                "name": "Vault",
                "ownerUserId": null
            },
            "folders": [{
                "id": "general",
                "path": "General",
                "access": "owner",
                "currentKeyVersion": 1,
                "sharedFolderSource": false,
                "accessible": true
            }],
            "keyGrants": key_grants,
            "accessState": {
                "members": [],
                "admins": []
            }
        })
        .to_string()
    }

    fn start_incremental_remote_sync_server(
        ciphertext: String,
        export_grants: Vec<Value>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            while requests.len() < 2 && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, _) = read_http_request(&mut stream);
                requests.push(request_line.clone());
                let (status, body) = if request_line.contains("/export") {
                    ("200 OK", export_body(&export_grants))
                } else if request_line.contains("/sync/records") {
                    (
                        "200 OK",
                        serde_json::json!({
                            "vaultId": "vault",
                            "afterSequence": 0,
                            "latestSequence": 7,
                            "records": [{
                                "sequence": 7,
                                "recordEventId": "evt-remote-7",
                                "recordType": "folder_object_revision",
                                "folderId": "general",
                                "objectId": "obj_remote000001",
                                "revision": 1,
                                "actorNpub": "npub-remote",
                                "clientCreatedAt": "2026-06-24T20:46:36Z",
                                "payloadJson": remote_revision_payload_json(&ciphertext),
                                "recordEventKind": APP_SPECIFIC_KIND
                            }],
                            "count": 1,
                            "hasMore": false,
                            "nextSequence": 7
                        })
                        .to_string(),
                    )
                } else {
                    (
                        "404 Not Found",
                        serde_json::json!({ "error": "not found" }).to_string(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn start_expired_cursor_sync_server(
        ciphertext: String,
        export_grants: Vec<Value>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            while requests.len() < 3 && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, _) = read_http_request(&mut stream);
                requests.push(request_line.clone());
                let (status, body) = if request_line.contains("/export") {
                    ("200 OK", export_body(&export_grants))
                } else if request_line.contains("/sync/records") {
                    (
                        "410 Gone",
                        serde_json::json!({
                            "error": "rebootstrap required from retention floor 3"
                        })
                        .to_string(),
                    )
                } else if request_line.contains("/sync/bootstrap") {
                    (
                        "200 OK",
                        serde_json::json!({
                            "latestSequence": 5,
                            "objects": [{
                                "folderId": "general",
                                "objectId": "obj_remote000001",
                                "revision": 1,
                                "ciphertext": ciphertext,
                                "deleted": false
                            }]
                        })
                        .to_string(),
                    )
                } else {
                    (
                        "404 Not Found",
                        serde_json::json!({ "error": "not found" }).to_string(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn start_two_agent_incremental_sync_server(
        export_grants: Vec<Value>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            let mut accepted_object = None::<(String, String)>;
            while requests.len() < 5 && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, body) = read_http_request(&mut stream);
                requests.push(request_line.clone());
                let (status, response_body) = if request_line.contains("/export") {
                    ("200 OK", export_body(&export_grants))
                } else if request_line.starts_with("PUT ") {
                    let path = request_line.split_whitespace().nth(1).unwrap_or_default();
                    let object_id = path.rsplit('/').next().unwrap_or_default().to_owned();
                    let body: Value = serde_json::from_str(&body).unwrap();
                    let ciphertext = body["ciphertext"].as_str().unwrap().to_owned();
                    accepted_object = Some((object_id, ciphertext));
                    (
                        "200 OK",
                        serde_json::json!({
                            "sequence": 1,
                            "duplicate": false,
                            "revision": 1
                        })
                        .to_string(),
                    )
                } else if request_line.contains("/sync/bootstrap") {
                    let objects = accepted_object
                        .as_ref()
                        .map(|(object_id, ciphertext)| {
                            vec![serde_json::json!({
                                "folderId": "general",
                                "objectId": object_id,
                                "revision": 1,
                                "ciphertext": ciphertext,
                                "deleted": false
                            })]
                        })
                        .unwrap_or_default();
                    (
                        "200 OK",
                        serde_json::json!({
                            "latestSequence": objects.len() as u64,
                            "objects": objects
                        })
                        .to_string(),
                    )
                } else if request_line.contains("/sync/records") {
                    let records = accepted_object
                        .as_ref()
                        .map(|(object_id, ciphertext)| {
                            vec![serde_json::json!({
                                "sequence": 1,
                                "recordEventId": "evt-agent-b-1",
                                "recordType": "folder_object_revision",
                                "folderId": "general",
                                "objectId": object_id,
                                "revision": 1,
                                "actorNpub": "npub-agent-b",
                                "clientCreatedAt": "2026-06-24T20:46:36Z",
                                "payloadJson": remote_revision_payload_json_for_object(
                                    object_id,
                                    ciphertext,
                                ),
                                "recordEventKind": APP_SPECIFIC_KIND
                            })]
                        })
                        .unwrap_or_default();
                    (
                        "200 OK",
                        serde_json::json!({
                            "vaultId": "vault",
                            "afterSequence": 0,
                            "latestSequence": records.len() as u64,
                            "records": records,
                            "count": records.len(),
                            "hasMore": false,
                            "nextSequence": records.len() as u64
                        })
                        .to_string(),
                    )
                } else {
                    (
                        "404 Not Found",
                        serde_json::json!({ "error": "not found" }).to_string(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn remote_revision_payload_json(ciphertext: &str) -> String {
        remote_revision_payload_json_for_object("obj_remote000001", ciphertext)
    }

    fn remote_revision_payload_json_for_object(object_id: &str, ciphertext: &str) -> String {
        serde_json::json!({
            "recordType": "folder_object_revision",
            "folderId": "general",
            "objectId": object_id,
            "baseRevision": null,
            "keyVersion": 1,
            "cipher": CIPHER_AES_256_GCM,
            "ciphertext": ciphertext,
            "revisionEvent": {}
        })
        .to_string()
    }

    fn remote_page_ciphertext(folder_key: &FolderKey, page_path: &str, markdown: &str) -> String {
        let plaintext = encode_folder_object_page_plaintext(
            &SafeRelativePath::new("page_path", page_path).unwrap(),
            markdown,
        )
        .unwrap();
        let aad = FolderObjectAad {
            vault_id: VaultId::new("vault").unwrap(),
            folder_id: FolderId::new("general").unwrap(),
            object_id: ObjectId::new("obj_remote000001").unwrap(),
            key_version: 1,
        };
        encrypt_folder_object(folder_key, &aad, &plaintext)
            .unwrap()
            .canonical_json()
    }

    fn setup_incremental_tree(tmp: &TempDir, latest_sequence: u64) -> PathBuf {
        setup_incremental_tree_named(tmp, "vault", latest_sequence)
    }

    fn setup_incremental_tree_named(tmp: &TempDir, name: &str, latest_sequence: u64) -> PathBuf {
        let tree = tmp.path().join(name);
        fs::create_dir_all(tree.join(".finitebrain")).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        let now = "2026-06-24T20:46:36Z";
        write_agent_state(&tree, &AgentState::new("vault", now)).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &VaultWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_vault_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence },
            },
        )
        .unwrap();
        tree
    }

    fn start_metadata_export_and_grant_server(
        admin_npub: String,
        export_grants: Vec<Value>,
    ) -> (String, thread::JoinHandle<Vec<(String, String)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            while requests.len() < 3 && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, body) = read_http_request(&mut stream);
                let response_body = if request_line.contains("/metadata") {
                    serde_json::json!({
                        "vaultId": "acme",
                        "kind": "organization",
                        "name": "Acme",
                        "ownerUserId": null,
                        "members": [admin_npub],
                        "admins": [admin_npub],
                        "folders": [{
                            "id": "general",
                            "name": "general",
                            "role": "general",
                            "access": "all_members",
                            "parentFolderId": null,
                            "path": "general",
                            "sharedFolderSource": false,
                            "accessUserIds": [],
                            "currentKeyVersion": 1,
                            "setupIncomplete": false
                        }],
                        "mountedFolders": [],
                        "grantCount": 1
                    })
                    .to_string()
                } else if request_line.contains("/export") {
                    serde_json::json!({
                        "vault": {
                            "id": "acme",
                            "kind": "organization",
                            "name": "Acme",
                            "ownerUserId": null
                        },
                        "folders": [{
                            "id": "general",
                            "path": "general",
                            "access": "all_members",
                            "currentKeyVersion": 1,
                            "sharedFolderSource": false,
                            "accessible": true
                        }],
                        "keyGrants": export_grants,
                        "accessState": {
                            "members": [admin_npub],
                            "admins": [admin_npub]
                        }
                    })
                    .to_string()
                } else {
                    serde_json::json!({ "status": "ok" }).to_string()
                };
                requests.push((request_line, body));
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn start_session_key_sync_server(
        export_grant: Value,
        ciphertext: String,
        expected_requests: usize,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            while requests.len() < expected_requests && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, _) = read_http_request(&mut stream);
                let response_body = if request_line.contains("/export") {
                    serde_json::json!({
                        "vault": {
                            "id": "vault",
                            "kind": "personal",
                            "name": "Vault",
                            "ownerUserId": null
                        },
                        "folders": [{
                            "id": "general",
                            "path": "General",
                            "access": "owner",
                            "currentKeyVersion": 1,
                            "sharedFolderSource": false,
                            "accessible": true
                        }],
                        "keyGrants": [export_grant],
                        "accessState": {
                            "members": [],
                            "admins": []
                        }
                    })
                    .to_string()
                } else {
                    serde_json::json!({
                        "latestSequence": 1,
                        "objects": [{
                            "folderId": "general",
                            "objectId": "obj_remote000001",
                            "revision": 1,
                            "ciphertext": ciphertext,
                            "deleted": false
                        }]
                    })
                    .to_string()
                };
                requests.push(request_line);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn export_grant_for_test(
        env: &CliEnvironment,
        vault_id: &str,
        folder_id: &str,
        key_version: u32,
        folder_key: &FolderKey,
        recipient_npub: &str,
    ) -> Value {
        let auth = load_signer(env).unwrap();
        let grant = folder_key_grant_request(
            &auth,
            vault_id,
            folder_id,
            key_version,
            recipient_npub,
            folder_key,
            env,
        )
        .unwrap();
        serde_json::json!({
            "folderId": folder_id,
            "keyVersion": key_version,
            "issuerNpub": auth.npub,
            "recipientNpub": recipient_npub,
            "wrappedEventJson": grant["wrappedEventJson"]
        })
    }

    fn start_email_invite_server(
        admin_npub: String,
        export_grants: Vec<Value>,
    ) -> (String, thread::JoinHandle<Vec<(String, String)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            while requests.len() < 3 && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, body) = read_http_request(&mut stream);
                let response_body = if request_line.contains("/metadata") {
                    serde_json::json!({
                        "vaultId": "acme",
                        "kind": "organization",
                        "name": "Acme",
                        "ownerUserId": null,
                        "members": [admin_npub],
                        "admins": [admin_npub],
                        "folders": [
                            {
                                "id": "getting-started",
                                "name": "getting-started",
                                "role": "general",
                                "access": "all_members",
                                "parentFolderId": null,
                                "path": "getting-started",
                                "sharedFolderSource": false,
                                "accessUserIds": [],
                                "currentKeyVersion": 1,
                                "setupIncomplete": false
                            },
                            {
                                "id": "restricted",
                                "name": "restricted",
                                "role": "folder",
                                "access": "restricted",
                                "parentFolderId": null,
                                "path": "restricted",
                                "sharedFolderSource": false,
                                "accessUserIds": [],
                                "currentKeyVersion": 1,
                                "setupIncomplete": false
                            }
                        ],
                        "mountedFolders": [],
                        "grantCount": 2
                    })
                    .to_string()
                } else if request_line.contains("/export") {
                    serde_json::json!({
                        "vault": {
                            "id": "acme",
                            "kind": "organization",
                            "name": "Acme",
                            "ownerUserId": null
                        },
                        "folders": [
                            {
                                "id": "getting-started",
                                "path": "getting-started",
                                "access": "all_members",
                                "currentKeyVersion": 1,
                                "sharedFolderSource": false,
                                "accessible": true
                            },
                            {
                                "id": "restricted",
                                "path": "restricted",
                                "access": "restricted",
                                "currentKeyVersion": 1,
                                "sharedFolderSource": false,
                                "accessible": true
                            }
                        ],
                        "keyGrants": export_grants,
                        "accessState": {
                            "members": [admin_npub],
                            "admins": [admin_npub]
                        }
                    })
                    .to_string()
                } else {
                    serde_json::json!({
                        "id": "invitation-email",
                        "vaultId": "acme",
                        "targetKind": "email_bootstrap",
                        "userId": null,
                        "invitedEmail": "friend@example.com",
                        "inviteUnwrapNpub": null,
                        "bootstrapPayloadHash": null,
                        "bootstrapWrappedEventJson": null,
                        "bootstrapAuthorizationEventJson": null,
                        "bootstrapScope": [],
                        "claimedByNpub": null,
                        "identities": [],
                        "status": "pending",
                        "inviteCode": "invite-email",
                        "acceptPath": "/_admin/vault-invitation-links/invite-email/claim",
                        "initialFolderAccess": ["getting-started", "restricted"],
                        "expiresAt": "2026-06-30T00:00:00.000Z",
                        "createdAt": "2026-06-23T00:00:00.000Z",
                        "updatedAt": "2026-06-23T00:00:00.000Z",
                        "acceptedAt": null,
                        "duplicateAccept": false
                    })
                    .to_string()
                };
                requests.push((request_line, body));
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn start_metadata_listing_server(
        expected_requests: usize,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            while requests.len() < expected_requests && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, _) = read_http_request(&mut stream);
                requests.push(request_line);
                let body = serde_json::json!({
                    "vaultId": "acme",
                    "kind": "organization",
                    "name": "Acme",
                    "ownerUserId": null,
                    "members": ["npub-member"],
                    "admins": ["npub-admin"],
                    "folders": [{
                        "id": "general",
                        "name": "General",
                        "role": "general",
                        "access": "all_members",
                        "parentFolderId": null,
                        "path": "general",
                        "sharedFolderSource": true,
                        "accessUserIds": [],
                        "currentKeyVersion": 2,
                        "setupIncomplete": false
                    }],
                    "mountedFolders": [{
                        "mountId": "mount-1",
                        "organizationVaultId": "acme",
                        "sourceVaultId": "partner",
                        "sourceFolderId": "strategy",
                        "connectionId": "connection-1",
                        "displayName": "Partner Strategy",
                        "displayParentFolderId": null,
                        "state": "available"
                    }],
                    "grantCount": 3
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn start_ok_capture_server(
        expected_requests: usize,
    ) -> (String, thread::JoinHandle<Vec<(String, String)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            while requests.len() < expected_requests && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, body) = read_http_request(&mut stream);
                requests.push((request_line, body));
                let response_body = serde_json::json!({ "status": "ok" }).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn start_identity_authority_server(
        response_body: Value,
    ) -> (String, thread::JoinHandle<Vec<(String, String)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            while requests.is_empty() && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, body) = read_http_request(&mut stream);
                requests.push((request_line, body));
                let response_body = response_body.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn grant_plaintext_folder_key(body: &Value, secret: &str, recipient_npub: &str) -> String {
        let wrapped = body["grant"]["wrappedEventJson"].as_str().unwrap();
        let event = Event::from_json(wrapped).unwrap();
        let keys = Keys::parse(secret).unwrap();
        let recipient = NostrPublicKey::parse(recipient_npub).unwrap();
        let opened = open_gift_wrap(&keys, &event, &GiftWrapValidation::new(recipient)).unwrap();
        let plaintext: Value = serde_json::from_str(&opened.rumor.content).unwrap();
        plaintext["folderKey"].as_str().unwrap().to_owned()
    }

    fn open_default_page_request(
        body: &str,
        vault_id: &str,
        folder_id: &str,
        object_id: &str,
        folder_key: &str,
    ) -> Value {
        let body: Value = serde_json::from_str(body).unwrap();
        let key_version = body["keyVersion"].as_u64().unwrap() as u32;
        let key = FolderKey::from_base64(folder_key).unwrap();
        let aad = FolderObjectAad {
            vault_id: VaultId::new(vault_id).unwrap(),
            folder_id: finite_brain_core::FolderId::new(folder_id).unwrap(),
            object_id: ObjectId::new(object_id).unwrap(),
            key_version,
        };
        let envelope =
            EncryptedFolderObjectEnvelope::from_json(body["ciphertext"].as_str().unwrap()).unwrap();
        let plaintext = open_folder_object(&key, &aad, &envelope).unwrap();
        serde_json::from_slice(&plaintext).unwrap()
    }

    fn read_http_request(stream: &mut TcpStream) -> (String, String) {
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            if Instant::now() >= deadline {
                break;
            }
            let size = match stream.read(&mut buffer) {
                Ok(size) => size,
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    continue;
                }
                Err(_) => 0,
            };
            if size == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..size]);
            let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let body_start = header_end + 4;
            if bytes.len() >= body_start + content_length {
                let body = String::from_utf8_lossy(&bytes[body_start..body_start + content_length])
                    .to_string();
                let request_line = headers.lines().next().unwrap_or_default().to_owned();
                return (request_line, body);
            }
        }
        let request = String::from_utf8_lossy(&bytes).to_string();
        (
            request.lines().next().unwrap_or_default().to_owned(),
            String::new(),
        )
    }

    fn start_partial_success_sync_server(
        export_grants: Vec<Value>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            let mut write_count = 0_usize;
            let mut accepted_object = None::<(String, String)>;
            while requests.len() < 4 && started.elapsed() < Duration::from_secs(5) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, body) = read_http_request(&mut stream);
                requests.push(request_line.clone());
                let (status, response_body) = if request_line.contains("/export") {
                    (
                        "200 OK",
                        serde_json::json!({
                            "vault": {
                                "id": "vault",
                                "kind": "personal",
                                "name": "Vault",
                                "ownerUserId": null
                            },
                            "folders": [{
                                "id": "general",
                                "path": "General",
                                "access": "owner",
                                "currentKeyVersion": 1,
                                "sharedFolderSource": false,
                                "accessible": true
                            }],
                            "keyGrants": export_grants.clone(),
                            "accessState": {
                                "members": [],
                                "admins": []
                            }
                        })
                        .to_string(),
                    )
                } else if request_line.contains("/sync/bootstrap") {
                    let objects = accepted_object
                        .as_ref()
                        .map(|(object_id, ciphertext)| {
                            vec![serde_json::json!({
                                "folderId": "general",
                                "objectId": object_id,
                                "revision": 1,
                                "ciphertext": ciphertext,
                                "deleted": false
                            })]
                        })
                        .unwrap_or_default();
                    (
                        "200 OK",
                        serde_json::json!({
                            "latestSequence": objects.len() as u64,
                            "objects": objects
                        })
                        .to_string(),
                    )
                } else if request_line.starts_with("PUT ") {
                    write_count += 1;
                    if write_count == 1 {
                        let path = request_line.split_whitespace().nth(1).unwrap_or_default();
                        let object_id = path.rsplit('/').next().unwrap_or_default().to_owned();
                        let body: Value = serde_json::from_str(&body).unwrap();
                        let ciphertext = body["ciphertext"].as_str().unwrap().to_owned();
                        accepted_object = Some((object_id, ciphertext));
                        ("200 OK", serde_json::json!({ "status": "ok" }).to_string())
                    } else {
                        (
                            "409 Conflict",
                            serde_json::json!({
                                "error": "baseRevision does not match current folder object revision"
                            })
                            .to_string(),
                        )
                    }
                } else {
                    (
                        "404 Not Found",
                        serde_json::json!({ "error": "not found" }).to_string(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    const TEST_SECRET_HEX: &str =
        "0000000000000000000000000000000000000000000000000000000000000001";

    fn npub_for_secret(secret: &str) -> String {
        let keys = Keys::parse(secret).unwrap();
        NostrPublicKey::from_protocol(keys.public_key())
            .to_npub()
            .unwrap()
    }

    fn pubkey_hex_for_secret(secret: &str) -> String {
        Keys::parse(secret).unwrap().public_key().to_hex()
    }

    fn identity_file_for(tmp: &TempDir) -> std::path::PathBuf {
        identity_paths(&env_for(tmp)).unwrap().identity_file()
    }

    /// No file under fbrain's config dir may contain the secret; the shared
    /// identity file is the only place it lives.
    fn assert_config_dir_has_no_secret(tmp: &TempDir, secret: &str) {
        let config_dir = env_for(tmp).config_dir;
        assert!(!config_dir.join("auth.json").exists());
        if let Ok(entries) = fs::read_dir(&config_dir) {
            for entry in entries.flatten() {
                let body = fs::read_to_string(entry.path()).unwrap_or_default();
                assert!(
                    !body.contains(secret),
                    "{} leaks the secret",
                    entry.path().display()
                );
            }
        }
    }

    #[test]
    fn auth_status_reports_missing_identity_and_never_mints() {
        let tmp = TempDir::new().unwrap();
        let status = run(&tmp, &["auth", "status", "--json"]);
        let json: Value = serde_json::from_str(&status).unwrap();
        assert_eq!(json["state"], "missing");
        assert_eq!(json["signer"], "none");
        assert_eq!(json["npub"], Value::Null);
        assert_eq!(
            json["identityFile"],
            identity_file_for(&tmp).display().to_string()
        );
        // Status must never mint (finite-identity CLI-CONVENTIONS.md).
        assert!(!identity_file_for(&tmp).exists());
        run(&tmp, &["auth", "status"]);
        run(&tmp, &["status", "--json"]);
        run(&tmp, &["doctor"]);
        assert!(!identity_file_for(&tmp).exists());
    }

    #[test]
    fn auth_status_reports_shared_identity_fields() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(&tmp, TEST_SECRET_HEX);
        let status = run(&tmp, &["auth", "status", "--json"]);
        let json: Value = serde_json::from_str(&status).unwrap();
        assert_eq!(json["state"], "authenticated");
        assert_eq!(json["signer"], "finite-identity");
        assert_eq!(json["npub"], npub_for_secret(TEST_SECRET_HEX));
        assert_eq!(
            json["identityFile"],
            identity_file_for(&tmp).display().to_string()
        );
        assert_eq!(json["createdBy"], "test-setup/0.0.0");
        assert!(json["createdAt"].as_str().is_some());
        assert_eq!(
            json["configDir"],
            env_for(&tmp).config_dir.display().to_string()
        );
    }

    #[test]
    fn auth_import_reads_the_secret_from_stdin() {
        let tmp = TempDir::new().unwrap();
        let mut input = format!("{TEST_SECRET_HEX}\n").into_bytes();
        let mut input = std::io::Cursor::new(&mut input);
        let mut output = Vec::new();
        run_with_io(
            ["auth", "import", "--json"],
            env_for(&tmp),
            &mut input,
            &mut output,
        )
        .unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["npub"], npub_for_secret(TEST_SECRET_HEX));
        assert_eq!(
            json["identityFile"],
            identity_file_for(&tmp).display().to_string()
        );
        assert!(identity_file_for(&tmp).exists());
        assert_config_dir_has_no_secret(&tmp, TEST_SECRET_HEX);
    }

    #[test]
    fn auth_login_requests_identity_authority_challenge_without_minting() {
        let tmp = TempDir::new().unwrap();
        let (identity_authority_url, server) =
            start_identity_authority_server(serde_json::json!({ "email": "paul@finite.vip" }));
        let mut output = Vec::new();
        run_with_env(
            ["auth", "login", "paul@finite.vip", "--json"],
            env_with_identity_authority(&tmp, identity_authority_url),
            &mut output,
        )
        .unwrap();

        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["email"], "paul@finite.vip");
        assert!(!identity_file_for(&tmp).exists());

        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "POST /api/v1/email-challenges HTTP/1.1");
        let body: Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["email"], "paul@finite.vip");
    }

    #[test]
    fn auth_redeem_binds_finite_vip_email_through_identity_authority() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(&tmp, TEST_SECRET_HEX);
        let pubkey = pubkey_hex_for_secret(TEST_SECRET_HEX);
        let (identity_authority_url, server) = start_identity_authority_server(serde_json::json!({
            "email": "paul@finite.vip",
            "pubkey": pubkey,
            "nip05": "paul@finite.vip"
        }));
        let mut output = Vec::new();
        run_with_env(
            ["auth", "redeem", "paul@finite.vip", "token-123", "--json"],
            env_with_identity_authority(&tmp, identity_authority_url),
            &mut output,
        )
        .unwrap();

        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["email"], "paul@finite.vip");
        assert_eq!(json["pubkey"], pubkey_hex_for_secret(TEST_SECRET_HEX));
        assert_eq!(json["principalKind"], "native");
        assert_eq!(json["nip05"], "paul@finite.vip");
        assert_eq!(json["limitation"], Value::Null);

        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].0,
            "POST /api/v1/vip-email-bindings/redeem HTTP/1.1"
        );
        let body: Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["email"], "paul@finite.vip");
        assert_eq!(body["token"], "token-123");
    }

    #[test]
    fn auth_redeem_external_email_reports_email_only_brain_limitation() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(&tmp, TEST_SECRET_HEX);
        let pubkey = pubkey_hex_for_secret(TEST_SECRET_HEX);
        let (identity_authority_url, server) = start_identity_authority_server(serde_json::json!({
            "email": "friend@example.com",
            "pubkey": pubkey,
            "principal": {
                "kind": "email_only",
                "email": "friend@example.com",
                "pubkey": pubkey_hex_for_secret(TEST_SECRET_HEX)
            }
        }));
        let mut output = Vec::new();
        run_with_env(
            [
                "auth",
                "redeem",
                "friend@example.com",
                "token-456",
                "--json",
            ],
            env_with_identity_authority(&tmp, identity_authority_url),
            &mut output,
        )
        .unwrap();

        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["email"], "friend@example.com");
        assert_eq!(json["principalKind"], "email_only");
        assert_eq!(json["nip05"], Value::Null);
        assert!(json["limitation"].as_str().unwrap().contains("npub"));

        let requests = server.join().unwrap();
        assert_eq!(
            requests[0].0,
            "POST /api/v1/email-only-principals/redeem HTTP/1.1"
        );
        let body: Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["email"], "friend@example.com");
        assert_eq!(body["token"], "token-456");
    }

    #[test]
    fn auth_import_reads_the_secret_from_a_file_and_never_argv() {
        let tmp = TempDir::new().unwrap();
        let secret_path = tmp.path().join("secret.txt");
        fs::write(&secret_path, format!("{TEST_SECRET_HEX}\n")).unwrap();
        let output = run(
            &tmp,
            &["auth", "import", "--file", secret_path.to_str().unwrap()],
        );
        assert!(output.contains(&npub_for_secret(TEST_SECRET_HEX)));
        assert!(!output.contains(TEST_SECRET_HEX), "secret echoed back");
        assert!(identity_file_for(&tmp).exists());
        assert_config_dir_has_no_secret(&tmp, TEST_SECRET_HEX);

        // The prototype login verb is a hard cut, not a silent alias.
        let error = run_with_env(
            ["auth", "login", "--nsec", TEST_SECRET_HEX],
            env_for(&tmp),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("fbrain auth import"));
        let error = run_with_env(["auth", "logout"], env_for(&tmp), &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("shared"));
    }

    #[test]
    fn auth_import_refuses_to_overwrite_an_existing_identity() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(&tmp, TEST_SECRET_HEX);
        let other_secret = "0000000000000000000000000000000000000000000000000000000000000002";
        let secret_path = tmp.path().join("secret.txt");
        fs::write(&secret_path, other_secret).unwrap();
        let error = run_with_env(
            ["auth", "import", "--file", secret_path.to_str().unwrap()],
            env_for(&tmp),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        // The existing identity is untouched.
        let status = run(&tmp, &["auth", "status", "--json"]);
        let json: Value = serde_json::from_str(&status).unwrap();
        assert_eq!(json["npub"], npub_for_secret(TEST_SECRET_HEX));
    }

    #[test]
    fn commands_that_need_the_key_mint_the_shared_identity_on_first_use() {
        let tmp = TempDir::new().unwrap();
        assert!(!identity_file_for(&tmp).exists());
        let npub = run(&tmp, &["signer", "public-key"]).trim().to_owned();
        assert!(npub.starts_with("npub"));
        assert!(identity_file_for(&tmp).exists());
        // Repeat use keeps the same key.
        assert_eq!(run(&tmp, &["signer", "public-key"]).trim(), npub);
        // The mint recorded fbrain as the creating tool.
        let status = run(&tmp, &["auth", "status", "--json"]);
        let json: Value = serde_json::from_str(&status).unwrap();
        assert_eq!(
            json["createdBy"],
            format!("fbrain/{}", env!("CARGO_PKG_VERSION"))
        );
        assert_config_dir_has_no_secret(&tmp, TEST_SECRET_HEX);
    }

    #[test]
    fn existing_shared_identity_from_another_tool_is_found_not_replaced() {
        let tmp = TempDir::new().unwrap();
        // Another Finite tool minted first: plant a crate-format identity.
        let paths = identity_paths(&env_for(&tmp)).unwrap();
        let minted =
            finite_identity::FiniteIdentity::load_or_generate(&paths, "finitechat/0.0.0").unwrap();
        let npub = run(&tmp, &["signer", "public-key"]).trim().to_owned();
        assert_eq!(npub, minted.npub());
        let status = run(&tmp, &["auth", "status", "--json"]);
        let json: Value = serde_json::from_str(&status).unwrap();
        assert_eq!(json["createdBy"], "finitechat/0.0.0");
    }

    #[test]
    fn version_command_prints_cli_package_version() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            run(&tmp, &["--version"]).trim(),
            concat!("fbrain ", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            run(&tmp, &["version"]).trim(),
            concat!("fbrain ", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn global_config_dir_never_holds_the_identity() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(&tmp, TEST_SECRET_HEX);
        let config_dir = tmp.path().join("agent-config");

        // --config-dir redirects fbrain state, but the identity stays in the
        // shared location: no auth.json anywhere.
        let mut output = Vec::new();
        run_with_env(
            [
                "--config-dir",
                config_dir.to_str().unwrap(),
                "auth",
                "status",
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["state"], "authenticated");
        assert_eq!(json["configDir"], config_dir.display().to_string());
        assert_eq!(
            json["identityFile"],
            identity_file_for(&tmp).display().to_string()
        );
        assert!(!config_dir.join("auth.json").exists());
        assert!(!tmp.path().join("config/auth.json").exists());
    }

    #[test]
    fn open_creates_working_tree_and_status_json() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("agent-vault");
        let output = run(
            &tmp,
            &[
                "open",
                "agent-vault",
                tree.to_str().unwrap(),
                "--server",
                "http://127.0.0.1:3015",
                "--json",
            ],
        );
        let json: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(json["vaultId"], "agent-vault");
        assert_eq!(json["daemon"], "running");
        assert!(tree.join(".finitebrain/vault-directory.json").exists());
        assert!(tree.join(".finitebrain/working-tree-state.json").exists());
        assert!(tree.join(".finitebrain/agent-state.json").exists());

        let mut env = env_for(&tmp);
        env.cwd = tree;
        let mut output = Vec::new();
        run_with_env(["status", "--json"], env, &mut output).unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["vaultId"], "agent-vault");
        assert_eq!(json["auth"]["state"], "authenticated");
        assert_eq!(json["daemon"]["state"], "running");
        assert_eq!(json["sync"]["mode"], "automatic");
    }

    #[test]
    fn grant_folder_opens_session_key_without_persisting_it() {
        let tmp = TempDir::new().unwrap();
        let secret = "0000000000000000000000000000000000000000000000000000000000000001";
        import_identity_secret(&tmp, secret);
        let admin_npub = run(&tmp, &["signer", "public-key"]).trim().to_owned();
        let folder_key = FolderKey::from_bytes([7; 32]);
        let tree = tmp.path().join("org");
        fs::create_dir_all(tree.join(".finitebrain")).unwrap();
        write_agent_state(&tree, &AgentState::new("acme", "2026-06-24T20:46:36Z")).unwrap();

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let export_grant =
            export_grant_for_test(&env, "acme", "general", 1, &folder_key, &admin_npub);
        let (server_url, server) =
            start_metadata_export_and_grant_server(admin_npub.clone(), vec![export_grant]);
        let mut output = Vec::new();
        run_with_env(
            [
                "permissions",
                "grant-folder",
                "--vault",
                "acme",
                "--folder",
                "general",
                "--target",
                &admin_npub,
                "--server",
                &server_url,
                "--json",
            ],
            env,
            &mut output,
        )
        .unwrap();

        let requests = server.join().unwrap();
        let (_, body) = requests
            .iter()
            .find(|(request, _)| request.starts_with("POST "))
            .expect("grant request captured");
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            grant_plaintext_folder_key(&body, secret, &admin_npub),
            folder_key.to_base64()
        );
        let durable_state = fs::read_to_string(tree.join(".finitebrain/agent-state.json")).unwrap();
        assert!(!durable_state.contains(&folder_key.to_base64()));
    }

    #[test]
    fn sync_now_uses_session_key_without_persisting_it() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let actor_npub = run(&tmp, &["signer", "public-key"]).trim().to_owned();
        let folder_key = FolderKey::from_bytes([17; 32]);
        let tree = tmp.path().join("vault");
        fs::create_dir_all(tree.join(".finitebrain")).unwrap();
        write_agent_state(&tree, &AgentState::new("vault", "2026-06-24T20:46:36Z")).unwrap();
        let agent_state_path = tree.join(".finitebrain/agent-state.json");
        let mut durable_state: Value =
            serde_json::from_str(&fs::read_to_string(&agent_state_path).unwrap()).unwrap();
        durable_state
            .as_object_mut()
            .unwrap()
            .remove("unlockedFolders");
        durable_state
            .as_object_mut()
            .unwrap()
            .remove("localFolderKeys");
        write_json_file(&agent_state_path, &durable_state).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &VaultWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: Vec::new(),
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let export_grant =
            export_grant_for_test(&env, "vault", "general", 1, &folder_key, &actor_npub);
        let ciphertext =
            remote_page_ciphertext(&folder_key, "compiled/session.md", "# Session only\n");
        let (server_url, server) = start_session_key_sync_server(export_grant, ciphertext, 2);

        let mut output = Vec::new();
        run_with_env(
            ["sync", "now", "--server", &server_url, "--json"],
            env,
            &mut output,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(tree.join("General/compiled/session.md")).unwrap(),
            "# Session only\n"
        );
        let raw_key = folder_key.to_base64();
        for path in [
            ".finitebrain/agent-state.json",
            ".finitebrain/encrypted-sync/export.json",
            ".finitebrain/encrypted-sync/bootstrap.json",
        ] {
            let body = fs::read_to_string(tree.join(path)).unwrap();
            assert!(
                !body.contains(&raw_key),
                "raw Folder Key persisted in {path}"
            );
        }
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn sync_now_fails_closed_on_unusable_grant_for_local_member() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let actor_npub = run(&tmp, &["signer", "public-key"]).trim().to_owned();
        let folder_key = FolderKey::from_bytes([17; 32]);
        let tree = tmp.path().join("vault");
        fs::create_dir_all(tree.join(".finitebrain")).unwrap();
        write_agent_state(&tree, &AgentState::new("vault", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &VaultWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: Vec::new(),
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let mut export_grant =
            export_grant_for_test(&env, "vault", "general", 1, &folder_key, &actor_npub);
        export_grant["wrappedEventJson"] = Value::String("not-a-nostr-event".to_owned());
        let (server_url, server) = start_session_key_sync_server(export_grant, String::new(), 1);

        let error = run_with_env(
            ["sync", "now", "--server", &server_url, "--json"],
            env,
            &mut Vec::new(),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("encrypted Folder Key Grant"));
        assert!(error.contains("local signer"));
        assert!(!error.contains("not-a-nostr-event"));
        assert!(
            !fs::read_to_string(tree.join(".finitebrain/agent-state.json"))
                .unwrap()
                .contains(&folder_key.to_base64())
        );
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[test]
    fn vault_create_writes_default_pages() {
        let cases = [
            (
                "personal",
                VaultKind::Personal,
                "personal-defaults",
                "Personal defaults",
                vec!["getting-started", "restricted"],
            ),
            (
                "organization",
                VaultKind::Organization,
                "org-defaults",
                "Org defaults",
                vec!["getting-started", "restricted"],
            ),
        ];

        for (kind, vault_kind, vault_id, name, expected_grant_folders) in cases {
            let tmp = TempDir::new().unwrap();
            let secret = "0000000000000000000000000000000000000000000000000000000000000001";
            import_identity_secret(&tmp, secret);
            let actor_npub = run(&tmp, &["signer", "public-key"]).trim().to_owned();
            let default_pages = finite_brain_core::default_vault_pages(vault_kind);
            let (server_url, server) = start_ok_capture_server(1 + default_pages.len());

            let mut output = Vec::new();
            run_with_env(
                [
                    "vault",
                    "create",
                    vault_id,
                    "--kind",
                    kind,
                    "--name",
                    name,
                    "--server",
                    &server_url,
                    "--json",
                ],
                env_for(&tmp),
                &mut output,
            )
            .unwrap();

            let response: Value = serde_json::from_slice(&output).unwrap();
            assert_eq!(response["status"], "ok");

            let requests = server.join().unwrap();
            assert_eq!(requests.len(), 1 + default_pages.len());
            let (_, post_body) = requests
                .iter()
                .find(|(request, _)| request.starts_with("POST /_admin/vaults "))
                .expect("vault create request captured");
            let post_body: Value = serde_json::from_str(post_body).unwrap();
            assert_eq!(post_body["vaultId"], vault_id);
            assert_eq!(post_body["kind"], kind);
            assert_eq!(post_body["name"], name);
            assert_eq!(
                post_body["bootstrapGrants"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|grant| grant["folderId"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                expected_grant_folders
            );

            let folder_keys = post_body["bootstrapGrants"]
                .as_array()
                .unwrap()
                .iter()
                .map(|grant| {
                    let folder_id = grant["folderId"].as_str().unwrap().to_owned();
                    let folder_key = grant_plaintext_folder_key(grant, secret, &actor_npub);
                    (folder_id, folder_key)
                })
                .collect::<BTreeMap<_, _>>();

            for page in default_pages {
                let folder_key = folder_keys
                    .get(page.folder_id)
                    .expect("default Page Folder grant captured");
                let (request, body) = requests
                    .iter()
                    .find(|(request, _)| {
                        request.starts_with(&format!(
                            "PUT /_admin/vaults/{vault_id}/folders/{}/objects/{} ",
                            page.folder_id, page.object_id
                        ))
                    })
                    .expect("default Page write captured");
                assert!(request.contains(page.object_id));
                let plaintext = open_default_page_request(
                    body,
                    vault_id,
                    page.folder_id,
                    page.object_id,
                    folder_key,
                );
                assert_eq!(plaintext["version"], "finite-folder-object-page-v1");
                assert_eq!(plaintext["path"], page.path);
                assert!(plaintext["markdown"].as_str().unwrap().starts_with('#'));
            }
        }
    }

    #[test]
    fn folder_mount_and_access_list_commands_use_typed_metadata() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let (server_url, server) = start_metadata_listing_server(3);

        let mut output = Vec::new();
        run_with_env(
            [
                "folder",
                "list",
                "--vault",
                "acme",
                "--server",
                &server_url,
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();
        let folders: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(folders[0]["id"], "general");
        assert_eq!(folders[0]["sharedFolderSource"], true);

        let mut output = Vec::new();
        run_with_env(
            [
                "mount",
                "list",
                "--vault",
                "acme",
                "--server",
                &server_url,
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();
        let mounts: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(mounts[0]["mountId"], "mount-1");
        assert_eq!(mounts[0]["state"], "available");

        let mut output = Vec::new();
        run_with_env(
            [
                "access",
                "list",
                "--vault",
                "acme",
                "--server",
                &server_url,
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();
        let access: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(access["vaultId"], "acme");
        assert_eq!(access["grantCount"], 3);
        assert_eq!(access["folders"][0]["currentKeyVersion"], 2);
        assert_eq!(access["mountedFolders"][0]["sourceVaultId"], "partner");

        let requests = server.join().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.contains("/_admin/vaults/acme/metadata"))
                .count(),
            3
        );
    }

    #[test]
    fn access_revoke_blocks_without_rotation_material() {
        let tmp = TempDir::new().unwrap();
        let mut output = Vec::new();
        run_with_env(
            [
                "access",
                "revoke",
                "--vault",
                "acme",
                "--folder",
                "general",
                "--target",
                "npub-target",
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();

        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["state"], "blocked");
        assert_eq!(json["operation"], "remove-folder-access");
        assert_eq!(json["folderId"], "general");
        assert!(
            json["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str().unwrap().contains("reencryptedRecords"))
        );
    }

    #[test]
    fn access_revoke_with_rotation_body_uses_safe_delete_route() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let body_path = tmp.path().join("rotation-body.json");
        fs::write(
            &body_path,
            serde_json::json!({
                "newKeyVersion": 2,
                "grants": [{
                    "id": "grant-1",
                    "keyVersion": 2,
                    "recipientNpub": "npub-recipient",
                    "wrappedEventJson": "{}",
                    "createdAt": "2026-06-24T20:46:36Z"
                }],
                "reencryptedRecords": [],
                "accessChangeEvent": {}
            })
            .to_string(),
        )
        .unwrap();
        let (server_url, server) = start_ok_capture_server(1);

        let mut output = Vec::new();
        run_with_env(
            [
                "access",
                "revoke",
                "--vault",
                "acme",
                "--folder",
                "general",
                "--target",
                "npub-target",
                "--rotation-body",
                body_path.to_str().unwrap(),
                "--server",
                &server_url,
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["status"], "ok");

        let requests = server.join().unwrap();
        let (request, body) = requests.first().expect("delete request captured");
        assert!(
            request.starts_with("DELETE /_admin/vaults/acme/folders/general/access/npub-target")
        );
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["newKeyVersion"], 2);
        assert_eq!(body["grants"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn invites_create_posts_initial_folder_access() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let (server_url, server) = start_ok_capture_server(1);
        let target =
            npub_for_secret("0000000000000000000000000000000000000000000000000000000000000002");

        let mut output = Vec::new();
        run_with_env(
            [
                "invites",
                "create",
                "--vault",
                "acme",
                "--target",
                &target,
                "--folder",
                "general",
                "--server",
                &server_url,
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["status"], "ok");

        let requests = server.join().unwrap();
        let (request, body) = requests.first().expect("invite request captured");
        assert!(request.starts_with("POST /_admin/vaults/acme/invitations"));
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["targetNpub"], target);
        assert_eq!(body["initialFolderAccess"][0], "general");
    }

    #[test]
    fn invites_create_email_bootstrap_uses_one_session_keyring_without_persisting_keys() {
        let tmp = TempDir::new().unwrap();
        let admin_secret = "0000000000000000000000000000000000000000000000000000000000000001";
        import_identity_secret(&tmp, admin_secret);
        let admin_npub = run(&tmp, &["signer", "public-key"]).trim().to_owned();
        let getting_started_key = FolderKey::from_bytes([11; 32]);
        let restricted_key = FolderKey::from_bytes([12; 32]);
        let tree = tmp.path().join("org");
        fs::create_dir_all(tree.join(".finitebrain")).unwrap();
        write_agent_state(&tree, &AgentState::new("acme", "2026-06-24T20:46:36Z")).unwrap();

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let export_grants = vec![
            export_grant_for_test(
                &env,
                "acme",
                "getting-started",
                1,
                &getting_started_key,
                &admin_npub,
            ),
            export_grant_for_test(&env, "acme", "restricted", 1, &restricted_key, &admin_npub),
        ];
        let (server_url, server) = start_email_invite_server(admin_npub, export_grants);
        let mut output = Vec::new();
        run_with_env(
            [
                "invites",
                "create",
                "--vault",
                "acme",
                "--target",
                "friend@example.com",
                "--folder",
                "restricted",
                "--expires",
                "2026-06-30T00:00:00.000Z",
                "--server",
                &server_url,
                "--json",
            ],
            env,
            &mut output,
        )
        .unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        let invite_secret = json["inviteSecret"].as_str().unwrap();
        assert!(
            json["inviteUrl"]
                .as_str()
                .unwrap()
                .contains(&format!("#inviteSecret={invite_secret}"))
        );

        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].0.contains("/_admin/vaults/acme/metadata"));
        assert!(requests[1].0.contains("/_admin/vaults/acme/export"));
        let (request, body) = &requests[2];
        assert!(request.starts_with("POST /_admin/vaults/acme/invitations"));
        assert!(
            !body.contains(invite_secret),
            "server-visible request body must not contain invite secret"
        );
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["target"], "friend@example.com");
        assert!(body.get("targetNpub").is_none());
        assert_eq!(
            body["initialFolderAccess"],
            serde_json::json!(["restricted"])
        );
        assert!(
            body["bootstrapPayloadHash"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        let invite_unwrap_npub = body["inviteUnwrapNpub"].as_str().unwrap();
        let wrapped = body["bootstrapWrappedEventJson"].as_str().unwrap();
        let event = Event::from_json(wrapped).unwrap();
        let unwrap_keys = Keys::parse(invite_secret).unwrap();
        let recipient = NostrPublicKey::parse(invite_unwrap_npub).unwrap();
        let opened =
            open_gift_wrap(&unwrap_keys, &event, &GiftWrapValidation::new(recipient)).unwrap();
        let payload: Value = serde_json::from_str(&opened.rumor.content).unwrap();
        assert_eq!(payload["invitedEmail"], "friend@example.com");
        assert_eq!(
            payload["folders"]
                .as_array()
                .unwrap()
                .iter()
                .map(|folder| folder["folderId"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["getting-started", "restricted"]
        );
        let grants = payload["grants"].as_array().unwrap();
        assert_eq!(
            grant_plaintext_folder_key(&grants[0], invite_secret, invite_unwrap_npub),
            getting_started_key.to_base64()
        );
        assert_eq!(
            grant_plaintext_folder_key(&grants[1], invite_secret, invite_unwrap_npub),
            restricted_key.to_base64()
        );
        let durable_state = fs::read_to_string(tree.join(".finitebrain/agent-state.json")).unwrap();
        assert!(!durable_state.contains(&getting_started_key.to_base64()));
        assert!(!durable_state.contains(&restricted_key.to_base64()));
    }

    #[test]
    fn invites_code_commands_reject_invitation_ids_locally() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );

        let mut output = Vec::new();
        let error = run_with_env(
            [
                "invites",
                "accept",
                "--code",
                "invitation-4f82a37c1b82bcdd54973c466cdde914",
                "--server",
                "http://127.0.0.1:9",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap_err();

        assert!(matches!(error, CliError::InvalidInput(_)));
        assert!(
            error
                .to_string()
                .contains("expected an invite code like invite-")
        );
        assert!(error.to_string().contains("--id <invitation-id>"));
        assert!(output.is_empty());

        let error = run_with_env(
            [
                "invites",
                "show",
                "--code",
                "invitation-4f82a37c1b82bcdd54973c466cdde914",
                "--server",
                "http://127.0.0.1:9",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap_err();
        assert!(matches!(error, CliError::InvalidInput(_)));
    }

    #[test]
    fn invites_accept_routes_codes_and_ids_explicitly() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let (server_url, server) = start_ok_capture_server(2);

        let mut output = Vec::new();
        run_with_env(
            [
                "invites",
                "accept",
                "--code",
                "invite-0fe6eda60e1bf6e662acb8e2b5c425d9",
                "--server",
                &server_url,
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["status"], "ok");

        let mut output = Vec::new();
        run_with_env(
            [
                "invites",
                "accept",
                "--vault",
                "bavinck-org",
                "--id",
                "invitation-4f82a37c1b82bcdd54973c466cdde914",
                "--server",
                &server_url,
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();

        let requests = server.join().unwrap();
        assert!(requests[0].0.starts_with(
            "POST /_admin/vault-invitation-links/invite-0fe6eda60e1bf6e662acb8e2b5c425d9/accept"
        ));
        assert!(requests[1].0.starts_with(
            "POST /_admin/vaults/bavinck-org/invitations/invitation-4f82a37c1b82bcdd54973c466cdde914/accept"
        ));
    }

    #[test]
    fn share_link_uses_session_folder_key_without_persisting_it() {
        let tmp = TempDir::new().unwrap();
        let admin_secret = "0000000000000000000000000000000000000000000000000000000000000001";
        let sharee_secret = "0000000000000000000000000000000000000000000000000000000000000002";
        import_identity_secret(&tmp, admin_secret);
        let admin_npub = run(&tmp, &["signer", "public-key"]).trim().to_owned();
        let sharee_tmp = TempDir::new().unwrap();
        import_identity_secret(&sharee_tmp, sharee_secret);
        let sharee_npub = run(&sharee_tmp, &["signer", "public-key"])
            .trim()
            .to_owned();
        let folder_key = FolderKey::from_bytes([11; 32]);
        let tree = tmp.path().join("org");
        fs::create_dir_all(tree.join(".finitebrain")).unwrap();
        write_agent_state(&tree, &AgentState::new("acme", "2026-06-24T20:46:36Z")).unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let export_grant =
            export_grant_for_test(&env, "acme", "general", 1, &folder_key, &admin_npub);
        let (server_url, server) =
            start_metadata_export_and_grant_server(admin_npub, vec![export_grant]);
        let mut output = Vec::new();
        run_with_env(
            [
                "share",
                "link",
                "--vault",
                "acme",
                "--folder",
                "general",
                "--target",
                &sharee_npub,
                "--server",
                &server_url,
                "--json",
            ],
            env,
            &mut output,
        )
        .unwrap();

        let requests = server.join().unwrap();
        let (_, body) = requests
            .iter()
            .find(|(request, _)| request.starts_with("POST "))
            .expect("share link request captured");
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            grant_plaintext_folder_key(&body, sharee_secret, &sharee_npub),
            folder_key.to_base64()
        );
        let durable_state = fs::read_to_string(tree.join(".finitebrain/agent-state.json")).unwrap();
        assert!(!durable_state.contains(&folder_key.to_base64()));
    }

    #[test]
    fn share_folder_invite_uses_session_folder_key_without_persisting_it() {
        let tmp = TempDir::new().unwrap();
        let admin_secret = "0000000000000000000000000000000000000000000000000000000000000001";
        let destination_admin_secret =
            "0000000000000000000000000000000000000000000000000000000000000002";
        import_identity_secret(&tmp, admin_secret);
        let admin_npub = run(&tmp, &["signer", "public-key"]).trim().to_owned();
        let destination_tmp = TempDir::new().unwrap();
        import_identity_secret(&destination_tmp, destination_admin_secret);
        let destination_admin_npub = run(&destination_tmp, &["signer", "public-key"])
            .trim()
            .to_owned();
        let folder_key = FolderKey::from_bytes([13; 32]);
        let tree = tmp.path().join("org");
        fs::create_dir_all(tree.join(".finitebrain")).unwrap();
        write_agent_state(&tree, &AgentState::new("acme", "2026-06-24T20:46:36Z")).unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let export_grant =
            export_grant_for_test(&env, "acme", "general", 1, &folder_key, &admin_npub);
        let (server_url, server) =
            start_metadata_export_and_grant_server(admin_npub, vec![export_grant]);
        let mut output = Vec::new();
        run_with_env(
            [
                "share",
                "folder-invite",
                "--vault",
                "acme",
                "--folder",
                "general",
                "--destination-vault",
                "partner",
                "--destination-admin",
                &destination_admin_npub,
                "--server",
                &server_url,
                "--json",
            ],
            env,
            &mut output,
        )
        .unwrap();

        let requests = server.join().unwrap();
        let (_, body) = requests
            .iter()
            .find(|(request, _)| request.starts_with("POST "))
            .expect("folder invite request captured");
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            grant_plaintext_folder_key(&body, destination_admin_secret, &destination_admin_npub),
            folder_key.to_base64()
        );
        let durable_state = fs::read_to_string(tree.join(".finitebrain/agent-state.json")).unwrap();
        assert!(!durable_state.contains(&folder_key.to_base64()));
    }

    #[test]
    fn daemon_unlock_conflicts_activity_and_access_commands_use_agent_state() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("vault");
        run(&tmp, &["open", "vault", tree.to_str().unwrap()]);
        let roots = VaultWorkingTreeStateManifest {
            version: WORKING_TREE_STATE_VERSION.to_owned(),
            folder_roots: vec![
                WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_vault_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                },
                WorkingTreeFolderRoot {
                    folder_id: "locked".to_owned(),
                    source_vault_id: None,
                    path: "Locked".to_owned(),
                    can_read: false,
                    metadata_only: true,
                },
            ],
            objects: Vec::new(),
            sync: WorkingTreeSyncState { latest_sequence: 7 },
        };
        write_json_file(&tree.join(".finitebrain/working-tree-state.json"), &roots).unwrap();

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let mut output = Vec::new();
        run_with_env(["daemon", "stop"], env.clone(), &mut output).unwrap();
        assert!(String::from_utf8(output).unwrap().contains("stopped"));

        let mut output = Vec::new();
        run_with_env(["daemon", "start", "--json"], env.clone(), &mut output).unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["state"], "running");

        let mut output = Vec::new();
        run_with_env(["unlock", "--all", "--json"], env.clone(), &mut output).unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["opened"][0], "general");

        let mut state = read_agent_state(&tree).unwrap();
        state.conflicts.push(ConflictEntry {
            id: "conflict-1".to_owned(),
            folder_id: Some("general".to_owned()),
            path: Some("General/page.md".to_owned()),
            reason: "baseRevision does not match current folder object revision".to_owned(),
            state: ConflictState::Open,
            created_at: "2026-06-24T20:46:36Z".to_owned(),
            resolved_at: None,
        });
        write_agent_state(&tree, &state).unwrap();

        let mut output = Vec::new();
        run_with_env(["conflicts", "--json"], env.clone(), &mut output).unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json[0]["id"], "conflict-1");

        let mut output = Vec::new();
        run_with_env(["resolve", "conflict-1"], env.clone(), &mut output).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap().trim(),
            "resolved conflict-1"
        );

        let mut output = Vec::new();
        run_with_env(
            ["access", "explain", "Locked", "--json"],
            env.clone(),
            &mut output,
        )
        .unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["state"], "locked");

        let mut output = Vec::new();
        run_with_env(["activity"], env, &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("working_tree.opened"));
    }

    #[test]
    fn daemon_watch_once_runs_sync_and_stops_cleanly() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("vault");
        run(&tmp, &["open", "vault", tree.to_str().unwrap()]);
        let (server_url, server) = start_empty_sync_server();

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let mut output = Vec::new();
        run_with_env(
            [
                "daemon",
                "watch",
                "--once",
                "--server",
                &server_url,
                "--json",
            ],
            env.clone(),
            &mut output,
        )
        .unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["state"], "stopped");
        assert_eq!(json["ticks"], 1);
        assert_eq!(json["failures"], 0);
        assert_eq!(json["lastStatus"], "caught-up");

        let requests = server.join().unwrap();
        assert!(requests[0].contains("/_admin/vaults/vault/export"));
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/vaults/vault/sync/records?after=0"))
        );

        let state = read_agent_state(&tree).unwrap();
        assert_eq!(state.daemon.state, DaemonRunState::Stopped);
        assert_eq!(state.sync.status, "caught-up");
        assert!(
            state
                .activity
                .iter()
                .any(|entry| entry.kind == "daemon.watch.started")
        );
        assert!(
            state
                .activity
                .iter()
                .any(|entry| entry.kind == "daemon.watch.tick")
        );
        assert!(
            state
                .activity
                .iter()
                .any(|entry| entry.kind == "daemon.watch.stopped")
        );
    }

    #[test]
    fn daemon_watch_file_strategy_skips_idle_ticks() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("vault");
        run(&tmp, &["open", "vault", tree.to_str().unwrap()]);
        let (server_url, server) = start_empty_sync_server();

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let mut output = Vec::new();
        run_with_env(
            [
                "daemon",
                "watch",
                "--max-ticks",
                "2",
                "--poll-ms",
                "10",
                "--server",
                &server_url,
                "--json",
            ],
            env,
            &mut output,
        )
        .unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["state"], "stopped");
        assert_eq!(json["watchStrategy"], "working-tree-files");
        assert_eq!(json["ticks"], 2);
        assert_eq!(json["skippedTicks"], 1);
        assert_eq!(json["failures"], 0);

        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        let state = read_agent_state(&tree).unwrap();
        assert_eq!(state.daemon.tick_count, 2);
        assert_eq!(state.daemon.last_local_change_count, Some(0));
        assert!(
            state
                .activity
                .iter()
                .any(|entry| entry.kind == "daemon.watch.idle")
        );
    }

    #[test]
    fn daemon_watch_once_applies_incremental_remote_records() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let folder_key = FolderKey::from_bytes([8; 32]);
        let tree = setup_incremental_tree(&tmp, 0);
        let ciphertext = remote_page_ciphertext(&folder_key, "compiled/daemon.md", "# Daemon\n");

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let actor_npub = load_signer(&env).unwrap().npub;
        let export_grant =
            export_grant_for_test(&env, "vault", "general", 1, &folder_key, &actor_npub);
        let (server_url, server) =
            start_incremental_remote_sync_server(ciphertext, vec![export_grant]);
        let mut output = Vec::new();
        run_with_env(
            [
                "daemon",
                "watch",
                "--once",
                "--server",
                &server_url,
                "--json",
            ],
            env,
            &mut output,
        )
        .unwrap();

        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["state"], "stopped");
        assert_eq!(json["lastStatus"], "applied-remote-records");
        assert_eq!(
            fs::read_to_string(tree.join("General/compiled/daemon.md")).unwrap(),
            "# Daemon\n"
        );
        let state = read_agent_state(&tree).unwrap();
        assert_eq!(state.sync.status, "applied-remote-records");
        let tree_state = read_working_tree_state(&tree).unwrap();
        assert_eq!(tree_state.sync.latest_sequence, 7);

        let requests = server.join().unwrap();
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/vaults/vault/sync/records?after=0"))
        );
    }

    #[test]
    fn pending_working_tree_change_count_detects_local_markdown() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("vault");
        fs::create_dir_all(tree.join(".finitebrain")).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        write_agent_state(&tree, &AgentState::new("vault", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &VaultWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_vault_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        fs::write(tree.join("General/new.md"), "# New\n").unwrap();

        assert_eq!(pending_working_tree_change_count(&tree).unwrap(), 1);
    }

    #[test]
    fn daemon_watch_once_records_blocked_sync_without_crashing() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("vault");
        run(&tmp, &["open", "vault", tree.to_str().unwrap()]);

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let mut output = Vec::new();
        run_with_env(
            ["daemon", "watch", "--once", "--json"],
            env.clone(),
            &mut output,
        )
        .unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["state"], "stopped");
        assert_eq!(json["ticks"], 1);
        assert_eq!(json["failures"], 1);
        assert_eq!(json["watchStrategy"], "working-tree-files");
        assert_eq!(json["retryBackoffMillis"], 5000);
        assert!(!json["lastError"].as_str().unwrap().is_empty());

        let state = read_agent_state(&tree).unwrap();
        assert_eq!(state.daemon.state, DaemonRunState::Stopped);
        assert_eq!(state.daemon.failure_count, 1);
        assert_eq!(state.daemon.retry_backoff_millis, 5000);
        assert!(
            state
                .daemon
                .last_error
                .as_deref()
                .unwrap()
                .contains("server URL")
        );
        assert!(state.sync.status.contains("blocked:"));
        assert!(
            state
                .activity
                .iter()
                .any(|entry| entry.kind == "daemon.watch.blocked")
        );

        let mut output = Vec::new();
        run_with_env(["status", "--json"], env, &mut output).unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["daemon"]["failureCount"], 1);
        assert_eq!(json["daemon"]["retryBackoffMillis"], 5000);
        assert!(
            json["daemon"]["lastError"]
                .as_str()
                .unwrap()
                .contains("server URL")
        );
    }

    #[test]
    fn sync_now_applies_incremental_remote_records_and_reports_them() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let folder_key = FolderKey::from_bytes([4; 32]);
        let tree = setup_incremental_tree(&tmp, 0);
        let ciphertext = remote_page_ciphertext(&folder_key, "compiled/remote.md", "# Remote\n");

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let actor_npub = load_signer(&env).unwrap().npub;
        let export_grant =
            export_grant_for_test(&env, "vault", "general", 1, &folder_key, &actor_npub);
        let (server_url, server) =
            start_incremental_remote_sync_server(ciphertext, vec![export_grant]);
        let mut output = Vec::new();
        run_with_env(
            ["sync", "now", "--server", &server_url, "--json"],
            env,
            &mut output,
        )
        .unwrap();

        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["status"], "applied-remote-records");
        assert_eq!(json["latestSequence"], 7);
        assert_eq!(json["recordCount"], 1);
        assert_eq!(json["remoteChanges"].as_array().unwrap().len(), 1);
        assert_eq!(json["remoteChanges"][0]["status"], "applied");
        assert_eq!(json["remoteChanges"][0]["action"], "create");
        assert_eq!(json["remoteChanges"][0]["sequence"], 7);
        assert_eq!(json["remoteChanges"][0]["folderId"], "general");
        assert_eq!(json["remoteChanges"][0]["objectId"], "obj_remote000001");
        assert_eq!(
            json["remoteChanges"][0]["path"],
            "General/compiled/remote.md"
        );

        assert_eq!(
            fs::read_to_string(tree.join("General/compiled/remote.md")).unwrap(),
            "# Remote\n"
        );
        let tree_state = read_working_tree_state(&tree).unwrap();
        assert_eq!(tree_state.sync.latest_sequence, 7);
        assert_eq!(tree_state.objects.len(), 1);

        let requests = server.join().unwrap();
        assert!(requests[0].contains("/_admin/vaults/vault/export"));
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/vaults/vault/sync/records?after=0"))
        );
        assert!(
            !requests
                .iter()
                .any(|request| request.contains("/_admin/vaults/vault/sync/bootstrap"))
        );
    }

    #[test]
    fn sync_now_summary_prints_incremental_remote_records() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let folder_key = FolderKey::from_bytes([7; 32]);
        let tree = setup_incremental_tree(&tmp, 0);
        let ciphertext = remote_page_ciphertext(&folder_key, "compiled/summary.md", "# Summary\n");

        let mut env = env_for(&tmp);
        env.cwd = tree;
        let actor_npub = load_signer(&env).unwrap().npub;
        let export_grant =
            export_grant_for_test(&env, "vault", "general", 1, &folder_key, &actor_npub);
        let (server_url, server) =
            start_incremental_remote_sync_server(ciphertext, vec![export_grant]);
        let mut output = Vec::new();
        run_with_env(
            ["sync", "now", "--server", &server_url, "--summary"],
            env,
            &mut output,
        )
        .unwrap();

        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("applied-remote-records latestSequence=7"));
        assert!(text.contains("local changes: none"));
        assert!(text.contains("remote changes:"));
        assert!(text.contains("- applied create General/compiled/summary.md seq=7"));
        assert!(text.contains("conflicts: none"));

        let requests = server.join().unwrap();
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/vaults/vault/sync/records?after=0"))
        );
    }

    #[test]
    fn sync_now_rebootstraps_when_incremental_cursor_expired() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let folder_key = FolderKey::from_bytes([5; 32]);
        let tree = setup_incremental_tree(&tmp, 2);
        let ciphertext =
            remote_page_ciphertext(&folder_key, "compiled/rebootstrap.md", "# Bootstrap\n");

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let actor_npub = load_signer(&env).unwrap().npub;
        let export_grant =
            export_grant_for_test(&env, "vault", "general", 1, &folder_key, &actor_npub);
        let (server_url, server) = start_expired_cursor_sync_server(ciphertext, vec![export_grant]);
        let mut output = Vec::new();
        run_with_env(
            ["sync", "now", "--server", &server_url, "--json"],
            env,
            &mut output,
        )
        .unwrap();

        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["status"], "applied-remote-records");
        assert_eq!(json["latestSequence"], 5);
        assert_eq!(json["remoteChanges"].as_array().unwrap().len(), 0);
        assert_eq!(
            fs::read_to_string(tree.join("General/compiled/rebootstrap.md")).unwrap(),
            "# Bootstrap\n"
        );
        let tree_state = read_working_tree_state(&tree).unwrap();
        assert_eq!(tree_state.sync.latest_sequence, 5);

        let requests = server.join().unwrap();
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/vaults/vault/sync/records?after=2"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/vaults/vault/sync/bootstrap"))
        );
    }

    #[test]
    fn two_agent_sync_uses_incremental_records_for_second_working_tree() {
        let tmp = TempDir::new().unwrap();
        let nsec = "0000000000000000000000000000000000000000000000000000000000000001";
        let agent_a_config = tmp.path().join("agent-a-config");
        let agent_b_config = tmp.path().join("agent-b-config");
        let agent_a_auth_env = CliEnvironment {
            cwd: tmp.path().to_path_buf(),
            config_dir: agent_a_config.clone(),
            now: Some("2026-06-24T20:46:36Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(tmp.path().join("finite-home")),
        };
        // Both agents share the one Finite identity: import once.
        import_identity_for(&agent_a_auth_env, nsec);
        let folder_key = FolderKey::from_bytes([6; 32]);
        let agent_a_tree = setup_incremental_tree_named(&tmp, "agent-a", 0);
        let agent_b_tree = setup_incremental_tree_named(&tmp, "agent-b", 0);
        fs::write(agent_b_tree.join("General/shared.md"), "# Shared\n").unwrap();

        let env_b = CliEnvironment {
            cwd: agent_b_tree.clone(),
            config_dir: agent_b_config,
            now: Some("2026-06-24T20:46:36Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(tmp.path().join("finite-home")),
        };
        let actor_npub = load_signer(&env_b).unwrap().npub;
        let export_grant =
            export_grant_for_test(&env_b, "vault", "general", 1, &folder_key, &actor_npub);
        let (server_url, server) = start_two_agent_incremental_sync_server(vec![export_grant]);
        let mut output_b = Vec::new();
        run_with_env(
            ["sync", "now", "--server", &server_url, "--json"],
            env_b,
            &mut output_b,
        )
        .unwrap();
        let json_b: Value = serde_json::from_slice(&output_b).unwrap();
        assert_eq!(json_b["status"], "pushed-local-changes");
        assert_eq!(json_b["localChanges"].as_array().unwrap().len(), 1);

        let env_a = CliEnvironment {
            cwd: agent_a_tree.clone(),
            config_dir: agent_a_config,
            now: Some("2026-06-24T20:46:36Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(tmp.path().join("finite-home")),
        };
        let mut output_a = Vec::new();
        run_with_env(
            ["sync", "now", "--server", &server_url, "--json"],
            env_a,
            &mut output_a,
        )
        .unwrap();

        let json_a: Value = serde_json::from_slice(&output_a).unwrap();
        assert_eq!(json_a["status"], "applied-remote-records");
        assert_eq!(json_a["latestSequence"], 1);
        assert_eq!(json_a["remoteChanges"].as_array().unwrap().len(), 1);
        assert_eq!(json_a["remoteChanges"][0]["status"], "applied");
        assert_eq!(json_a["remoteChanges"][0]["sequence"], 1);
        assert_eq!(json_a["remoteChanges"][0]["path"], "General/shared.md");
        assert_eq!(
            fs::read_to_string(agent_a_tree.join("General/shared.md")).unwrap(),
            "# Shared\n"
        );
        let agent_a_state = read_working_tree_state(&agent_a_tree).unwrap();
        assert_eq!(agent_a_state.sync.latest_sequence, 1);

        let requests = server.join().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("PUT "))
                .count(),
            1
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/vaults/vault/sync/bootstrap"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/vaults/vault/sync/records?after=0"))
        );
    }

    #[test]
    fn sync_now_records_server_write_conflicts_through_public_command() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("vault");
        fs::create_dir_all(tree.join(".finitebrain")).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        fs::write(tree.join("General/new.md"), "# New\n").unwrap();
        let folder_key = FolderKey::from_bytes([9; 32]);

        let now = "2026-06-24T20:46:36Z";
        let mut state = AgentState::new("vault", now);
        state.server_url = Some("http://127.0.0.1:9".to_owned());
        state.daemon.state = DaemonRunState::Running;
        write_agent_state(&tree, &state).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &VaultWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_vault_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let actor_npub = load_signer(&env).unwrap().npub;
        let export_grant =
            export_grant_for_test(&env, "vault", "general", 1, &folder_key, &actor_npub);
        let (server_url, server) = start_conflict_sync_server(vec![export_grant]);
        let mut output = Vec::new();
        run_with_env(
            ["sync", "now", "--server", &server_url, "--json"],
            env,
            &mut output,
        )
        .unwrap();

        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["status"], "blocked-local-conflicts");
        assert_eq!(json["serverUrl"], server_url);
        assert_eq!(json["localChanges"].as_array().unwrap().len(), 1);
        assert_eq!(json["localChanges"][0]["status"], "conflicted");
        assert_eq!(json["localChanges"][0]["action"], "create");
        assert_eq!(json["localChanges"][0]["path"], "General/new.md");
        assert_eq!(json["conflicts"].as_array().unwrap().len(), 1);
        assert_eq!(json["remoteChanges"].as_array().unwrap().len(), 0);

        let requests = server.join().unwrap();
        assert!(requests[0].contains("/_admin/vaults/vault/export"));
        assert!(requests.iter().any(|request| {
            request.starts_with("PUT /_admin/vaults/vault/folders/general/objects/obj_")
        }));
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/vaults/vault/sync/bootstrap"))
        );

        let state = read_agent_state(&tree).unwrap();
        assert_eq!(state.conflicts.len(), 1);
        assert_eq!(state.conflicts[0].folder_id.as_deref(), Some("general"));
        assert_eq!(state.conflicts[0].path.as_deref(), Some("General/new.md"));
        assert_eq!(state.conflicts[0].state, ConflictState::Open);
        assert!(state.conflicts[0].reason.contains("409"));
    }

    #[test]
    fn sync_now_rematerializes_accepted_writes_while_preserving_conflicted_edits() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("vault");
        fs::create_dir_all(tree.join(".finitebrain")).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        fs::write(tree.join("General/a.md"), "# Accepted\n").unwrap();
        fs::write(tree.join("General/b.md"), "# Conflict\n").unwrap();
        let folder_key = FolderKey::from_bytes([9; 32]);

        let now = "2026-06-24T20:46:36Z";
        let mut state = AgentState::new("vault", now);
        state.server_url = Some("http://127.0.0.1:9".to_owned());
        state.daemon.state = DaemonRunState::Running;
        write_agent_state(&tree, &state).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &VaultWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_vault_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let actor_npub = load_signer(&env).unwrap().npub;
        let export_grant =
            export_grant_for_test(&env, "vault", "general", 1, &folder_key, &actor_npub);
        let (server_url, server) = start_partial_success_sync_server(vec![export_grant]);
        let mut output = Vec::new();
        run_with_env(
            ["sync", "now", "--server", &server_url, "--json"],
            env,
            &mut output,
        )
        .unwrap();

        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["status"], "blocked-local-conflicts");
        assert_eq!(json["localChanges"].as_array().unwrap().len(), 2);
        assert_eq!(json["localChanges"][0]["status"], "pushed");
        assert_eq!(json["localChanges"][0]["path"], "General/a.md");
        assert_eq!(json["localChanges"][1]["status"], "conflicted");
        assert_eq!(json["localChanges"][1]["path"], "General/b.md");
        assert_eq!(json["conflicts"].as_array().unwrap().len(), 1);
        let requests = server.join().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("PUT "))
                .count(),
            2
        );

        let tree_state = read_working_tree_state(&tree).unwrap();
        assert_eq!(tree_state.objects.len(), 1);
        assert_eq!(tree_state.objects[0].path, "a.md");
        assert_eq!(
            fs::read_to_string(tree.join("General/a.md")).unwrap(),
            "# Accepted\n"
        );
        assert_eq!(
            fs::read_to_string(tree.join("General/b.md")).unwrap(),
            "# Conflict\n"
        );
        let state = read_agent_state(&tree).unwrap();
        assert_eq!(state.conflicts.len(), 1);
        assert_eq!(state.conflicts[0].path.as_deref(), Some("General/b.md"));
    }

    #[test]
    fn sync_now_summary_prints_change_groups() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("vault");
        run(&tmp, &["open", "vault", tree.to_str().unwrap()]);
        let (server_url, server) = start_empty_sync_server();

        let mut env = env_for(&tmp);
        env.cwd = tree;
        let mut output = Vec::new();
        run_with_env(
            ["sync", "now", "--server", &server_url, "--summary"],
            env,
            &mut output,
        )
        .unwrap();

        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("caught-up latestSequence=0"));
        assert!(text.contains("local changes: none"));
        assert!(text.contains("remote changes: none"));
        assert!(text.contains("conflicts: none"));

        let requests = server.join().unwrap();
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/vaults/vault/sync/records?after=0"))
        );
    }

    #[test]
    fn doctor_reports_missing_working_tree_without_failing() {
        let tmp = TempDir::new().unwrap();
        let output = run(&tmp, &["doctor", "--json"]);
        let json: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(json["cli"]["state"], "ok");
        assert_eq!(json["auth"]["state"], "warn");
        assert_eq!(json["workingTree"]["state"], "warn");
    }

    #[test]
    fn signer_sign_encrypt_and_decrypt_behaves_like_local_nip07() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );

        let public_key = run(&tmp, &["signer", "public-key"]);
        assert!(public_key.trim().starts_with("npub"));

        let signed = run(
            &tmp,
            &[
                "signer",
                "sign",
                "--kind",
                "text",
                "--content",
                "hello finite",
                "--json",
            ],
        );
        let json: Value = serde_json::from_str(&signed).unwrap();
        assert_eq!(json["event"]["kind"], 1);
        assert_eq!(json["event"]["content"], "hello finite");

        let encrypted = run(
            &tmp,
            &[
                "signer",
                "encrypt",
                "--to",
                public_key.trim(),
                "--text",
                "folder secret",
                "--json",
            ],
        );
        let encrypted: Value = serde_json::from_str(&encrypted).unwrap();
        let decrypted = run(
            &tmp,
            &[
                "signer",
                "decrypt",
                "--from",
                public_key.trim(),
                "--payload",
                encrypted["ciphertext"].as_str().unwrap(),
                "--json",
            ],
        );
        let decrypted: Value = serde_json::from_str(&decrypted).unwrap();
        assert_eq!(decrypted["plaintext"], "folder secret");
    }

    #[test]
    fn signed_http_auth_header_validates_against_finite_nostr() {
        let keys = Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")
            .unwrap();
        let body = br#"{"vaultId":"agent"}"#;
        let url = "http://127.0.0.1:3015/_admin/vaults";
        let header = signed_http_auth_header(&keys, "POST", url, Some(body)).unwrap();
        let event = decode_http_auth_header(&header).unwrap();
        let expected = finite_nostr::HttpAuthValidation::new("POST", url, unix_timestamp(), 60)
            .with_body(body.to_vec());

        let signer = finite_nostr::validate_http_auth_event(&event, &expected).unwrap();

        assert_eq!(signer, NostrPublicKey::from_protocol(keys.public_key()));
    }

    #[test]
    fn server_url_selection_prefers_agent_transport_before_public_origin() {
        assert_eq!(
            select_server_url(
                Some("https://explicit.finite.test".to_owned()),
                Some("https://saved.finite.test".to_owned()),
                Some("https://server-env.finite.test".to_owned()),
                Some("https://public.finite.test".to_owned()),
            )
            .unwrap(),
            "https://explicit.finite.test"
        );
        assert_eq!(
            select_server_url(
                None,
                Some("https://saved.finite.test".to_owned()),
                Some("https://server-env.finite.test".to_owned()),
                Some("https://public.finite.test".to_owned()),
            )
            .unwrap(),
            "https://saved.finite.test"
        );
        assert_eq!(
            select_server_url(
                None,
                None,
                Some("https://server-env.finite.test".to_owned()),
                Some("https://public.finite.test".to_owned()),
            )
            .unwrap(),
            "https://server-env.finite.test"
        );
        assert_eq!(
            select_server_url(
                None,
                None,
                None,
                Some("https://public.finite.test".to_owned()),
            )
            .unwrap(),
            "https://public.finite.test"
        );
    }

    #[test]
    fn transport_url_validation_accepts_https_and_local_http() {
        assert!(validate_http_url("https://brain.smoke.finite.test").is_ok());
        assert!(validate_http_url("http://127.0.0.1:3015").is_ok());
        assert!(validate_http_url("http://[::1]:3015").is_ok());
        assert!(validate_http_url("http://localhost:3015").is_ok());
        assert!(validate_http_url("http://brain.smoke.finite.test").is_err());
        assert!(validate_http_url("ftp://brain.smoke.finite.test").is_err());
    }

    #[test]
    fn management_parser_uses_current_vault_not_target_positional() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("vault");
        run(&tmp, &["open", "agent-vault", tree.to_str().unwrap()]);

        let mut env = env_for(&tmp);
        env.cwd = tree;
        let args = vec!["add-member".to_owned(), "npub-target".to_owned()];

        assert_eq!(command_vault_id(&args, &env).unwrap(), "agent-vault");
    }

    #[test]
    fn folder_required_recipients_follow_access_mode() {
        let metadata = VaultMetadataView {
            vault_id: "org".to_owned(),
            kind: "organization".to_owned(),
            name: "Org".to_owned(),
            owner_user_id: None,
            members: vec!["npub-member".to_owned()],
            admins: vec!["npub-admin".to_owned()],
            folders: Vec::new(),
            mounted_folders: Vec::new(),
            grant_count: 0,
        };

        assert_eq!(
            folder_required_recipients(&metadata, "restricted", &["npub-member".to_owned()])
                .unwrap(),
            vec!["npub-admin".to_owned(), "npub-member".to_owned()]
        );
        assert_eq!(
            folder_required_recipients(&metadata, "admin_only", &[]).unwrap(),
            vec!["npub-admin".to_owned()]
        );

        let personal_metadata = VaultMetadataView {
            vault_id: "personal".to_owned(),
            kind: "personal".to_owned(),
            name: "Personal".to_owned(),
            owner_user_id: Some("npub-owner".to_owned()),
            members: Vec::new(),
            admins: Vec::new(),
            folders: Vec::new(),
            mounted_folders: Vec::new(),
            grant_count: 0,
        };
        assert_eq!(
            folder_required_recipients(&personal_metadata, "restricted", &[]).unwrap(),
            vec!["npub-owner".to_owned()]
        );
    }
}
