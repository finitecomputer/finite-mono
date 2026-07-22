//! Agent-native FiniteBrain CLI surface.

mod admin;
mod args;
mod clock;
mod embedding_provider;
mod environment;
mod error;
mod http;
mod identity_authority;
mod models;
mod output;
mod search;
mod semantic_index;
mod signer;
mod state;
mod sync_engine;
mod wiki;
mod working_tree_security;

pub use embedding_provider::{
    EmbeddingProviderAdapter, EmbeddingProviderConfig, EmbeddingProviderInput,
    EmbeddingProviderResponse, EmbeddingProviderVector,
};
pub use environment::CliEnvironment;
pub use error::CliError;
pub use models::{ActivityEntry, ConflictEntry, ConflictState};

pub(crate) use admin::*;
pub(crate) use args::*;
pub(crate) use clock::*;
pub(crate) use http::*;
pub(crate) use identity_authority::*;
pub(crate) use models::*;
pub(crate) use output::*;
pub(crate) use search::*;
pub(crate) use signer::*;
pub(crate) use state::*;
pub(crate) use sync_engine::*;
pub(crate) use wiki::*;
pub(crate) use working_tree_security::*;

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use finite_brain_core::portability::{
    BrainDirectoryBrainSummary, BrainDirectoryManifest, BrainDirectoryPath,
    BrainDirectoryPortability, BrainWorkingTreeStateManifest, WorkingTreeFolderRoot,
    WorkingTreeObjectManifestEntry, WorkingTreeSyncState,
};
use finite_brain_core::{
    AdminAccessAction, BrainId, FolderKey, bootstrap_organization_brain,
    bootstrap_organization_brain_with_requester, bootstrap_personal_brain,
};
use finite_nostr::{NostrPublicKey, build_rumor, decrypt_nip44, encrypt_nip44, wrap_rumor};
use nostr::{Keys, Kind};
use sha2::{Digest, Sha256};

pub(crate) const AGENT_STATE_VERSION: &str = "finitebrain-agent-state-v2";
pub(crate) const BRAIN_DIRECTORY_VERSION: &str = "finite-brain-directory-v1";
pub(crate) const WORKING_TREE_STATE_VERSION: &str = "finite-brain-working-tree-state-v1";
pub(crate) const APP_SPECIFIC_KIND: u16 = 30_078;
#[cfg(test)]
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
        "repair" => repair(&env, json, output),
        "auth" => auth(&args[1..], &env, json, input, output),
        "signer" => signer(&args[1..], &env, json, output),
        "daemon" => daemon(&args[1..], &env, json, output),
        "sync" => sync(&args[1..], &env, json, output),
        "open" => open_brain(&args[1..], &env, json, output),
        "status" => status(&env, json, output),
        "unlock" => unlock(&args[1..], &env, json, output),
        "conflicts" => conflicts(&env, json, output),
        "resolve" => resolve(&args[1..], &env, json, output),
        "search" => search(&args[1..], &env, json, output),
        "search-index" => search_index(&args[1..], &env, json, output),
        "activity" => activity(&env, json, output),
        "wiki" => wiki(&args[1..], &env, json, output),
        "access" => access(&args[1..], &env, json, output),
        "brain" => brain(&args[1..], &env, json, output),
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
        "fbrain [--config-dir <path>] doctor\nrepair\nauth status|import [--file <path>]|login <email>|redeem <email> <token>\nsigner status|public-key|sign|encrypt|decrypt\ndaemon status|start|stop|logs|tick|watch\nsync status|now [--summary]\nopen <brain-id> [path]\nstatus [--json]\nconflicts\nresolve <id>\nsearch <query> [--folder <folder>...] [--limit <1-50>] [--lexical-only] [--json]\nsearch-index status [--folder <folder>...]|enable --folder <folder>|disable --folder <folder> [--json]\nactivity\nwiki check\naccess explain|list|grant|revoke\nbrain list|create [--requesting-user-npub <npub|hex>]|bootstrap-personal|metadata|export\nfolder create|list|delete\nmount list\npermissions add-member|remove-member|add-admin|remove-admin|grant-folder --target <NIP-05|npub|hex>\ninvites create --target <NIP-05|npub|hex>|show --code invite-...|accept --code invite-...|accept --brain <brain-id> --id invitation-...|revoke\nshare link --target <NIP-05|npub|hex>|accept|revoke|source|folder-invite --destination-admin <NIP-05|npub|hex>|folder-accept"
    )?;
    Ok(())
}

fn version<W: Write>(output: &mut W) -> Result<(), CliError> {
    writeln!(output, "fbrain {}", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

fn wiki<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str).unwrap_or("check") {
        "check" => {
            let root = current_tree_root(env)?;
            let report = check_wiki_links(&root)?;
            if json {
                write_json(output, &report)
            } else {
                writeln!(
                    output,
                    "wiki links {}: {} pages, {} resolved, {} missing, {} ambiguous",
                    report.status,
                    report.page_count,
                    report.resolved_link_count,
                    report.missing_link_count,
                    report.ambiguous_link_count
                )?;
                for issue in report.issues {
                    writeln!(
                        output,
                        "- {} -> {} ({}){}",
                        issue.path,
                        issue.reference,
                        issue.status,
                        if issue.matches.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", issue.matches.join(", "))
                        }
                    )?;
                }
                Ok(())
            }
        }
        other => Err(CliError::InvalidCommand(format!("wiki {other}"))),
    }
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
    let (working_tree, working_tree_discovery_error) = match find_agent_state(&env.cwd) {
        Ok(working_tree) => (working_tree, None),
        Err(error) => (None, Some(error.to_string())),
    };
    let identity = load_identity_optional(env)?;
    let working_tree_boundary = working_tree
        .as_ref()
        .map(|root| validate_private_working_tree(root));
    let daemon_state = match working_tree_boundary.as_ref() {
        Some(Ok(())) => working_tree
            .as_ref()
            .and_then(|root| read_agent_state(root).ok())
            .map(|state| state.daemon.state)
            .unwrap_or(DaemonRunState::Missing),
        _ => DaemonRunState::Missing,
    };
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
        working_tree: match (
            working_tree.as_ref(),
            working_tree_boundary.as_ref(),
            working_tree_discovery_error.as_deref(),
        ) {
            (Some(root), Some(Ok(())), _) => {
                CheckState::ok(format!("Brain Working Tree at {}", root.display()))
            }
            (Some(_), Some(Err(error)), _) => CheckState::warn(error.to_string()),
            (_, _, Some(error)) => CheckState::warn(format!(
                "Brain Working Tree discovery failed: {error}; inspect the Working Tree boundary before continuing"
            )),
            _ => CheckState::warn("not inside a Brain Working Tree"),
        },
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

fn repair<W: Write>(env: &CliEnvironment, json: bool, output: &mut W) -> Result<(), CliError> {
    let root = find_agent_state(&env.cwd)?.ok_or(CliError::MissingWorkingTree)?;
    let report = repair_private_working_tree(&root)?;
    if json {
        write_json(output, &report)
    } else {
        writeln!(
            output,
            "repaired Brain Working Tree boundary at {} ({} directories, {} files)",
            report.working_tree_path, report.repaired_directories, report.repaired_files
        )?;
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

    // A cold daemon performs one complete reconciliation. Hot ticks below use
    // the exact Page paths returned by sync rather than rescanning every Folder.
    if let Err(error) = reconcile_search_indexes(&root) {
        let message = error.to_string();
        mutate_agent_state(env, |state, now| {
            state.add_activity(
                now,
                "search.index.blocked",
                format!("Cold search index reconciliation failed: {message}"),
            );
            Ok(())
        })?;
    }
    let mut semantic_worker = None;

    let mut ticks = 0_usize;
    let mut failures = 0_usize;
    let mut skipped_ticks = 0_usize;
    let mut consecutive_failures = 0_usize;
    let mut retry_backoff_millis: u64;
    let mut last_status = None::<String>;
    let mut last_error: Option<String>;
    loop {
        ticks += 1;
        let discovered_local_paths = pending_working_tree_change_paths(&root)?;
        let local_change_count = if file_aware {
            Some(discovered_local_paths.len())
        } else {
            None
        };
        let local_changes_due = local_change_count.unwrap_or_default() > 0;
        let remote_poll_due =
            remote_poll_ticks.is_some_and(|interval| ticks.is_multiple_of(interval));
        let should_sync = !file_aware || ticks == 1 || local_changes_due || remote_poll_due;

        if should_sync {
            match sync_once_with_local_paths(
                env,
                args,
                "daemon.watch.tick",
                Some(discovered_local_paths),
            ) {
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
                    "No local Brain Working Tree changes detected",
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
                        "Detected {count} pending Brain Working Tree change(s)",
                        count = local_change_count.unwrap_or_default()
                    ),
                );
            }
            Ok(())
        })?;

        if semantic_worker.is_none() {
            semantic_worker = env.embedding_provider.clone().map(|provider| {
                let root = root.clone();
                std::thread::spawn(move || refresh_semantic_indexes(&root, &provider))
            });
        }
        let should_stop = max_ticks.is_some_and(|limit| ticks >= limit);
        if semantic_worker
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            if let Some(worker) = semantic_worker.take() {
                record_semantic_worker_result(env, worker.join());
            }
            if !should_stop {
                semantic_worker = env.embedding_provider.clone().map(|provider| {
                    let root = root.clone();
                    std::thread::spawn(move || refresh_semantic_indexes(&root, &provider))
                });
            }
        }
        if should_stop {
            break;
        }
        std::thread::sleep(poll + std::time::Duration::from_millis(retry_backoff_millis));
    }

    // Bounded/one-shot watches wait for the already-backgrounded refresh only
    // after sync work has completed, giving deterministic CLI acceptance tests.
    if let Some(worker) = semantic_worker.take() {
        record_semantic_worker_result(env, worker.join());
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

fn record_semantic_worker_result(
    env: &CliEnvironment,
    result: std::thread::Result<Result<SemanticRefreshReport, CliError>>,
) {
    let (kind, detail) = match result {
        Ok(Ok(report)) => (
            "search.semantic.refresh",
            format!(
                "Semantic refresh selected={} rebuilt={} failed={}",
                report.selected_folders, report.rebuilt_folders, report.failed_folders
            ),
        ),
        Ok(Err(_)) => (
            "search.semantic.refresh_failed",
            "Semantic refresh failed before Folder isolation".to_owned(),
        ),
        Err(_) => (
            "search.semantic.refresh_failed",
            "Semantic refresh worker stopped unexpectedly".to_owned(),
        ),
    };
    let _ = mutate_agent_state(env, |state, now| {
        state.add_activity(now, kind, detail);
        Ok(())
    });
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
        let actor = change
            .actor_npub
            .as_deref()
            .map(|actor| format!(" actor={actor}"))
            .unwrap_or_default();
        if let Some(from_path) = change.from_path.as_deref() {
            writeln!(
                output,
                "- {} {} {} -> {}{}{}",
                change.status, change.action, from_path, path, sequence, actor
            )?;
        } else {
            writeln!(
                output,
                "- {} {} {}{}{}",
                change.status, change.action, path, sequence, actor
            )?;
        }
        if let Some(reason) = change.reason.as_deref() {
            writeln!(output, "  reason: {reason}")?;
        }
    }
    Ok(())
}

fn open_brain<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let brain_id = BrainId::new(
        args.first()
            .ok_or(CliError::MissingArgument("brain-id"))?
            .to_owned(),
    )
    .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let path = positional_values(args)
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env.working_tree_root
                .as_ref()
                .unwrap_or(&env.cwd)
                .join(brain_id.as_str())
        });
    let server_url = configured_server_url_for_open(args);
    if let Some(server_url) = server_url.as_deref() {
        validate_http_url(server_url)?;
    }
    let agent_state_path = path.join(".finitebrain/agent-state.json");
    let reopening = path_entry_exists(&agent_state_path)?;
    let portable_manifest_exists =
        path_entry_exists(&path.join(".finitebrain/brain-directory.json"))?
            || path_entry_exists(&path.join(".finitebrain/working-tree-state.json"))?;
    if !reopening && portable_manifest_exists {
        return Err(CliError::InvalidInput(
            "Working Tree has portable manifests but no fbrain Agent State; preserve it and use an explicit adoption or recovery flow"
                .to_owned(),
        ));
    }
    initialize_private_working_tree(&path)?;
    let now = timestamp(env);
    // Opening a Brain Working Tree needs the acting identity for signed sync.
    // The server projection supplies the actual Personal Brain owner; the
    // acting identity may instead be a limited Agent Principal.
    let auth = load_signer(env)?;
    let mut state = if reopening {
        let mut state = read_agent_state(&path)?;
        if state.brain_id != brain_id.as_str() {
            return Err(CliError::InvalidInput(format!(
                "Working Tree is already bound to Brain {}",
                state.brain_id
            )));
        }
        if let Some(existing_npub) = state.auth_npub.as_deref()
            && existing_npub != auth.npub
        {
            return Err(CliError::InvalidSigner(
                "Working Tree is bound to a different Member Identity".to_owned(),
            ));
        }
        if server_url.is_some() {
            state.server_url = server_url.clone();
        }
        state
    } else {
        let directory = BrainDirectoryManifest {
            version: BRAIN_DIRECTORY_VERSION.to_owned(),
            brain: BrainDirectoryBrainSummary {
                id: brain_id.to_string(),
                kind: "unknown".to_owned(),
                name: brain_id.to_string(),
                owner_npub: None,
            },
            working_tree: BrainDirectoryPath {
                path: ".".to_owned(),
            },
            encrypted_sync: BrainDirectoryPath {
                path: ".finitebrain/encrypted-sync".to_owned(),
            },
            portability: BrainDirectoryPortability {
                owned_by_agent_runtime: true,
                owned_by_app_surface: false,
            },
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let tree_state = BrainWorkingTreeStateManifest {
            version: WORKING_TREE_STATE_VERSION.to_owned(),
            folder_roots: Vec::<WorkingTreeFolderRoot>::new(),
            objects: Vec::<WorkingTreeObjectManifestEntry>::new(),
            sync: WorkingTreeSyncState { latest_sequence: 0 },
        };
        write_json_file(&path.join(".finitebrain/brain-directory.json"), &directory)?;
        write_working_tree_state(&path, &tree_state)?;
        let mut state = AgentState::new(brain_id.as_str(), &now);
        state.server_url = server_url.clone();
        state
    };
    state.daemon.state = DaemonRunState::Running;
    state.daemon.last_started_at = Some(now.clone());
    state.auth_npub = Some(auth.npub);
    state.add_activity(
        now.clone(),
        if reopening {
            "working_tree.reopened"
        } else {
            "working_tree.opened"
        },
        if reopening {
            "Existing Brain Working Tree reopened without resetting sync state"
        } else {
            "Brain Working Tree opened for agent use"
        },
    );
    write_agent_state(&path, &state)?;
    let mut opened_env = env.clone();
    opened_env.cwd = path.clone();
    let sync_status = match sync_once(
        &opened_env,
        args,
        if reopening {
            "working_tree.reopened.sync"
        } else {
            "working_tree.opened.sync"
        },
    ) {
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
                "brainId": brain_id.as_str(),
                "path": path,
                "daemon": "running",
                "syncMode": "automatic",
                "syncStatus": sync_status,
                "plaintextPersistence": "member-authored files persist until the Working Tree is explicitly removed"
            }),
        )
    } else {
        writeln!(output, "opened Brain Working Tree {}", path.display())?;
        writeln!(
            output,
            "member-authored plaintext persists until this Working Tree is explicitly removed"
        )?;
        Ok(())
    }
}

fn path_entry_exists(path: &Path) -> Result<bool, CliError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn status<W: Write>(env: &CliEnvironment, json: bool, output: &mut W) -> Result<(), CliError> {
    let report = status_report(env)?;
    if json {
        write_json(output, &report)
    } else {
        writeln!(
            output,
            "Brain: {}",
            report.brain_id.as_deref().unwrap_or("-")
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
        writeln!(output, "Conflicts: {}", report.conflicts.len())?;
        Ok(())
    }
}

fn unlock<W: Write>(
    _args: &[String],
    env: &CliEnvironment,
    _json: bool,
    _output: &mut W,
) -> Result<(), CliError> {
    if let Some(root) = find_agent_state(&env.cwd)? {
        read_agent_state(&root)?;
    }
    Err(CliError::Unsupported(
        "fbrain unlock was removed; run `fbrain sync now` to reopen encrypted Folder Key Grants for that operation"
            .to_owned(),
    ))
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

struct BrainCreatePlan {
    requesting_user_npub: Option<String>,
}

fn plan_brain_create(
    env: &CliEnvironment,
    brain_id: &str,
    kind: &str,
    name: &str,
    requesting_user_input: Option<&str>,
) -> Result<BrainCreatePlan, CliError> {
    let auth = load_signer(env)?;
    let requesting_user_npub = requesting_user_input
        .map(|input| {
            NostrPublicKey::parse(input)
                .and_then(|public_key| public_key.to_npub())
                .map_err(|error| {
                    CliError::InvalidInput(format!(
                        "invalid Organization Brain requester identity: {error}"
                    ))
                })
        })
        .transpose()?;
    let output = match kind {
        "personal" if requesting_user_npub.is_some() => {
            return Err(CliError::InvalidInput(
                "Organization Brain requester identity is only valid for an Organization Brain"
                    .to_owned(),
            ));
        }
        "personal" => bootstrap_personal_brain(brain_id, name, auth.npub.clone()),
        "organization" => {
            if let Some(requester) = requesting_user_npub.as_ref() {
                bootstrap_organization_brain_with_requester(
                    brain_id,
                    name,
                    auth.npub.clone(),
                    requester.clone(),
                )
            } else {
                bootstrap_organization_brain(brain_id, name, auth.npub.clone())
            }
        }
        other => {
            return Err(CliError::InvalidInput(format!(
                "unknown brain kind {other}"
            )));
        }
    };
    output.map_err(|error| CliError::InvalidInput(error.to_string()))?;

    Ok(BrainCreatePlan {
        requesting_user_npub,
    })
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
            let metadata = fetch_brain_metadata_for_command(env, args)?;
            let report = access_summary_report(metadata)?;
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
    let brain_id = command_brain_id(args, env)?;
    let folder_id = option_value(args, "--folder").ok_or(CliError::MissingArgument("--folder"))?;
    let target = required_option_or_positional(args, "--target", 1, "target-npub")?;
    let route = format!("/_admin/brains/{brain_id}/folders/{folder_id}/access/{target}");
    let Some(body) = access_rotation_body(args)? else {
        let report = AccessRemovalBlockedReport {
            state: "blocked".to_owned(),
            operation: "remove-folder-access".to_owned(),
            brain_id,
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
            "blocked remove-folder-access brain={} folder={} target={}",
            report.brain_id, report.folder_id, report.target_npub
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

fn fetch_brain_metadata_for_command(
    env: &CliEnvironment,
    args: &[String],
) -> Result<BrainMetadataView, CliError> {
    let brain_id = command_brain_id(args, env)?;
    fetch_brain_metadata(env, args, &brain_id)
}

fn access_summary_report(metadata: BrainMetadataView) -> Result<AccessSummaryReport, CliError> {
    let folders = metadata
        .folders
        .iter()
        .map(|folder| {
            Ok(FolderAccessSummary {
                id: folder.id.clone(),
                name: folder.name.clone(),
                role: folder.role.clone(),
                access: folder.access.clone(),
                parent_folder_id: folder.parent_folder_id.clone(),
                path: folder.path.clone(),
                shared_folder_source: folder.shared_folder_source,
                current_key_version: folder.current_key_version,
                setup_incomplete: folder.setup_incomplete,
                explicit_access_user_ids: folder.access_user_ids.clone(),
                effective_access_user_ids: folder_required_recipients(
                    &metadata,
                    &folder.access,
                    &folder.access_user_ids,
                )?,
            })
        })
        .collect::<Result<Vec<_>, CliError>>()?;
    Ok(AccessSummaryReport {
        brain_id: metadata.brain_id,
        members: metadata.members,
        admins: metadata.admins,
        folders,
        mounted_folders: metadata.mounted_folders,
        grant_count: metadata.grant_count,
    })
}

fn write_access_summary_rows<W: Write>(
    output: &mut W,
    report: &AccessSummaryReport,
) -> Result<(), CliError> {
    writeln!(
        output,
        "brain {} admins={} members={} grants={}",
        report.brain_id,
        report.admins.len(),
        report.members.len(),
        report.grant_count
    )?;
    if report.folders.is_empty() {
        writeln!(output, "no folders")?;
    } else {
        for folder in &report.folders {
            let access_details = format!(
                " explicitAccessUserIds=[{}] effectiveAccessUserIds=[{}]",
                folder.explicit_access_user_ids.join(","),
                folder.effective_access_user_ids.join(",")
            );
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
                "folder {} path={} access={} keyVersion={} state={}{}{}",
                folder.id,
                folder.path,
                folder.access,
                folder.current_key_version,
                setup,
                source,
                access_details
            )?;
        }
    }
    write_mount_rows(output, &report.mounted_folders)
}

fn brain<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    match args.first().map(String::as_str).unwrap_or("metadata") {
        "list" | "ls" => {
            let response = signed_json_request(env, args, "GET", "/_admin/brains", None)?;
            write_command_response(output, json, &response)
        }
        "bootstrap-personal" => {
            let response = signed_json_request(
                env,
                args,
                "POST",
                "/_admin/personal-brain-bootstrap",
                Some(serde_json::json!({})),
            )?;
            write_command_response(output, json, &response)
        }
        "create" => {
            let values = positional_values(args);
            let brain_id = values.get(1).ok_or(CliError::MissingArgument("brain-id"))?;
            let kind = option_value(args, "--kind").unwrap_or_else(|| "personal".to_owned());
            let normalized_kind = normalize_brain_kind(&kind)?;
            let name = option_value(args, "--name").unwrap_or_else(|| brain_id.clone());
            let requesting_user_input = unique_option_value(args, "--requesting-user-npub")?;
            let create_plan = plan_brain_create(
                env,
                brain_id,
                normalized_kind,
                &name,
                requesting_user_input.as_deref(),
            )?;
            let mut body = serde_json::json!({
                "brainId": brain_id,
                "kind": normalized_kind,
                "name": name,
                "bootstrapGrants": []
            });
            if let Some(requester) = create_plan.requesting_user_npub.as_ref() {
                body["requestingUserNpub"] = serde_json::Value::String(requester.clone());
            }
            let server_url = server_url_for_command(env, args)?;
            let response = signed_json_request_to_server(
                env,
                &server_url,
                "POST",
                "/_admin/brains",
                Some(body),
            )?;
            write_command_response(output, json, &response)
        }
        "metadata" | "status" => {
            let explicit_brain_id =
                option_value(args, "--brain").or_else(|| positional_values(args).get(1).cloned());
            let brain_id = match explicit_brain_id {
                Some(brain_id) => brain_id,
                None => current_brain_id(env)?
                    .ok_or(CliError::MissingArgument("brain-id or --brain"))?,
            };
            let path = format!("/_admin/brains/{brain_id}/metadata");
            let response = signed_json_request(env, args, "GET", &path, None)?;
            write_command_response(output, json, &response)
        }
        "export" => {
            let explicit_brain_id =
                option_value(args, "--brain").or_else(|| positional_values(args).get(1).cloned());
            let brain_id = match explicit_brain_id {
                Some(brain_id) => brain_id,
                None => current_brain_id(env)?
                    .ok_or(CliError::MissingArgument("brain-id or --brain"))?,
            };
            let path = format!("/_admin/brains/{brain_id}/export");
            let response = signed_json_request(env, args, "GET", &path, None)?;
            write_command_response(output, json, &response)
        }
        other => Err(CliError::InvalidCommand(format!("brain {other}"))),
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
            let metadata = fetch_brain_metadata_for_command(env, args)?;
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
            let brain_id = command_brain_id(args, env)?;
            let name = option_value(args, "--name").unwrap_or_else(|| folder_id.clone());
            let path = option_value(args, "--path").unwrap_or_else(|| name.clone());
            let role = option_value(args, "--role").unwrap_or_else(|| "folder".to_owned());
            let metadata = fetch_brain_metadata(env, args, &brain_id)?;
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
                &brain_id,
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
                        &brain_id,
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
            let route = format!("/_admin/brains/{brain_id}/folders");
            let response = signed_json_request(env, args, "POST", &route, Some(body))?;
            update_local_folder_after_create(env, folder_id, &path)?;
            write_command_response(output, json, &response)
        }
        "delete" => {
            let values = positional_values(args);
            let folder_id = values
                .get(1)
                .ok_or(CliError::MissingArgument("folder-id"))?;
            let brain_id = command_brain_id(args, env)?;
            let metadata = fetch_brain_metadata(env, args, &brain_id)?;
            let folder = metadata
                .folders
                .iter()
                .find(|folder| folder.id == *folder_id)
                .ok_or_else(|| CliError::NotFound(format!("folder {folder_id}")))?;
            let deletion_event = admin_access_change_event(
                env,
                &brain_id,
                AdminAccessAction::DeleteFolder,
                Some(folder_id),
                None,
                Some(folder.current_key_version),
            )?;
            let route = format!("/_admin/brains/{brain_id}/folders/{folder_id}");
            let response = signed_json_request(
                env,
                args,
                "DELETE",
                &route,
                Some(serde_json::json!({ "deletionEvent": deletion_event })),
            )?;
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
            let metadata = fetch_brain_metadata_for_command(env, args)?;
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
        write_folder_row(output, folder, "")?;
    }
    Ok(())
}

fn write_folder_row<W: Write>(
    output: &mut W,
    folder: &FolderMetadataView,
    details: &str,
) -> Result<(), CliError> {
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
        "folder {} path={} access={} keyVersion={} state={}{}{}",
        folder.id, folder.path, folder.access, folder.current_key_version, setup, source, details
    )?;
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
            mount.source_brain_id,
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
            let brain_id = command_brain_id(args, env)?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let event = admin_access_change_event(
                env,
                &brain_id,
                AdminAccessAction::AddMember,
                None,
                Some(&target),
                None,
            )?;
            let body = serde_json::json!({
                "targetNpub": target,
                "accessChangeEvent": event
            });
            let route = format!("/_admin/brains/{brain_id}/members");
            let response = signed_json_request(env, args, "POST", &route, Some(body))?;
            write_command_response(output, json, &response)
        }
        Some("remove-member") | Some("member-remove") => {
            let brain_id = command_brain_id(args, env)?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let event = admin_access_change_event(
                env,
                &brain_id,
                AdminAccessAction::RemoveMember,
                None,
                Some(&target),
                None,
            )?;
            let route = format!("/_admin/brains/{brain_id}/members/{target}");
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
            let brain_id = command_brain_id(args, env)?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let event = admin_access_change_event(
                env,
                &brain_id,
                AdminAccessAction::AddAdmin,
                None,
                Some(&target),
                None,
            )?;
            let body = serde_json::json!({
                "targetNpub": target,
                "accessChangeEvent": event
            });
            let route = format!("/_admin/brains/{brain_id}/admins");
            let response = signed_json_request(env, args, "POST", &route, Some(body))?;
            write_command_response(output, json, &response)
        }
        Some("remove-admin") | Some("admin-remove") => {
            let brain_id = command_brain_id(args, env)?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let event = admin_access_change_event(
                env,
                &brain_id,
                AdminAccessAction::RemoveAdmin,
                None,
                Some(&target),
                None,
            )?;
            let route = format!("/_admin/brains/{brain_id}/admins/{target}");
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
            let brain_id = command_brain_id(args, env)?;
            let folder_id =
                option_value(args, "--folder").ok_or(CliError::MissingArgument("--folder"))?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let metadata = fetch_brain_metadata(env, args, &brain_id)?;
            let key_version = metadata
                .folders
                .iter()
                .find(|folder| folder.id == folder_id)
                .map(|folder| folder.current_key_version)
                .ok_or_else(|| CliError::NotFound(format!("folder {folder_id}")))?;
            let session_keys = open_brain_session_folder_keys(env, args, &brain_id)?;
            let folder_key = opened_folder_key(&session_keys, &brain_id, &folder_id, key_version)?;
            let auth = load_signer(env)?;
            let event = admin_access_change_event(
                env,
                &brain_id,
                AdminAccessAction::GrantFolderAccess,
                Some(&folder_id),
                Some(&target),
                Some(key_version),
            )?;
            let grant = folder_key_grant_request(
                &auth,
                &brain_id,
                &folder_id,
                key_version,
                &target,
                &folder_key,
                env,
            )?;
            let route = format!("/_admin/brains/{brain_id}/folders/{folder_id}/access");
            let body = serde_json::json!({
                "targetNpub": target,
                "grant": grant,
                "accessChangeEvent": event
            });
            let response = signed_json_request(env, args, "POST", &route, Some(body))?;
            if !json && response["outcome"] == "alreadyHasAccess" {
                writeln!(output, "This person already has access.")?;
                Ok(())
            } else {
                write_command_response(output, json, &response)
            }
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
            let brain_id = command_brain_id(args, env)?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let expires_at = option_value(args, "--expires")
                .unwrap_or_else(|| "2099-01-01T00:00:00Z".to_owned());
            let folders = option_values(args, "--folder");
            let route = format!("/_admin/brains/{brain_id}/invitations");
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
                        &brain_id,
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
                    &brain_id,
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
            let route = format!("/_admin/brain-invitation-links/{code}");
            let response = signed_json_request(env, args, "GET", &route, None)?;
            write_command_response(output, json, &response)
        }
        Some("accept") => {
            let route = if let Some(code) =
                option_value(args, "--code").or_else(|| positional_values(args).get(1).cloned())
            {
                let code = require_invite_code(code)?;
                format!("/_admin/brain-invitation-links/{code}/accept")
            } else {
                let brain_id = command_brain_id(args, env)?;
                let id = option_value(args, "--id")
                    .ok_or(CliError::MissingArgument("--id or --code"))?;
                format!("/_admin/brains/{brain_id}/invitations/{id}/accept")
            };
            let response = signed_json_request(env, args, "POST", &route, None)?;
            write_command_response(output, json, &response)
        }
        Some("revoke") => {
            let brain_id = command_brain_id(args, env)?;
            let id = required_option_or_positional(args, "--id", 1, "invitation-id")?;
            let route = format!("/_admin/brains/{brain_id}/invitations/{id}");
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
    brain_id: &str,
    raw_target: &str,
    folders: &[String],
    expires_at: &str,
) -> Result<(), CliError> {
    let (body, invite_secret) =
        email_invite_create_body(env, args, brain_id, raw_target, folders, expires_at)?;
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
            "expected an invite code like invite-...; invitation-... is an invitation id. Use `fbrain invites accept --brain <brain-id> --id <invitation-id>` for by-id accept."
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
    metadata: &BrainMetadataView,
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
    brain_id: &str,
    target_email: &str,
    selected_folders: &[String],
    expires_at: &str,
) -> Result<(serde_json::Value, String), CliError> {
    let invited_email = canonical_invite_email(target_email)?;
    let metadata = fetch_brain_metadata(env, args, brain_id)?;
    let scope = email_invite_scope(&metadata, selected_folders)?;
    let auth = load_signer(env)?;
    let session_keys = open_brain_session_folder_keys(env, args, brain_id)?;
    let unwrap_keys = Keys::generate();
    let invite_unwrap_npub = NostrPublicKey::from_protocol(unwrap_keys.public_key())
        .to_npub()
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    let invite_secret = unwrap_keys.secret_key().to_secret_hex();

    let mut bootstrap_grants = Vec::new();
    for item in &scope {
        let folder_key =
            opened_folder_key(&session_keys, brain_id, &item.folder_id, item.key_version)?;
        bootstrap_grants.push(serde_json::json!({
            "folderId": item.folder_id,
            "grant": folder_key_grant_request(
                &auth,
                brain_id,
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
        "brainId": brain_id,
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
        brain_id,
        &invite_unwrap_npub,
        &bootstrap_payload_json,
    )?;
    let bootstrap_authorization_event_json = email_invite_authorization_event(
        &auth,
        brain_id,
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
    brain_id: &str,
    invite_unwrap_npub: &str,
    bootstrap_payload_json: &str,
) -> Result<String, CliError> {
    let recipient = NostrPublicKey::parse(invite_unwrap_npub)
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    let rumor = build_rumor(
        NostrPublicKey::from_protocol(auth.keys.public_key()),
        Kind::Custom(APP_SPECIFIC_KIND),
        vec![
            tag_vec(["d", &format!("finite-email-invite-bootstrap:{brain_id}")])?,
            tag_vec(["brain", brain_id])?,
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
    brain_id: &str,
    invited_email: &str,
    invite_unwrap_npub: &str,
    bootstrap_payload_hash: &str,
    expires_at: &str,
    scope: &[EmailInviteScopeItem],
) -> Result<String, CliError> {
    let content = serde_json::json!({
        "version": "finite-email-invite-bootstrap-authorization-v1",
        "brainId": brain_id,
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
                &format!("finite-email-invite-bootstrap-authorization:{brain_id}:{invited_email}"),
            ])?,
            tag_vec(["brain", brain_id])?,
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
            let brain_id = command_brain_id(args, env)?;
            let folder_id =
                option_value(args, "--folder").ok_or(CliError::MissingArgument("--folder"))?;
            let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
            let target = resolve_identity_npub(env, args, &raw_target)?;
            let expires_at = option_value(args, "--expires")
                .unwrap_or_else(|| "2099-01-01T00:00:00Z".to_owned());
            let metadata = fetch_brain_metadata(env, args, &brain_id)?;
            let key_version = metadata
                .folders
                .iter()
                .find(|folder| folder.id == folder_id)
                .map(|folder| folder.current_key_version)
                .ok_or_else(|| CliError::NotFound(format!("folder {folder_id}")))?;
            let session_keys = open_brain_session_folder_keys(env, args, &brain_id)?;
            let folder_key = opened_folder_key(&session_keys, &brain_id, &folder_id, key_version)?;
            let auth = load_signer(env)?;
            let event = admin_access_change_event(
                env,
                &brain_id,
                AdminAccessAction::GrantFolderAccess,
                Some(&folder_id),
                Some(&target),
                Some(key_version),
            )?;
            let grant = folder_key_grant_request(
                &auth,
                &brain_id,
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
            let route = format!("/_admin/brains/{brain_id}/folders/{folder_id}/share-links");
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
            let brain_id = command_brain_id(args, env)?;
            let folder_id =
                option_value(args, "--folder").ok_or(CliError::MissingArgument("--folder"))?;
            let metadata = fetch_brain_metadata(env, args, &brain_id)?;
            let key_version = metadata
                .folders
                .iter()
                .find(|folder| folder.id == folder_id)
                .map(|folder| folder.current_key_version)
                .ok_or_else(|| CliError::NotFound(format!("folder {folder_id}")))?;
            let event = admin_access_change_event(
                env,
                &brain_id,
                AdminAccessAction::SetFolderAccessMode,
                Some(&folder_id),
                None,
                Some(key_version),
            )?;
            let route = format!("/_admin/brains/{brain_id}/folders/{folder_id}/share-source");
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
            let brain_id = command_brain_id(args, env)?;
            let folder_id =
                option_value(args, "--folder").ok_or(CliError::MissingArgument("--folder"))?;
            let destination_brain_id = option_value(args, "--destination-brain")
                .ok_or(CliError::MissingArgument("--destination-brain"))?;
            let raw_destination_admin = option_value(args, "--destination-admin")
                .ok_or(CliError::MissingArgument("--destination-admin"))?;
            let destination_admin = resolve_identity_npub(env, args, &raw_destination_admin)?;
            let metadata = fetch_brain_metadata(env, args, &brain_id)?;
            let key_version = metadata
                .folders
                .iter()
                .find(|folder| folder.id == folder_id)
                .map(|folder| folder.current_key_version)
                .ok_or_else(|| CliError::NotFound(format!("folder {folder_id}")))?;
            let session_keys = open_brain_session_folder_keys(env, args, &brain_id)?;
            let folder_key = opened_folder_key(&session_keys, &brain_id, &folder_id, key_version)?;
            let auth = load_signer(env)?;
            let event = admin_access_change_event(
                env,
                &brain_id,
                AdminAccessAction::GrantFolderAccess,
                Some(&folder_id),
                Some(&destination_admin),
                Some(key_version),
            )?;
            let grant = folder_key_grant_request(
                &auth,
                &brain_id,
                &folder_id,
                key_version,
                &destination_admin,
                &folder_key,
                env,
            )?;
            let route =
                format!("/_admin/brains/{brain_id}/folders/{folder_id}/shared-folder-invitations");
            let body = serde_json::json!({
                "destinationBrainId": destination_brain_id,
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
    use finite_brain_core::{
        BrainGrantIntent, FolderId, FolderObjectAad, ObjectId, SafeRelativePath,
        encrypt_folder_object, open_folder_key_grant,
    };
    use finite_nostr::{
        GiftWrapValidation, NostrPublicKey, decode_http_auth_header, open_gift_wrap,
    };
    use nostr::{Event, Keys};
    use serde_json::Value;
    use std::io::{ErrorKind, Read};
    #[cfg(unix)]
    use std::io::{Seek, SeekFrom};
    use std::net::{TcpListener, TcpStream};
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::process::{Command as ProcessCommand, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    fn env_for(tmp: &TempDir) -> CliEnvironment {
        CliEnvironment {
            cwd: tmp.path().to_path_buf(),
            config_dir: tmp.path().join("config"),
            working_tree_root: None,
            now: Some("2026-06-24T20:46:36Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(tmp.path().join("finite-home")),
            embedding_provider: None,
        }
    }

    fn semantic_test_process(test_name: &str) -> ProcessCommand {
        let mut command = ProcessCommand::new(std::env::current_exe().unwrap());
        command
            .arg(test_name)
            .arg("--ignored")
            .arg("--exact")
            .arg("--nocapture")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn spawn_semantic_refresh_process(
        tree: &Path,
        config: &EmbeddingProviderConfig,
        report_path: &Path,
    ) -> std::process::Child {
        semantic_test_process("tests::semantic_refresh_process_helper")
            .env("FBRAIN_TEST_SEMANTIC_TREE", tree)
            .env("FBRAIN_TEST_EMBEDDING_ENDPOINT", &config.endpoint)
            .env("FBRAIN_TEST_EMBEDDING_TOKEN", &config.bearer_token)
            .env("FBRAIN_TEST_SEMANTIC_REPORT", report_path)
            .spawn()
            .unwrap()
    }

    fn read_semantic_refresh_report(path: &Path) -> SemanticRefreshReport {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
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

    #[cfg(unix)]
    fn unix_mode(path: &std::path::Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
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
                            "brain": {
                                "id": "brain",
                                "kind": "personal",
                                "name": "Brain",
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
                        "brainId": "brain",
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
            "brain": {
                "id": "brain",
                "kind": "personal",
                "name": "Brain",
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
                            "brainId": "brain",
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

    fn start_two_member_identity_incremental_sync_server(
        export_grants: Vec<Value>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            let mut accepted_object = None::<(String, String, String)>;
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
                    let body_json: Value = serde_json::from_str(&body).unwrap();
                    let revision_event =
                        Event::from_json(body_json["revisionEvent"].to_string()).unwrap();
                    revision_event.verify().unwrap();
                    let writer_npub = NostrPublicKey::from_protocol(revision_event.pubkey)
                        .to_npub()
                        .unwrap();
                    let ciphertext = body_json["ciphertext"].as_str().unwrap().to_owned();
                    accepted_object = Some((object_id, ciphertext, writer_npub));
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
                        .map(|(object_id, ciphertext, _)| {
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
                        .map(|(object_id, ciphertext, writer_npub)| {
                            vec![serde_json::json!({
                                "sequence": 1,
                                "recordEventId": "evt-agent-b-1",
                                "recordType": "folder_object_revision",
                                "folderId": "general",
                                "objectId": object_id,
                                "revision": 1,
                                "actorNpub": writer_npub,
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
                            "brainId": "brain",
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
            brain_id: BrainId::new("brain").unwrap(),
            folder_id: FolderId::new("general").unwrap(),
            object_id: ObjectId::new("obj_remote000001").unwrap(),
            key_version: 1,
        };
        encrypt_folder_object(folder_key, &aad, &plaintext)
            .unwrap()
            .canonical_json()
    }

    fn setup_incremental_tree(tmp: &TempDir, latest_sequence: u64) -> PathBuf {
        setup_incremental_tree_named(tmp, "brain", latest_sequence)
    }

    fn setup_incremental_tree_named(tmp: &TempDir, name: &str, latest_sequence: u64) -> PathBuf {
        let tree = tmp.path().join(name);
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        let now = "2026-06-24T20:46:36Z";
        write_agent_state(&tree, &AgentState::new("brain", now)).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
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
        grant_outcome: &'static str,
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
                        "brainId": "acme",
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
                        "brain": {
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
                    serde_json::json!({
                        "brainId": "acme",
                        "outcome": grant_outcome,
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
                        "brain": {
                            "id": "brain",
                            "kind": "personal",
                            "name": "Brain",
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
        brain_id: &str,
        folder_id: &str,
        key_version: u32,
        folder_key: &FolderKey,
        recipient_npub: &str,
    ) -> Value {
        let auth = load_signer(env).unwrap();
        let grant = folder_key_grant_request(
            &auth,
            brain_id,
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
                        "brainId": "acme",
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
                        "brain": {
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
                        "brainId": "acme",
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
                        "acceptPath": "/_admin/brain-invitation-links/invite-email/claim",
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
                    "brainId": "acme",
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
                    }, {
                        "id": "leadership",
                        "name": "Leadership",
                        "role": "folder",
                        "access": "admin_only",
                        "parentFolderId": null,
                        "path": "leadership",
                        "sharedFolderSource": false,
                        "accessUserIds": [],
                        "currentKeyVersion": 1,
                        "setupIncomplete": false
                    }, {
                        "id": "project",
                        "name": "Project",
                        "role": "folder",
                        "access": "restricted",
                        "parentFolderId": null,
                        "path": "project",
                        "sharedFolderSource": false,
                        "accessUserIds": ["npub-member"],
                        "currentKeyVersion": 1,
                        "setupIncomplete": false
                    }],
                    "mountedFolders": [{
                        "mountId": "mount-1",
                        "organizationBrainId": "acme",
                        "sourceBrainId": "partner",
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

    fn start_personal_access_listing_server() -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (request_line, _) = read_http_request(&mut stream);
            let body = serde_json::json!({
                "brainId": "personal-alice",
                "kind": "personal",
                "name": "Personal Brain",
                "ownerUserId": "npub-owner",
                "personalAgent": { "agentNpub": "npub-agent" },
                "members": ["npub-owner", "npub-collaborator"],
                "admins": [],
                "folders": [{
                    "id": "private",
                    "name": "Private",
                    "role": "folder",
                    "access": "owner",
                    "parentFolderId": null,
                    "path": "private",
                    "sharedFolderSource": false,
                    "accessUserIds": [],
                    "currentKeyVersion": 1,
                    "setupIncomplete": false
                }, {
                    "id": "shared",
                    "name": "Shared",
                    "role": "folder",
                    "access": "restricted",
                    "parentFolderId": null,
                    "path": "shared",
                    "sharedFolderSource": false,
                    "accessUserIds": ["npub-collaborator"],
                    "currentKeyVersion": 1,
                    "setupIncomplete": false
                }],
                "mountedFolders": [],
                "grantCount": 5
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            request_line
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
                            "brain": {
                                "id": "brain",
                                "kind": "personal",
                                "name": "Brain",
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
    fn wiki_check_reports_missing_and_ambiguous_links_from_readable_folders_only() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        let now = "2026-06-24T20:46:36Z";
        write_agent_state(&tree, &AgentState::new("brain", now)).unwrap();
        for folder in ["Notes/compiled", "Alpha", "Beta", "Locked"] {
            fs::create_dir_all(tree.join(folder)).unwrap();
        }
        fs::write(
            tree.join("Notes/summary.md"),
            "# Summary\n\nSee [[compiled/deep.md|Deep Dive]], [[Missing]], and [[Roadmap]].\n",
        )
        .unwrap();
        fs::write(
            tree.join("Notes/compiled/deep.md"),
            "# Deep Dive\n\nBack to [Summary](summary.md).\n",
        )
        .unwrap();
        fs::write(tree.join("Alpha/roadmap.md"), "# Roadmap\n").unwrap();
        fs::write(tree.join("Beta/roadmap.md"), "# Roadmap\n").unwrap();
        fs::write(tree.join("Locked/roadmap.md"), "# Roadmap\n").unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![
                    WorkingTreeFolderRoot {
                        folder_id: "notes".to_owned(),
                        source_brain_id: None,
                        path: "Notes".to_owned(),
                        can_read: true,
                        metadata_only: false,
                    },
                    WorkingTreeFolderRoot {
                        folder_id: "alpha".to_owned(),
                        source_brain_id: None,
                        path: "Alpha".to_owned(),
                        can_read: true,
                        metadata_only: false,
                    },
                    WorkingTreeFolderRoot {
                        folder_id: "beta".to_owned(),
                        source_brain_id: None,
                        path: "Beta".to_owned(),
                        can_read: true,
                        metadata_only: false,
                    },
                    WorkingTreeFolderRoot {
                        folder_id: "locked".to_owned(),
                        source_brain_id: None,
                        path: "Locked".to_owned(),
                        can_read: false,
                        metadata_only: true,
                    },
                ],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();

        let mut output = Vec::new();
        let mut env = env_for(&tmp);
        env.cwd = tree;
        run_with_env(["wiki", "check", "--json"], env, &mut output).unwrap();
        let report: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(report["status"], "issues");
        assert_eq!(report["pageCount"], 4);
        assert_eq!(report["resolvedLinkCount"], 2);
        assert_eq!(report["missingLinkCount"], 1);
        assert_eq!(report["ambiguousLinkCount"], 1);
        assert_eq!(report["issues"][0]["path"], "Notes/summary.md");
        assert_eq!(report["issues"][0]["reference"], "Missing");
        assert_eq!(report["issues"][0]["status"], "missing");
        assert_eq!(report["issues"][1]["reference"], "Roadmap");
        assert_eq!(report["issues"][1]["status"], "ambiguous");
        assert_eq!(
            report["issues"][1]["matches"],
            serde_json::json!(["Alpha/roadmap.md", "Beta/roadmap.md"])
        );
    }

    #[test]
    fn wiki_check_uses_folder_routes_and_normalized_reference_rules() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        fs::create_dir_all(tree.join("Local")).unwrap();
        fs::create_dir_all(tree.join("Mounted")).unwrap();
        fs::write(
            tree.join("Local/source.md"),
            "# Source\n\n[[Guide]] [[guide]] [[Re\u{301}sume\u{301}]] [Angle](<angle.md>) [Web](HTTPS://example.com) [Mail](mailto:person@example.com) ![Diagram](raw/assets/diagram.png) [PDF](raw/assets/source.pdf) [Manual](manual.md \"read\") [Version](guide(v2).md)\n\n`[Ghost](missing.md)`\n\n```md\n```not-a-close\n[Ghost](missing.md)\n[[Ghost]]\n```\n\n    [Ghost](missing.md)\n\nParagraph\n    [Paragraph Guide](paragraph.md)\n\n- item\n\n    [List Guide](list.md)\n\n[[Guide]] [Manual][manual-ref] [Duplicate][duplicate] [see [Manual]](upper.md) [escaped \\] label](manual.md) [Escaped Version](guide\\(v2\\).md)\n\n[Guide]: missing.md\n[manual-ref]: manual.md\n[duplicate]: upper.md\n[duplicate]: missing.md\n",
        )
        .unwrap();
        fs::write(tree.join("Local/upper.md"), "# Guide\n").unwrap();
        fs::write(tree.join("Local/lower.md"), "# guide\n").unwrap();
        fs::write(tree.join("Local/resume.md"), "# R\u{e9}sum\u{e9}\n").unwrap();
        fs::write(tree.join("Local/angle.md"), "# Angle\n").unwrap();
        fs::write(tree.join("Local/manual.md"), "# Manual\n").unwrap();
        fs::write(tree.join("Local/guide(v2).md"), "# Version Guide\n").unwrap();
        fs::write(tree.join("Local/paragraph.md"), "# Paragraph Guide\n").unwrap();
        fs::write(tree.join("Local/list.md"), "# List Guide\n").unwrap();
        fs::write(tree.join("Mounted/guide.md"), "# Guide\n").unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![
                    WorkingTreeFolderRoot {
                        folder_id: "notes".to_owned(),
                        source_brain_id: None,
                        path: "Local".to_owned(),
                        can_read: true,
                        metadata_only: false,
                    },
                    WorkingTreeFolderRoot {
                        folder_id: "notes".to_owned(),
                        source_brain_id: Some("mounted-brain".to_owned()),
                        path: "Mounted".to_owned(),
                        can_read: true,
                        metadata_only: false,
                    },
                ],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();

        let mut output = Vec::new();
        let mut env = env_for(&tmp);
        env.cwd = tree;
        run_with_env(["wiki", "check", "--json"], env, &mut output).unwrap();
        let report: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(report["status"], "ok", "{report:#}");
        assert_eq!(report["pageCount"], 10);
        assert_eq!(report["resolvedLinkCount"], 9);
        assert_eq!(report["missingLinkCount"], 0);
        assert_eq!(report["ambiguousLinkCount"], 0);
    }

    #[cfg(unix)]
    #[test]
    fn wiki_check_rejects_folder_root_and_intermediate_symlinks() {
        use std::os::unix::fs::symlink;

        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        let external = tmp.path().join("external");
        fs::create_dir_all(external.join("Notes")).unwrap();
        fs::write(external.join("Notes/secret.md"), "# External Secret\n").unwrap();

        for (folder_path, link_path, target) in [
            ("Notes", tree.join("Notes"), external.join("Notes")),
            ("Mount/Notes", tree.join("Mount"), external.clone()),
        ] {
            symlink(target, &link_path).unwrap();
            write_json_file(
                &tree.join(".finitebrain/working-tree-state.json"),
                &BrainWorkingTreeStateManifest {
                    version: WORKING_TREE_STATE_VERSION.to_owned(),
                    folder_roots: vec![WorkingTreeFolderRoot {
                        folder_id: "notes".to_owned(),
                        source_brain_id: None,
                        path: folder_path.to_owned(),
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
            assert!(matches!(
                run_with_env(["wiki", "check", "--json"], env, &mut Vec::new()),
                Err(CliError::InsecureWorkingTree { .. })
            ));
            fs::remove_file(link_path).unwrap();
        }
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
        let tree = tmp.path().join("agent-brain");
        let output = run(
            &tmp,
            &[
                "open",
                "agent-brain",
                tree.to_str().unwrap(),
                "--server",
                "http://127.0.0.1:3015",
                "--json",
            ],
        );
        let json: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(json["brainId"], "agent-brain");
        assert_eq!(json["daemon"], "running");
        assert!(tree.join(".finitebrain/brain-directory.json").exists());
        assert!(tree.join(".finitebrain/working-tree-state.json").exists());
        assert!(tree.join(".finitebrain/agent-state.json").exists());
        #[cfg(unix)]
        {
            assert_eq!(unix_mode(&tree), 0o700);
            assert_eq!(unix_mode(&tree.join(".finitebrain")), 0o700);
            assert_eq!(unix_mode(&tree.join(".finitebrain/encrypted-sync")), 0o700);
            assert_eq!(
                unix_mode(&tree.join(".finitebrain/brain-directory.json")),
                0o600
            );
            assert_eq!(
                unix_mode(&tree.join(".finitebrain/working-tree-state.json")),
                0o600
            );
            assert_eq!(
                unix_mode(&tree.join(".finitebrain/agent-state.json")),
                0o600
            );
        }

        let mut env = env_for(&tmp);
        env.cwd = tree;
        let mut output = Vec::new();
        run_with_env(["status", "--json"], env, &mut output).unwrap();
        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["brainId"], "agent-brain");
        assert_eq!(json["auth"]["state"], "authenticated");
        assert_eq!(json["daemon"]["state"], "running");
        assert_eq!(json["sync"]["mode"], "automatic");
    }

    #[test]
    fn open_defaults_to_the_configured_working_tree_root() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let working_tree_root = tmp.path().join("data/workspace/finitebrain");
        let mut env = env_for(&tmp);
        env.working_tree_root = Some(working_tree_root.clone());
        let mut output = Vec::new();

        run_with_env(
            [
                "open",
                "paired-personal-brain",
                "--server",
                "http://127.0.0.1:3015",
                "--json",
            ],
            env,
            &mut output,
        )
        .unwrap();

        let json: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            json["path"],
            working_tree_root
                .join("paired-personal-brain")
                .display()
                .to_string()
        );
        assert!(
            working_tree_root
                .join("paired-personal-brain/.finitebrain/agent-state.json")
                .exists()
        );
    }

    #[test]
    fn open_rejects_brain_id_path_escape_before_creating_default_tree() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let working_tree_root = tmp.path().join("data/workspace/finitebrain");
        let mut env = env_for(&tmp);
        env.working_tree_root = Some(working_tree_root.clone());

        let error =
            run_with_env(["open", "../outside", "--json"], env, &mut Vec::new()).unwrap_err();

        assert!(matches!(error, CliError::InvalidInput(_)));
        assert!(!working_tree_root.join("../outside").exists());
    }

    #[test]
    fn reopening_same_brain_preserves_sync_state_and_does_not_claim_signer_ownership() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("paired-personal-brain");
        run(
            &tmp,
            &["open", "paired-personal-brain", tree.to_str().unwrap()],
        );
        let directory_path = tree.join(".finitebrain/brain-directory.json");
        let directory: Value = serde_json::from_slice(&fs::read(&directory_path).unwrap()).unwrap();
        assert_eq!(directory["brain"]["ownerNpub"], Value::Null);

        let state_path = tree.join(".finitebrain/working-tree-state.json");
        let preserved = serde_json::json!({
            "version": WORKING_TREE_STATE_VERSION,
            "folderRoots": [{
                "folderId": "agent-workspace",
                "path": "agent-workspace",
                "canRead": true,
                "metadataOnly": false
            }],
            "objects": [],
            "sync": { "latestSequence": 42 }
        });
        write_json_file(&state_path, &preserved).unwrap();
        let before = fs::read(&state_path).unwrap();

        run(
            &tmp,
            &["open", "paired-personal-brain", tree.to_str().unwrap()],
        );

        assert_eq!(fs::read(&state_path).unwrap(), before);
        let agent_state = read_agent_state(&tree).unwrap();
        assert_eq!(agent_state.brain_id, "paired-personal-brain");
        assert!(
            agent_state
                .activity
                .iter()
                .any(|entry| entry.kind == "working_tree.reopened")
        );
    }

    #[test]
    fn open_fails_closed_on_portable_tree_without_agent_state() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("portable-brain");
        initialize_private_working_tree(&tree).unwrap();
        let directory_path = tree.join(".finitebrain/brain-directory.json");
        let state_path = tree.join(".finitebrain/working-tree-state.json");
        write_json_file(&directory_path, &serde_json::json!({ "portable": true })).unwrap();
        write_json_file(&state_path, &serde_json::json!({ "latestSequence": 73 })).unwrap();
        let directory_before = fs::read(&directory_path).unwrap();
        let state_before = fs::read(&state_path).unwrap();

        let error = run_with_env(
            ["open", "portable-brain", tree.to_str().unwrap()],
            env_for(&tmp),
            &mut Vec::new(),
        )
        .unwrap_err();

        assert!(matches!(error, CliError::InvalidInput(_)));
        assert!(error.to_string().contains("portable manifests"));
        assert_eq!(fs::read(&directory_path).unwrap(), directory_before);
        assert_eq!(fs::read(&state_path).unwrap(), state_before);
        assert!(!tree.join(".finitebrain/agent-state.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn insecure_working_tree_fails_closed_and_repair_does_not_touch_member_content() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let member_file = tree.join("General/member-note.md");
        fs::create_dir_all(member_file.parent().unwrap()).unwrap();
        fs::write(&member_file, "member plaintext\n").unwrap();
        fs::set_permissions(&member_file, fs::Permissions::from_mode(0o640)).unwrap();
        fs::set_permissions(&tree, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(tree.join(".finitebrain"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(
            tree.join(".finitebrain/agent-state.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let error = run_with_env(["status", "--json"], env.clone(), &mut Vec::new()).unwrap_err();
        assert!(matches!(
            &error,
            CliError::InsecureWorkingTreePermissions { .. }
        ));
        assert!(error.to_string().contains("fbrain repair"));

        let mut doctor_output = Vec::new();
        run_with_env(["doctor", "--json"], env.clone(), &mut doctor_output).unwrap();
        let doctor: Value = serde_json::from_slice(&doctor_output).unwrap();
        assert_eq!(doctor["workingTree"]["state"], "warn");
        assert!(
            doctor["workingTree"]["message"]
                .as_str()
                .unwrap()
                .contains("fbrain repair")
        );

        let mut repair_output = Vec::new();
        run_with_env(["repair", "--json"], env.clone(), &mut repair_output).unwrap();
        let repair: Value = serde_json::from_slice(&repair_output).unwrap();
        assert_eq!(repair["state"], "repaired");
        run_with_env(["status", "--json"], env, &mut Vec::new()).unwrap();

        assert_eq!(unix_mode(&tree), 0o700);
        assert_eq!(unix_mode(&tree.join(".finitebrain")), 0o700);
        assert_eq!(
            unix_mode(&tree.join(".finitebrain/agent-state.json")),
            0o600
        );
        assert_eq!(unix_mode(&member_file), 0o640);
        assert_eq!(
            fs::read_to_string(member_file).unwrap(),
            "member plaintext\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn managed_symlink_is_rejected_before_external_state_is_read() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let external = tmp.path().join("external-agent-state.json");
        fs::write(&external, "external sentinel").unwrap();
        let agent_state = tree.join(".finitebrain/agent-state.json");
        fs::remove_file(&agent_state).unwrap();
        symlink(&external, &agent_state).unwrap();

        let mut env = env_for(&tmp);
        env.cwd = tree;
        let error = run_with_env(["status", "--json"], env.clone(), &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(error.contains("managed symlink"));
        assert!(error.contains("fbrain repair"));
        assert_eq!(fs::read_to_string(&external).unwrap(), "external sentinel");

        let mut doctor_output = Vec::new();
        run_with_env(["doctor", "--json"], env, &mut doctor_output).unwrap();
        let doctor: Value = serde_json::from_slice(&doctor_output).unwrap();
        assert_eq!(doctor["workingTree"]["state"], "warn");
        assert!(
            doctor["workingTree"]["message"]
                .as_str()
                .unwrap()
                .contains("managed symlink")
        );
    }

    #[cfg(unix)]
    #[test]
    fn broken_control_directory_symlink_is_discovered_without_following_it() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let control_dir = tree.join(".finitebrain");
        fs::rename(&control_dir, tree.join("finitebrain-backup")).unwrap();
        symlink(tmp.path().join("missing-external-control"), &control_dir).unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree;

        let error = run_with_env(["status", "--json"], env.clone(), &mut Vec::new()).unwrap_err();
        assert!(matches!(&error, CliError::InsecureWorkingTree { .. }));
        assert!(error.to_string().contains("managed symlink"));

        let mut doctor_output = Vec::new();
        run_with_env(["doctor", "--json"], env.clone(), &mut doctor_output).unwrap();
        let doctor: Value = serde_json::from_slice(&doctor_output).unwrap();
        assert_eq!(doctor["workingTree"]["state"], "warn");
        assert!(
            doctor["workingTree"]["message"]
                .as_str()
                .unwrap()
                .contains("managed symlink")
        );
        let repair_error = run_with_env(["repair"], env, &mut Vec::new()).unwrap_err();
        assert!(repair_error.to_string().contains("remove the link"));
    }

    #[cfg(unix)]
    #[test]
    fn repair_recovers_an_untraversable_control_directory() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let control_dir = tree.join(".finitebrain");
        fs::set_permissions(&control_dir, fs::Permissions::from_mode(0o100)).unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();

        let mut doctor_output = Vec::new();
        run_with_env(["doctor", "--json"], env.clone(), &mut doctor_output).unwrap();
        let doctor: Value = serde_json::from_slice(&doctor_output).unwrap();
        assert_eq!(doctor["workingTree"]["state"], "warn");
        assert!(
            doctor["workingTree"]["message"]
                .as_str()
                .unwrap()
                .contains("expected 0700")
        );

        run_with_env(["repair"], env.clone(), &mut Vec::new()).unwrap();
        run_with_env(["status", "--json"], env, &mut Vec::new()).unwrap();
        assert_eq!(unix_mode(&control_dir), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_managed_file_reports_typed_permissions_and_repair_recovers_it() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let state_path = tree.join(".finitebrain/agent-state.json");
        fs::set_permissions(&state_path, fs::Permissions::from_mode(0o000)).unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();

        let error = run_with_env(["status", "--json"], env.clone(), &mut Vec::new()).unwrap_err();
        assert!(matches!(
            error,
            CliError::InsecureWorkingTreePermissions { .. }
        ));

        let mut doctor_output = Vec::new();
        run_with_env(["doctor", "--json"], env.clone(), &mut doctor_output).unwrap();
        let doctor: Value = serde_json::from_slice(&doctor_output).unwrap();
        assert_eq!(doctor["workingTree"]["state"], "warn");
        assert!(
            doctor["workingTree"]["message"]
                .as_str()
                .unwrap()
                .contains("expected 0600")
        );

        run_with_env(["repair"], env.clone(), &mut Vec::new()).unwrap();
        run_with_env(["status", "--json"], env, &mut Vec::new()).unwrap();
        assert_eq!(unix_mode(&state_path), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn finite_control_file_replacement_is_private_and_atomic() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let agent_state_path = tree.join(".finitebrain/agent-state.json");
        let old_body = fs::read_to_string(&agent_state_path).unwrap();
        let mut old_handle = fs::File::open(&agent_state_path).unwrap();
        let mut state = read_agent_state(&tree).unwrap();
        state.sync.status = "atomic-replacement".to_owned();

        write_agent_state(&tree, &state).unwrap();

        let mut body_from_old_inode = String::new();
        old_handle.seek(SeekFrom::Start(0)).unwrap();
        old_handle.read_to_string(&mut body_from_old_inode).unwrap();
        assert_eq!(body_from_old_inode, old_body);
        assert!(
            fs::read_to_string(&agent_state_path)
                .unwrap()
                .contains("atomic-replacement")
        );
        assert_eq!(unix_mode(&agent_state_path), 0o600);
        assert!(
            fs::read_dir(tree.join(".finitebrain"))
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp-"))
        );
    }

    #[test]
    fn legacy_agent_state_is_scrubbed_and_restored_legacy_state_is_scrubbed_again() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let state_path = tree.join(".finitebrain/agent-state.json");
        let raw_folder_key = FolderKey::from_bytes([99; 32]).to_base64();
        let mut legacy: Value =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        legacy["version"] = Value::String("finitebrain-agent-state-v1".to_owned());
        legacy["localFolderKeys"] = serde_json::json!([{
            "brainId": "brain",
            "folderId": "general",
            "keyVersion": 1,
            "keyBase64": raw_folder_key,
            "source": "legacy-test",
            "openedAt": "2026-06-24T20:46:36Z"
        }]);
        legacy["unlockedFolders"] = serde_json::json!([{
            "brainId": "brain",
            "folderId": "general",
            "keyVersion": 1,
            "source": "legacy-test",
            "openedAt": "2026-06-24T20:46:36Z"
        }]);
        write_json_file(&state_path, &legacy).unwrap();

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let mut status_output = Vec::new();
        run_with_env(["status", "--json"], env.clone(), &mut status_output).unwrap();
        let status: Value = serde_json::from_slice(&status_output).unwrap();
        assert!(status.get("unlockedFolders").is_none());
        let migrated_body = fs::read_to_string(&state_path).unwrap();
        let migrated: Value = serde_json::from_str(&migrated_body).unwrap();
        assert_eq!(migrated["version"], "finitebrain-agent-state-v2");
        assert!(migrated.get("localFolderKeys").is_none());
        assert!(migrated.get("unlockedFolders").is_none());
        assert!(!migrated_body.contains(&raw_folder_key));

        let mut human_status = Vec::new();
        run_with_env(["status"], env.clone(), &mut human_status).unwrap();
        assert!(
            !String::from_utf8(human_status)
                .unwrap()
                .contains("Unlocked Folders")
        );

        write_json_file(&state_path, &legacy).unwrap();
        run_with_env(["conflicts", "--json"], env, &mut Vec::new()).unwrap();
        let remigrated_body = fs::read_to_string(state_path).unwrap();
        assert!(!remigrated_body.contains(&raw_folder_key));
        assert!(!remigrated_body.contains("localFolderKeys"));
        assert!(!remigrated_body.contains("unlockedFolders"));
    }

    #[cfg(unix)]
    #[test]
    fn legacy_keys_are_scrubbed_before_insecure_permissions_block_the_command() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let state_path = tree.join(".finitebrain/agent-state.json");
        let raw_folder_key = FolderKey::from_bytes([101; 32]).to_base64();
        let mut legacy: Value =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        legacy["version"] = Value::String("finitebrain-agent-state-v1".to_owned());
        legacy["localFolderKeys"] = serde_json::json!([{
            "folderId": "general",
            "keyVersion": 1,
            "keyBase64": raw_folder_key
        }]);
        legacy["unlockedFolders"] = serde_json::json!([{"folderId": "general"}]);
        write_json_file(&state_path, &legacy).unwrap();
        fs::set_permissions(&tree, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(tree.join(".finitebrain"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&state_path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();

        let error = run_with_env(["status", "--json"], env.clone(), &mut Vec::new()).unwrap_err();

        assert!(matches!(
            error,
            CliError::InsecureWorkingTreePermissions { .. }
        ));
        let migrated_body = fs::read_to_string(&state_path).unwrap();
        assert!(migrated_body.contains("finitebrain-agent-state-v2"));
        assert!(!migrated_body.contains(&raw_folder_key));
        assert!(!migrated_body.contains("localFolderKeys"));
        assert!(!migrated_body.contains("unlockedFolders"));
        assert_eq!(unix_mode(&state_path), 0o600);

        run_with_env(["repair"], env.clone(), &mut Vec::new()).unwrap();
        run_with_env(["status", "--json"], env, &mut Vec::new()).unwrap();
    }

    #[test]
    fn unknown_future_agent_state_is_rejected_without_rewrite() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let state_path = tree.join(".finitebrain/agent-state.json");
        let mut future: Value =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        future["version"] = Value::String("finitebrain-agent-state-v3".to_owned());
        future["localFolderKeys"] = serde_json::json!([{"future": "opaque"}]);
        write_json_file(&state_path, &future).unwrap();
        let before = fs::read(&state_path).unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree;

        let error = run_with_env(["status", "--json"], env, &mut Vec::new()).unwrap_err();

        assert!(matches!(error, CliError::AgentStateMigration { .. }));
        assert_eq!(fs::read(state_path).unwrap(), before);
    }

    #[test]
    fn removed_unlock_fails_with_sync_guidance_after_scrubbing_legacy_state() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let state_path = tree.join(".finitebrain/agent-state.json");
        let raw_folder_key = FolderKey::from_bytes([100; 32]).to_base64();
        let mut legacy: Value =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        legacy["version"] = Value::String("finitebrain-agent-state-v1".to_owned());
        legacy["localFolderKeys"] = serde_json::json!([{
            "folderId": "general",
            "keyVersion": 1,
            "keyBase64": raw_folder_key,
            "source": "legacy-test",
            "openedAt": "2026-06-24T20:46:36Z"
        }]);
        legacy["unlockedFolders"] = serde_json::json!([]);
        write_json_file(&state_path, &legacy).unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree;

        let error = run_with_env(["unlock", "--all"], env, &mut Vec::new())
            .unwrap_err()
            .to_string();

        assert!(error.contains("fbrain unlock was removed"));
        assert!(error.contains("fbrain sync now"));
        let migrated_body = fs::read_to_string(state_path).unwrap();
        assert!(!migrated_body.contains(&raw_folder_key));
        assert!(!migrated_body.contains("localFolderKeys"));
        assert!(!migrated_body.contains("unlockedFolders"));
    }

    #[test]
    fn grant_folder_opens_session_key_without_persisting_it() {
        let tmp = TempDir::new().unwrap();
        let secret = "0000000000000000000000000000000000000000000000000000000000000001";
        import_identity_secret(&tmp, secret);
        let admin_npub = run(&tmp, &["signer", "public-key"]).trim().to_owned();
        let folder_key = FolderKey::from_bytes([7; 32]);
        let tree = tmp.path().join("org");
        initialize_private_working_tree(&tree).unwrap();
        write_agent_state(&tree, &AgentState::new("acme", "2026-06-24T20:46:36Z")).unwrap();

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let export_grant =
            export_grant_for_test(&env, "acme", "general", 1, &folder_key, &admin_npub);
        let (server_url, server) = start_metadata_export_and_grant_server(
            admin_npub.clone(),
            vec![export_grant],
            "granted",
        );
        let mut output = Vec::new();
        run_with_env(
            [
                "permissions",
                "grant-folder",
                "--brain",
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
    fn redundant_folder_grant_reports_that_the_person_already_has_access() {
        let output = run_redundant_folder_grant(false);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "This person already has access.\n"
        );
    }

    #[test]
    fn redundant_folder_grant_json_preserves_machine_readable_outcome() {
        let output = run_redundant_folder_grant(true);
        let response: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(response["brainId"], "acme");
        assert_eq!(response["outcome"], "alreadyHasAccess");
    }

    fn run_redundant_folder_grant(json: bool) -> Vec<u8> {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(&tmp, TEST_SECRET_HEX);
        let admin_npub = run(&tmp, &["signer", "public-key"]).trim().to_owned();
        let folder_key = FolderKey::from_bytes([7; 32]);
        let tree = tmp.path().join("org");
        initialize_private_working_tree(&tree).unwrap();
        write_agent_state(&tree, &AgentState::new("acme", "2026-06-24T20:46:36Z")).unwrap();

        let mut env = env_for(&tmp);
        env.cwd = tree;
        let export_grant =
            export_grant_for_test(&env, "acme", "general", 1, &folder_key, &admin_npub);
        let (server_url, server) = start_metadata_export_and_grant_server(
            admin_npub.clone(),
            vec![export_grant],
            "alreadyHasAccess",
        );
        let mut output = Vec::new();
        let mut args = vec![
            "permissions".to_owned(),
            "grant-folder".to_owned(),
            "--brain".to_owned(),
            "acme".to_owned(),
            "--folder".to_owned(),
            "general".to_owned(),
            "--target".to_owned(),
            admin_npub,
            "--server".to_owned(),
            server_url,
        ];
        if json {
            args.push("--json".to_owned());
        }

        run_with_env(args, env, &mut output).unwrap();

        server.join().unwrap();
        output
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
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
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
            &BrainWorkingTreeStateManifest {
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
            export_grant_for_test(&env, "brain", "general", 1, &folder_key, &actor_npub);
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
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        let state_path = tree.join(".finitebrain/agent-state.json");
        let mut legacy: Value =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        legacy["version"] = Value::String("finitebrain-agent-state-v1".to_owned());
        legacy["localFolderKeys"] = serde_json::json!([{
            "folderId": "general",
            "keyVersion": 1,
            "keyBase64": folder_key.to_base64(),
            "source": "legacy-test",
            "openedAt": "2026-06-24T20:46:36Z"
        }]);
        legacy["unlockedFolders"] = serde_json::json!([]);
        write_json_file(&state_path, &legacy).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
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
            export_grant_for_test(&env, "brain", "general", 1, &folder_key, &actor_npub);
        export_grant["wrappedEventJson"] = Value::String("not-a-nostr-event".to_owned());
        let (server_url, server) = start_session_key_sync_server(export_grant, String::new(), 1);

        let error = run_with_env(
            ["sync", "now", "--server", &server_url, "--json"],
            env,
            &mut Vec::new(),
        )
        .unwrap_err();

        assert!(matches!(&error, CliError::GrantOpening { .. }));
        let error = error.to_string();
        assert!(error.contains("encrypted Folder Key Grant"));
        assert!(error.contains("local signer"));
        assert!(!error.contains("not-a-nostr-event"));
        let migrated_body = fs::read_to_string(state_path).unwrap();
        assert!(!migrated_body.contains(&folder_key.to_base64()));
        assert!(!migrated_body.contains("localFolderKeys"));
        assert!(!migrated_body.contains("unlockedFolders"));
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[test]
    fn sync_now_scrubs_legacy_keys_before_an_unavailable_signer_blocks_the_command() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let state_path = tree.join(".finitebrain/agent-state.json");
        let raw_folder_key = FolderKey::from_bytes([103; 32]).to_base64();
        let mut legacy: Value =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        legacy["version"] = Value::String("finitebrain-agent-state-v1".to_owned());
        legacy["localFolderKeys"] = serde_json::json!([{
            "folderId": "general",
            "keyVersion": 1,
            "keyBase64": raw_folder_key,
            "source": "legacy-test",
            "openedAt": "2026-06-24T20:46:36Z"
        }]);
        legacy["unlockedFolders"] = serde_json::json!([{"folderId": "general"}]);
        write_json_file(&state_path, &legacy).unwrap();

        let corrupt_identity = "corrupt-identity-sentinel";
        fs::write(identity_file_for(&tmp), corrupt_identity).unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree;

        let error = run_with_env(
            ["sync", "now", "--server", "http://127.0.0.1:9", "--json"],
            env,
            &mut Vec::new(),
        )
        .unwrap_err();

        assert!(matches!(&error, CliError::Identity(_)));
        let error = error.to_string();
        assert!(!error.contains(corrupt_identity));
        assert!(!error.contains(&raw_folder_key));
        let migrated_body = fs::read_to_string(state_path).unwrap();
        assert!(migrated_body.contains("finitebrain-agent-state-v2"));
        assert!(!migrated_body.contains(&raw_folder_key));
        assert!(!migrated_body.contains("localFolderKeys"));
        assert!(!migrated_body.contains("unlockedFolders"));
    }

    #[test]
    fn brain_bootstrap_personal_uses_the_signed_agent_authority_route_without_owner_input() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let (server_url, server) = start_ok_capture_server(1);

        let mut output = Vec::new();
        run_with_env(
            [
                "brain",
                "bootstrap-personal",
                "--server",
                &server_url,
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();

        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]
                .0
                .starts_with("POST /_admin/personal-brain-bootstrap ")
        );
        assert_eq!(requests[0].1, "{}");
    }

    #[test]
    fn brain_create_starts_personal_and_organization_brains_empty() {
        let cases = [
            ("personal", "personal-empty", "Personal empty"),
            ("organization", "org-empty", "Org empty"),
        ];

        for (kind, brain_id, name) in cases {
            let tmp = TempDir::new().unwrap();
            import_identity_secret(&tmp, TEST_SECRET_HEX);
            let (server_url, server) = start_ok_capture_server(1);

            let mut output = Vec::new();
            run_with_env(
                [
                    "brain",
                    "create",
                    brain_id,
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
            assert_eq!(requests.len(), 1);
            let (_, post_body) = requests
                .iter()
                .find(|(request, _)| request.starts_with("POST /_admin/brains "))
                .expect("brain create request captured");
            let post_body: Value = serde_json::from_str(post_body).unwrap();
            assert_eq!(post_body["brainId"], brain_id);
            assert_eq!(post_body["kind"], kind);
            assert_eq!(post_body["name"], name);
            assert!(post_body["bootstrapGrants"].as_array().unwrap().is_empty());
        }
    }

    #[test]
    fn brain_create_includes_authenticated_requester_without_bootstrap_grants() {
        let tmp = TempDir::new().unwrap();
        let requester_keys = Keys::generate();
        import_identity_secret(&tmp, TEST_SECRET_HEX);
        let requester_npub = NostrPublicKey::from_protocol(requester_keys.public_key())
            .to_npub()
            .unwrap();
        let (server_url, server) = start_ok_capture_server(1);

        let mut output = Vec::new();
        run_with_env(
            [
                "brain",
                "create",
                "org-requested",
                "--kind",
                "organization",
                "--name",
                "Requested Org",
                "--requesting-user-npub",
                &requester_npub,
                "--server",
                &server_url,
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();

        let requests = server.join().unwrap();
        let (_, post_body) = requests
            .iter()
            .find(|(request, _)| request.starts_with("POST /_admin/brains "))
            .expect("brain create request captured");
        let post_body: Value = serde_json::from_str(post_body).unwrap();
        assert_eq!(post_body["requestingUserNpub"], requester_npub);
        assert!(post_body["bootstrapGrants"].as_array().unwrap().is_empty());
    }

    #[test]
    fn brain_create_rejects_duplicate_requester_options() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(&tmp, TEST_SECRET_HEX);
        let first_requester = NostrPublicKey::from_protocol(Keys::generate().public_key())
            .to_npub()
            .unwrap();
        let second_requester = NostrPublicKey::from_protocol(Keys::generate().public_key())
            .to_npub()
            .unwrap();
        let mut output = Vec::new();

        let error = run_with_env(
            [
                "brain",
                "create",
                "org-requested",
                "--kind",
                "organization",
                "--requesting-user-npub",
                &first_requester,
                "--requesting-user-npub",
                &second_requester,
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid input: --requesting-user-npub may only be supplied once"
        );
        assert!(output.is_empty());
    }

    #[test]
    fn brain_create_rejects_a_requester_option_without_an_identity() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(&tmp, TEST_SECRET_HEX);
        let mut output = Vec::new();

        let error = run_with_env(
            [
                "brain",
                "create",
                "org-requested",
                "--kind",
                "organization",
                "--requesting-user-npub",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "missing required argument: --requesting-user-npub"
        );
        assert!(output.is_empty());
    }

    #[test]
    fn brain_list_discovers_an_explicitly_paired_personal_brain() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let server_url = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (request_line, _) = read_http_request(&mut stream);
            let body = serde_json::json!({
                "brains": [{
                    "brainId": "personal-user",
                    "kind": "personal",
                    "name": "Personal Brain",
                    "role": "member",
                    "inviteCode": null
                }]
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            request_line
        });
        let mut output = Vec::new();

        run_with_env(
            ["brain", "list", "--server", &server_url, "--json"],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();

        let response: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(response["brains"][0]["brainId"], "personal-user");
        assert_eq!(response["brains"][0]["kind"], "personal");
        assert_eq!(response["brains"][0]["role"], "member");
        assert_eq!(server.join().unwrap(), "GET /_admin/brains HTTP/1.1");
    }

    #[test]
    fn folder_mount_and_access_list_commands_use_typed_metadata() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let (server_url, server) = start_metadata_listing_server(4);

        let mut output = Vec::new();
        run_with_env(
            [
                "folder",
                "list",
                "--brain",
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
                "--brain",
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
                "--brain",
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
        assert_eq!(access["brainId"], "acme");
        assert_eq!(access["grantCount"], 3);
        assert_eq!(access["folders"][0]["currentKeyVersion"], 2);
        assert!(access["folders"][0].get("accessUserIds").is_none());
        assert_eq!(
            access["folders"][0]["explicitAccessUserIds"],
            serde_json::json!([])
        );
        assert_eq!(
            access["folders"][0]["effectiveAccessUserIds"],
            serde_json::json!(["npub-admin", "npub-member"])
        );
        assert_eq!(
            access["folders"][1]["effectiveAccessUserIds"],
            serde_json::json!(["npub-admin"])
        );
        assert_eq!(
            access["folders"][2]["explicitAccessUserIds"],
            serde_json::json!(["npub-member"])
        );
        assert_eq!(
            access["folders"][2]["effectiveAccessUserIds"],
            serde_json::json!(["npub-admin", "npub-member"])
        );
        assert_eq!(access["mountedFolders"][0]["sourceBrainId"], "partner");

        let mut output = Vec::new();
        run_with_env(
            ["access", "list", "--brain", "acme", "--server", &server_url],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();
        let access_text = String::from_utf8(output).unwrap();
        assert!(access_text.contains(
            "folder general path=general access=all_members keyVersion=2 state=ready shared-source explicitAccessUserIds=[] effectiveAccessUserIds=[npub-admin,npub-member]"
        ));
        assert!(access_text.contains(
            "folder leadership path=leadership access=admin_only keyVersion=1 state=ready explicitAccessUserIds=[] effectiveAccessUserIds=[npub-admin]"
        ));
        assert!(access_text.contains(
            "folder project path=project access=restricted keyVersion=1 state=ready explicitAccessUserIds=[npub-member] effectiveAccessUserIds=[npub-admin,npub-member]"
        ));

        let requests = server.join().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.contains("/_admin/brains/acme/metadata"))
                .count(),
            4
        );
    }

    #[test]
    fn access_list_reports_personal_owner_and_personal_agent_as_effective() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let (server_url, server) = start_personal_access_listing_server();
        let mut output = Vec::new();

        run_with_env(
            [
                "access",
                "list",
                "--brain",
                "personal-alice",
                "--server",
                &server_url,
                "--json",
            ],
            env_for(&tmp),
            &mut output,
        )
        .unwrap();

        let access: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            access["folders"][0]["effectiveAccessUserIds"],
            serde_json::json!(["npub-agent", "npub-owner"])
        );
        assert!(access["folders"][0].get("accessUserIds").is_none());
        assert_eq!(
            access["folders"][1]["explicitAccessUserIds"],
            serde_json::json!(["npub-collaborator"])
        );
        assert_eq!(
            access["folders"][1]["effectiveAccessUserIds"],
            serde_json::json!(["npub-agent", "npub-collaborator", "npub-owner"])
        );
        assert_eq!(
            server.join().unwrap(),
            "GET /_admin/brains/personal-alice/metadata HTTP/1.1"
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
                "--brain",
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
                "--brain",
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
            request.starts_with("DELETE /_admin/brains/acme/folders/general/access/npub-target")
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
                "--brain",
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
        assert!(request.starts_with("POST /_admin/brains/acme/invitations"));
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
        initialize_private_working_tree(&tree).unwrap();
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
                "--brain",
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
        assert!(requests[0].0.contains("/_admin/brains/acme/metadata"));
        assert!(requests[1].0.contains("/_admin/brains/acme/export"));
        let (request, body) = &requests[2];
        assert!(request.starts_with("POST /_admin/brains/acme/invitations"));
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
                "--brain",
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
            "POST /_admin/brain-invitation-links/invite-0fe6eda60e1bf6e662acb8e2b5c425d9/accept"
        ));
        assert!(requests[1].0.starts_with(
            "POST /_admin/brains/bavinck-org/invitations/invitation-4f82a37c1b82bcdd54973c466cdde914/accept"
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
        initialize_private_working_tree(&tree).unwrap();
        write_agent_state(&tree, &AgentState::new("acme", "2026-06-24T20:46:36Z")).unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let export_grant =
            export_grant_for_test(&env, "acme", "general", 1, &folder_key, &admin_npub);
        let (server_url, server) =
            start_metadata_export_and_grant_server(admin_npub, vec![export_grant], "granted");
        let mut output = Vec::new();
        run_with_env(
            [
                "share",
                "link",
                "--brain",
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
        initialize_private_working_tree(&tree).unwrap();
        write_agent_state(&tree, &AgentState::new("acme", "2026-06-24T20:46:36Z")).unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let export_grant =
            export_grant_for_test(&env, "acme", "general", 1, &folder_key, &admin_npub);
        let (server_url, server) =
            start_metadata_export_and_grant_server(admin_npub, vec![export_grant], "granted");
        let mut output = Vec::new();
        run_with_env(
            [
                "share",
                "folder-invite",
                "--brain",
                "acme",
                "--folder",
                "general",
                "--destination-brain",
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
    fn daemon_conflicts_activity_and_access_commands_use_agent_state() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
        let roots = BrainWorkingTreeStateManifest {
            version: WORKING_TREE_STATE_VERSION.to_owned(),
            folder_roots: vec![
                WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                },
                WorkingTreeFolderRoot {
                    folder_id: "locked".to_owned(),
                    source_brain_id: None,
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
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
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
        assert!(requests[0].contains("/_admin/brains/brain/export"));
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/brains/brain/sync/records?after=0"))
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
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
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
            export_grant_for_test(&env, "brain", "general", 1, &folder_key, &actor_npub);
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
                .any(|request| request.contains("/_admin/brains/brain/sync/records?after=0"))
        );
    }

    #[test]
    fn pending_working_tree_change_count_detects_local_markdown() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
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
    fn search_returns_ranked_markdown_section_evidence_across_readable_folders() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        fs::create_dir_all(tree.join("Research")).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![
                    WorkingTreeFolderRoot {
                        folder_id: "z-general".to_owned(),
                        source_brain_id: None,
                        path: "General".to_owned(),
                        can_read: true,
                        metadata_only: false,
                    },
                    WorkingTreeFolderRoot {
                        folder_id: "a-research".to_owned(),
                        source_brain_id: None,
                        path: "Research".to_owned(),
                        can_read: true,
                        metadata_only: false,
                    },
                ],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        fs::write(
            tree.join("General/authentication.md"),
            "# Authentication\n\n## Quartz protocol\n\nRotate the quartz token after a device is removed.\n",
        )
        .unwrap();
        fs::write(
            tree.join("Research/minerals.md"),
            "# Mineral notes\n\nQuartz is a crystalline mineral used in timing hardware.\n",
        )
        .unwrap();
        fs::write(
            tree.join("General/OPS-142.md"),
            "# Recovery Runbook\n\nRoutine service recovery procedure.\n",
        )
        .unwrap();

        let mut env = env_for(&tmp);
        env.cwd = tree.join("General");
        let mut output = Vec::new();
        run_with_env(
            ["search", "quartz token", "--json"],
            env.clone(),
            &mut output,
        )
        .unwrap();

        let report: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(report["query"], "quartz token");
        assert_eq!(report["mode"], "lexical");
        assert_eq!(
            report["searchedFolders"],
            serde_json::json!(["a-research", "z-general"])
        );
        assert_eq!(report["results"][0]["rank"], 1);
        assert_eq!(report["results"][0]["folderId"], "z-general");
        assert_eq!(report["results"][0]["pagePath"], "authentication.md");
        assert_eq!(report["results"][0]["pageTitle"], "Authentication");
        assert_eq!(
            report["results"][0]["headingAncestry"],
            serde_json::json!(["Authentication", "Quartz protocol"])
        );
        assert_eq!(report["results"][0]["disposition"], "local_only");
        assert_eq!(
            report["results"][0]["signals"],
            serde_json::json!(["lexical"])
        );

        let mut human = Vec::new();
        run_with_env(["search", "quartz token"], env.clone(), &mut human).unwrap();
        let human = String::from_utf8(human).unwrap();
        assert!(human.contains("General/authentication.md > Quartz protocol"));
        assert!(human.contains("[local_only; lexical]"));
        assert!(human.contains("Rotate the quartz token"));

        let mut filename_output = Vec::new();
        run_with_env(["search", "OPS-142", "--json"], env, &mut filename_output).unwrap();
        let filename_report: Value = serde_json::from_slice(&filename_output).unwrap();
        assert_eq!(filename_report["results"][0]["pagePath"], "OPS-142.md");

        fs::write(
            tree.join("General/plain.md"),
            "Saffron evidence without a heading.\n",
        )
        .unwrap();
        for index in 0..55 {
            fs::write(
                tree.join(format!("General/bulk-{index:02}.md")),
                format!("# Bulk {index}\n\nBulkneedle evidence {index}.\n"),
            )
            .unwrap();
        }
        let mut root_env = env_for(&tmp);
        root_env.cwd = tree.clone();
        let mut scoped_output = Vec::new();
        run_with_env(
            [
                "search",
                "quartz token",
                "--folder",
                "Research",
                "--folder",
                "General",
                "--json",
            ],
            root_env.clone(),
            &mut scoped_output,
        )
        .unwrap();
        let scoped: Value = serde_json::from_slice(&scoped_output).unwrap();
        assert_eq!(scoped["searchedFolders"], report["searchedFolders"]);
        assert_eq!(scoped["results"][0]["pagePath"], "authentication.md");

        let mut plain_output = Vec::new();
        run_with_env(
            ["search", "saffron", "--json"],
            root_env.clone(),
            &mut plain_output,
        )
        .unwrap();
        let plain: Value = serde_json::from_slice(&plain_output).unwrap();
        assert_eq!(plain["results"][0]["pageTitle"], "plain");
        assert!(plain["results"][0]["heading"].is_null());

        let mut default_output = Vec::new();
        run_with_env(
            ["search", "bulkneedle", "--json"],
            root_env.clone(),
            &mut default_output,
        )
        .unwrap();
        let default_report: Value = serde_json::from_slice(&default_output).unwrap();
        assert_eq!(default_report["results"].as_array().unwrap().len(), 10);

        let mut fifty_output = Vec::new();
        run_with_env(
            ["search", "bulkneedle", "--limit", "50", "--json"],
            root_env,
            &mut fifty_output,
        )
        .unwrap();
        let fifty_report: Value = serde_json::from_slice(&fifty_output).unwrap();
        assert_eq!(fifty_report["results"].as_array().unwrap().len(), 50);
    }

    #[test]
    fn hybrid_search_cli_fuses_signals_and_controls_folder_semantics() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        fs::write(
            tree.join("General/00-both.md"),
            "# Inspection\n\nVehicle inspection for an automobile.\n",
        )
        .unwrap();
        fs::write(
            tree.join("General/01-semantic.md"),
            "# Maintenance\n\nAutomobile upkeep schedule.\n",
        )
        .unwrap();
        fs::write(
            tree.join("General/02-lexical.md"),
            "# Benefits\n\nVehicle benefits policy.\n",
        )
        .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::<Value>::new();
            while requests.len() < 5 && started.elapsed() < Duration::from_secs(10) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (_, headers, body) = read_http_request_with_headers(&mut stream);
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("authorization: bearer hybrid-test-token")
                );
                let request: Value = serde_json::from_str(&body).unwrap();
                let vectors = request["inputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|input| {
                        let text = input["text"].as_str().unwrap().to_ascii_lowercase();
                        let embedding = if input["kind"] == "query" || text.contains("automobile") {
                            serde_json::json!([1.0, 0.0])
                        } else {
                            serde_json::json!([0.0, 1.0])
                        };
                        serde_json::json!({
                            "id": input["id"],
                            "embedding": embedding
                        })
                    })
                    .collect::<Vec<_>>();
                let response_body = serde_json::json!({
                    "model": "embed-test",
                    "modelVersion": "v1",
                    "dimensions": 2,
                    "vectors": vectors
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                requests.push(request);
            }
            requests
        });

        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        env.embedding_provider = Some(EmbeddingProviderConfig {
            endpoint,
            bearer_token: "hybrid-test-token".to_owned(),
            timeout: Duration::from_secs(2),
        });

        let mut status_output = Vec::new();
        run_with_env(
            ["search-index", "status", "--json"],
            env.clone(),
            &mut status_output,
        )
        .unwrap();
        let status: Value = serde_json::from_slice(&status_output).unwrap();
        assert_eq!(status["folders"][0]["enabled"], true);
        assert_eq!(status["folders"][0]["lifecycle"], "building");
        assert_eq!(status["folders"][0]["currentSections"], 3);

        let mut initial_output = Vec::new();
        run_with_env(
            ["search", "vehicle care", "--json"],
            env.clone(),
            &mut initial_output,
        )
        .unwrap();
        let initial: Value = serde_json::from_slice(&initial_output).unwrap();
        assert_eq!(initial["mode"], "lexical");

        run_with_env(
            ["daemon", "watch", "--once", "--json"],
            env.clone(),
            &mut Vec::new(),
        )
        .unwrap();

        let mut search_output = Vec::new();
        run_with_env(
            ["search", "vehicle care", "--json"],
            env.clone(),
            &mut search_output,
        )
        .unwrap();
        let report: Value = serde_json::from_slice(&search_output).unwrap();
        assert_eq!(report["mode"], "hybrid");
        assert_eq!(report["results"][0]["pagePath"], "00-both.md");
        assert_eq!(
            report["results"][0]["signals"],
            serde_json::json!(["lexical", "semantic"])
        );
        assert!(report["results"].as_array().unwrap().iter().any(|result| {
            result["pagePath"] == "01-semantic.md"
                && result["signals"] == serde_json::json!(["semantic"])
        }));
        assert!(report["results"].as_array().unwrap().iter().any(|result| {
            result["pagePath"] == "02-lexical.md"
                && result["signals"] == serde_json::json!(["lexical"])
        }));

        let lexical_markdown = fs::read_to_string(tree.join("General/02-lexical.md")).unwrap();
        let mut tree_state = read_working_tree_state(&tree).unwrap();
        tree_state.objects.push(WorkingTreeObjectManifestEntry {
            folder_id: "general".to_owned(),
            source_brain_id: None,
            path: "02-lexical.md".to_owned(),
            object_id: "obj_lexical0001".to_owned(),
            revision: 1,
            key_version: 1,
            content_type: "text/markdown".to_owned(),
            content_hash: finite_brain_core::sha256_hex(lexical_markdown.as_bytes()),
        });
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &tree_state,
        )
        .unwrap();
        reconcile_search_indexes(&tree).unwrap();
        let disposition_only =
            refresh_semantic_indexes(&tree, env.embedding_provider.as_ref().unwrap()).unwrap();
        assert_eq!(disposition_only.rebuilt_folders, 0);

        fs::write(
            tree.join("General/00-both.md"),
            "# Inspection\n\nVehicle inspection changed for an automobile.\n",
        )
        .unwrap();
        let mut stale_output = Vec::new();
        run_with_env(
            ["search", "vehicle care", "--json"],
            env.clone(),
            &mut stale_output,
        )
        .unwrap();
        let stale: Value = serde_json::from_slice(&stale_output).unwrap();
        assert!(stale["results"].as_array().unwrap().iter().any(|result| {
            result["pagePath"] == "00-both.md"
                && result["signals"] == serde_json::json!(["lexical"])
        }));
        assert!(stale["results"].as_array().unwrap().iter().any(|result| {
            result["pagePath"] == "01-semantic.md"
                && result["signals"] == serde_json::json!(["semantic"])
        }));
        let mut stale_status_output = Vec::new();
        run_with_env(
            ["search-index", "status", "--json"],
            env.clone(),
            &mut stale_status_output,
        )
        .unwrap();
        let stale_status: Value = serde_json::from_slice(&stale_status_output).unwrap();
        assert_eq!(stale_status["folders"][0]["lifecycle"], "stale");
        assert_eq!(stale_status["folders"][0]["currentVectors"], 2);

        run_with_env(
            ["daemon", "watch", "--once", "--json"],
            env.clone(),
            &mut Vec::new(),
        )
        .unwrap();
        let mut refreshed_output = Vec::new();
        run_with_env(
            ["search", "vehicle care", "--json"],
            env.clone(),
            &mut refreshed_output,
        )
        .unwrap();
        let refreshed: Value = serde_json::from_slice(&refreshed_output).unwrap();
        assert_eq!(
            refreshed["results"][0]["signals"],
            serde_json::json!(["lexical", "semantic"])
        );

        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 5);
        let section_requests = requests
            .iter()
            .filter(|request| request["inputs"][0]["kind"] == "section")
            .collect::<Vec<_>>();
        assert_eq!(section_requests.len(), 2);
        assert_eq!(section_requests[0]["inputs"].as_array().unwrap().len(), 3);
        assert_eq!(section_requests[1]["inputs"].as_array().unwrap().len(), 1);
        let section_request = section_requests[0];
        let serialized = section_request.to_string();
        for forbidden in [
            "general",
            "00-both.md",
            "brain",
            "hybrid-test-token",
            "revision",
            "grant",
        ] {
            assert!(!serialized.contains(forbidden));
        }

        let index_file = fs::read_dir(tree.join(".finitebrain/search-indexes"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join("index.sqlite3");
        let index_bytes = fs::read(&index_file).unwrap();
        assert!(
            !index_bytes
                .windows("hybrid-test-token".len())
                .any(|window| window == b"hybrid-test-token")
        );

        let mismatch_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mismatch_endpoint = format!("http://{}", mismatch_listener.local_addr().unwrap());
        let mismatch_server = thread::spawn(move || {
            let (mut stream, _) = mismatch_listener.accept().unwrap();
            let (_, _, body) = read_http_request_with_headers(&mut stream);
            let request: Value = serde_json::from_str(&body).unwrap();
            let vectors = request["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| {
                    serde_json::json!({
                        "id": input["id"],
                        "embedding": [1.0, 0.0]
                    })
                })
                .collect::<Vec<_>>();
            let response_body = serde_json::json!({
                "model": "embed-test",
                "modelVersion": "v2",
                "dimensions": 2,
                "vectors": vectors
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        env.embedding_provider.as_mut().unwrap().endpoint = mismatch_endpoint;
        let mut mismatched_output = Vec::new();
        run_with_env(
            ["search", "vehicle care", "--json"],
            env.clone(),
            &mut mismatched_output,
        )
        .unwrap();
        mismatch_server.join().unwrap();
        let mismatched: Value = serde_json::from_slice(&mismatched_output).unwrap();
        assert_eq!(mismatched["mode"], "lexical");
        let mut mismatched_status_output = Vec::new();
        run_with_env(
            ["search-index", "status", "--json"],
            env.clone(),
            &mut mismatched_status_output,
        )
        .unwrap();
        let mismatched_status: Value = serde_json::from_slice(&mismatched_status_output).unwrap();
        assert_eq!(mismatched_status["folders"][0]["lifecycle"], "stale");

        rusqlite::Connection::open(&index_file)
            .unwrap()
            .execute(
                "UPDATE semantic_vectors SET vector = X'00' WHERE rowid = (
                    SELECT rowid FROM semantic_vectors LIMIT 1
                 )",
                [],
            )
            .unwrap();
        let corrupt_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let corrupt_endpoint = format!("http://{}", corrupt_listener.local_addr().unwrap());
        let corrupt_server = thread::spawn(move || {
            let (mut stream, _) = corrupt_listener.accept().unwrap();
            let (_, _, body) = read_http_request_with_headers(&mut stream);
            let request: Value = serde_json::from_str(&body).unwrap();
            let vectors = request["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| {
                    serde_json::json!({
                        "id": input["id"],
                        "embedding": [1.0, 0.0]
                    })
                })
                .collect::<Vec<_>>();
            let response_body = serde_json::json!({
                "model": "embed-test",
                "modelVersion": "v1",
                "dimensions": 2,
                "vectors": vectors
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        env.embedding_provider.as_mut().unwrap().endpoint = corrupt_endpoint;
        let mut corrupt_output = Vec::new();
        run_with_env(
            ["search", "vehicle care", "--json"],
            env.clone(),
            &mut corrupt_output,
        )
        .unwrap();
        corrupt_server.join().unwrap();
        let corrupt: Value = serde_json::from_slice(&corrupt_output).unwrap();
        assert_eq!(corrupt["mode"], "lexical");
        let mut corrupt_status_output = Vec::new();
        run_with_env(
            ["search-index", "status", "--json"],
            env.clone(),
            &mut corrupt_status_output,
        )
        .unwrap();
        let corrupt_status: Value = serde_json::from_slice(&corrupt_status_output).unwrap();
        assert_eq!(corrupt_status["folders"][0]["lifecycle"], "failed");

        let mut lexical_output = Vec::new();
        run_with_env(
            ["search", "vehicle care", "--lexical-only", "--json"],
            env.clone(),
            &mut lexical_output,
        )
        .unwrap();
        let lexical: Value = serde_json::from_slice(&lexical_output).unwrap();
        assert_eq!(lexical["mode"], "lexical");
        assert!(
            lexical["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|result| { result["signals"] == serde_json::json!(["lexical"]) })
        );

        let mut unavailable_output = Vec::new();
        run_with_env(
            ["search", "vehicle care", "--json"],
            env.clone(),
            &mut unavailable_output,
        )
        .unwrap();
        let unavailable: Value = serde_json::from_slice(&unavailable_output).unwrap();
        assert_eq!(unavailable["mode"], "lexical");

        let mut disable_output = Vec::new();
        run_with_env(
            ["search-index", "disable", "--folder", "general", "--json"],
            env.clone(),
            &mut disable_output,
        )
        .unwrap();
        let disabled: Value = serde_json::from_slice(&disable_output).unwrap();
        assert_eq!(disabled["folders"][0]["enabled"], false);
        assert_eq!(disabled["folders"][0]["lifecycle"], "disabled");
        assert_eq!(disabled["folders"][0]["currentVectors"], 0);

        let mut enable_output = Vec::new();
        run_with_env(
            ["search-index", "enable", "--folder", "general", "--json"],
            env.clone(),
            &mut enable_output,
        )
        .unwrap();
        let enabled: Value = serde_json::from_slice(&enable_output).unwrap();
        assert_eq!(enabled["folders"][0]["enabled"], true);
        assert_eq!(enabled["folders"][0]["lifecycle"], "building");
        assert_eq!(enabled["folders"][0]["currentVectors"], 0);

        let rebuild_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let rebuild_endpoint = format!("http://{}", rebuild_listener.local_addr().unwrap());
        let rebuild_server = thread::spawn(move || {
            let (mut stream, _) = rebuild_listener.accept().unwrap();
            let (_, _, body) = read_http_request_with_headers(&mut stream);
            let request: Value = serde_json::from_str(&body).unwrap();
            let vectors = request["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| {
                    serde_json::json!({
                        "id": input["id"],
                        "embedding": [1.0, 0.0]
                    })
                })
                .collect::<Vec<_>>();
            let response_body = serde_json::json!({
                "model": "embed-test",
                "modelVersion": "v3",
                "dimensions": 2,
                "vectors": vectors
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        env.embedding_provider.as_mut().unwrap().endpoint = rebuild_endpoint;
        run_with_env(
            ["daemon", "watch", "--once", "--json"],
            env.clone(),
            &mut Vec::new(),
        )
        .unwrap();
        rebuild_server.join().unwrap();
        let mut rebuilt_status_output = Vec::new();
        run_with_env(
            ["search-index", "status", "--json"],
            env.clone(),
            &mut rebuilt_status_output,
        )
        .unwrap();
        let rebuilt_status: Value = serde_json::from_slice(&rebuilt_status_output).unwrap();
        assert_eq!(rebuilt_status["folders"][0]["lifecycle"], "ready");
        assert_eq!(rebuilt_status["folders"][0]["currentVectors"], 3);

        let rate_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let rate_endpoint = format!("http://{}", rate_listener.local_addr().unwrap());
        let rate_server = thread::spawn(move || {
            let (mut stream, _) = rate_listener.accept().unwrap();
            let _ = read_http_request_with_headers(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        env.embedding_provider.as_mut().unwrap().endpoint = rate_endpoint;
        let mut rate_output = Vec::new();
        run_with_env(
            ["search", "vehicle care", "--json"],
            env.clone(),
            &mut rate_output,
        )
        .unwrap();
        rate_server.join().unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&rate_output).unwrap()["mode"],
            "lexical"
        );

        let timeout_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let timeout_endpoint = format!("http://{}", timeout_listener.local_addr().unwrap());
        let timeout_server = thread::spawn(move || {
            let (mut stream, _) = timeout_listener.accept().unwrap();
            let _ = read_http_request_with_headers(&mut stream);
            thread::sleep(Duration::from_millis(150));
        });
        let provider = env.embedding_provider.as_mut().unwrap();
        provider.endpoint = timeout_endpoint;
        provider.timeout = Duration::from_millis(50);
        let mut timeout_output = Vec::new();
        run_with_env(
            ["search", "vehicle care", "--json"],
            env.clone(),
            &mut timeout_output,
        )
        .unwrap();
        timeout_server.join().unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&timeout_output).unwrap()["mode"],
            "lexical"
        );

        let malformed_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let malformed_endpoint = format!("http://{}", malformed_listener.local_addr().unwrap());
        let malformed_server = thread::spawn(move || {
            let (mut stream, _) = malformed_listener.accept().unwrap();
            let (_, _, body) = read_http_request_with_headers(&mut stream);
            let request: Value = serde_json::from_str(&body).unwrap();
            let id = request["inputs"][0]["id"].clone();
            let response_body = serde_json::json!({
                "model": "embed-test",
                "modelVersion": "v3",
                "dimensions": 2,
                "vectors": [
                    { "id": id, "embedding": [1.0, 0.0] },
                    { "id": id, "embedding": [1.0, 0.0] }
                ]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let provider = env.embedding_provider.as_mut().unwrap();
        provider.endpoint = malformed_endpoint;
        provider.timeout = Duration::from_secs(2);
        let mut malformed_output = Vec::new();
        run_with_env(
            ["search", "vehicle care", "--json"],
            env,
            &mut malformed_output,
        )
        .unwrap();
        malformed_server.join().unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&malformed_output).unwrap()["mode"],
            "lexical"
        );
    }

    #[test]
    fn semantic_generation_stops_after_disable_or_access_loss() {
        fn run_case(revoke_access: bool) {
            let tmp = TempDir::new().unwrap();
            let tree = tmp.path().join("brain");
            initialize_private_working_tree(&tree).unwrap();
            fs::create_dir_all(tree.join("General")).unwrap();
            write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
            let mut tree_state = BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            };
            write_json_file(
                &tree.join(".finitebrain/working-tree-state.json"),
                &tree_state,
            )
            .unwrap();
            for index in 0..65 {
                fs::write(
                    tree.join(format!("General/page-{index:02}.md")),
                    format!("# Page {index}\n\nSemantic cancellation evidence {index}.\n"),
                )
                .unwrap();
            }
            reconcile_search_indexes(&tree).unwrap();
            let index_path = fs::read_dir(tree.join(".finitebrain/search-indexes"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path()
                .join("index.sqlite3");

            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let (_, _, body) = read_http_request_with_headers(&mut stream);
                let request: Value = serde_json::from_str(&body).unwrap();
                assert_eq!(request["inputs"].as_array().unwrap().len(), 64);
                accepted_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                let vectors = request["inputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|input| {
                        serde_json::json!({
                            "id": input["id"],
                            "embedding": [1.0, 0.0]
                        })
                    })
                    .collect::<Vec<_>>();
                let response_body = serde_json::json!({
                    "model": "cancel-test",
                    "modelVersion": "v1",
                    "dimensions": 2,
                    "vectors": vectors
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                listener.set_nonblocking(true).unwrap();
                let started = Instant::now();
                let mut requests = 1;
                while started.elapsed() < Duration::from_millis(300) {
                    match listener.accept() {
                        Ok((_stream, _)) => requests += 1,
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("cancellation listener failed: {error}"),
                    }
                }
                requests
            });
            let config = EmbeddingProviderConfig {
                endpoint,
                bearer_token: "cancel-token".to_owned(),
                timeout: Duration::from_secs(2),
            };
            let worker_tree = tree.clone();
            let worker_config = config.clone();
            let worker = thread::spawn(move || {
                refresh_semantic_indexes(&worker_tree, &worker_config).unwrap()
            });
            accepted_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            let (control_done_tx, control_done_rx) = std::sync::mpsc::channel();
            let (published_tx, published_rx) = std::sync::mpsc::channel();
            let (continue_tx, continue_rx) = std::sync::mpsc::channel();
            let control_tree = tree.clone();
            let mut control_env = env_for(&tmp);
            control_env.cwd = control_tree.clone();
            control_env.embedding_provider = Some(config.clone());
            let control = thread::spawn(move || {
                if revoke_access {
                    tree_state.folder_roots[0].can_read = false;
                    tree_state.folder_roots[0].metadata_only = true;
                    write_working_tree_state(&control_tree, &tree_state).unwrap();
                    published_tx.send(()).unwrap();
                    continue_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                    reconcile_search_indexes(&control_tree).unwrap();
                } else {
                    run_with_env(
                        ["search-index", "disable", "--folder", "general", "--json"],
                        control_env,
                        &mut Vec::new(),
                    )
                    .unwrap();
                }
                control_done_tx.send(()).unwrap();
            });
            if revoke_access {
                published_rx
                    .recv_timeout(Duration::from_millis(500))
                    .expect("access publication must not wait for remote embedding I/O");
                let published = read_working_tree_state(&tree).unwrap();
                assert!(!published.folder_roots[0].can_read);
                assert!(index_path.parent().unwrap().exists());
                let connection = rusqlite::Connection::open(&index_path).unwrap();
                let (enabled, vectors): (bool, usize) = connection
                    .query_row(
                        "SELECT enabled, (SELECT count(*) FROM semantic_vectors)
                           FROM semantic_settings WHERE singleton = 1",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                assert!(!enabled);
                assert_eq!(vectors, 0);
            } else {
                control_done_rx
                    .recv_timeout(Duration::from_millis(500))
                    .expect("semantic control must not wait for remote embedding I/O");
                let connection = rusqlite::Connection::open(&index_path).unwrap();
                let (enabled, vectors): (bool, usize) = connection
                    .query_row(
                        "SELECT enabled, (SELECT count(*) FROM semantic_vectors)
                           FROM semantic_settings WHERE singleton = 1",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                assert!(!enabled);
                assert_eq!(vectors, 0);
            }
            release_tx.send(()).unwrap();
            let refresh = worker.join().unwrap();
            assert_eq!(refresh.rebuilt_folders, 0);
            assert_eq!(server.join().unwrap(), 1);
            if revoke_access {
                continue_tx.send(()).unwrap();
                control_done_rx
                    .recv_timeout(Duration::from_millis(500))
                    .unwrap();
                control.join().unwrap();
                assert!(!index_path.parent().unwrap().exists());
            } else {
                control.join().unwrap();
                let connection = rusqlite::Connection::open(&index_path).unwrap();
                let vectors: usize = connection
                    .query_row("SELECT count(*) FROM semantic_vectors", [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(vectors, 0);
            }
        }

        run_case(false);
        run_case(true);
    }

    #[test]
    fn semantic_generation_does_not_activate_an_outdated_section_snapshot() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        fs::write(
            tree.join("General/original.md"),
            "# Original\n\nInitial semantic content.\n",
        )
        .unwrap();
        reconcile_search_indexes(&tree).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (_, _, body) = read_http_request_with_headers(&mut stream);
            let request: Value = serde_json::from_str(&body).unwrap();
            accepted_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            let vectors = request["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|input| {
                    serde_json::json!({
                        "id": input["id"],
                        "embedding": [1.0, 0.0]
                    })
                })
                .collect::<Vec<_>>();
            let response_body = serde_json::json!({
                "model": "snapshot-test",
                "modelVersion": "v1",
                "dimensions": 2,
                "vectors": vectors
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let config = EmbeddingProviderConfig {
            endpoint,
            bearer_token: "snapshot-token".to_owned(),
            timeout: Duration::from_secs(2),
        };
        let worker_tree = tree.clone();
        let worker_config = config.clone();
        let worker =
            thread::spawn(move || refresh_semantic_indexes(&worker_tree, &worker_config).unwrap());
        accepted_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        fs::write(
            tree.join("General/arrived-during-build.md"),
            "# New current section\n\nThis must be embedded before activation.\n",
        )
        .unwrap();
        reconcile_search_indexes(&tree).unwrap();
        release_tx.send(()).unwrap();
        let refresh = worker.join().unwrap();
        server.join().unwrap();
        assert_eq!(refresh.rebuilt_folders, 0);

        let mut env = env_for(&tmp);
        env.cwd = tree;
        env.embedding_provider = Some(config);
        let mut status_output = Vec::new();
        run_with_env(
            ["search-index", "status", "--json"],
            env,
            &mut status_output,
        )
        .unwrap();
        let status: Value = serde_json::from_slice(&status_output).unwrap();
        assert_eq!(status["folders"][0]["lifecycle"], "building");
        assert_eq!(status["folders"][0]["currentSections"], 2);
        assert_eq!(status["folders"][0]["currentVectors"], 0);
    }

    #[test]
    #[ignore = "subprocess helper invoked by semantic_generation_uses_one_cross_process_folder_lease"]
    fn semantic_refresh_process_helper() {
        let Ok(tree) = std::env::var("FBRAIN_TEST_SEMANTIC_TREE") else {
            return;
        };
        let config = EmbeddingProviderConfig {
            endpoint: std::env::var("FBRAIN_TEST_EMBEDDING_ENDPOINT").unwrap(),
            bearer_token: std::env::var("FBRAIN_TEST_EMBEDDING_TOKEN").unwrap(),
            timeout: Duration::from_secs(2),
        };
        let report = refresh_semantic_indexes(Path::new(&tree), &config).unwrap();
        fs::write(
            std::env::var("FBRAIN_TEST_SEMANTIC_REPORT").unwrap(),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
    }

    #[test]
    #[ignore = "subprocess helper invoked by semantic_generation_uses_one_cross_process_folder_lease"]
    fn semantic_build_lock_process_helper() {
        let Ok(lock_path) = std::env::var("FBRAIN_TEST_BUILD_LOCK") else {
            return;
        };
        let lock = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .unwrap();
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
        fs::write(
            std::env::var("FBRAIN_TEST_BUILD_LOCK_READY").unwrap(),
            b"ready",
        )
        .unwrap();
        thread::sleep(Duration::from_secs(30));
    }

    #[test]
    fn semantic_generation_uses_one_cross_process_folder_lease() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        fs::write(
            tree.join("General/lease.md"),
            "# Lease\n\nOnly one semantic builder may submit this section.\n",
        )
        .unwrap();
        reconcile_search_indexes(&tree).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (_, _, body) = read_http_request_with_headers(&mut stream);
            let request: Value = serde_json::from_str(&body).unwrap();
            accepted_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            let response_body = serde_json::json!({
                "model": "lease-test",
                "modelVersion": "v1",
                "dimensions": 2,
                "vectors": [{
                    "id": request["inputs"][0]["id"],
                    "embedding": [1.0, 0.0]
                }]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            listener.set_nonblocking(true).unwrap();
            let started = Instant::now();
            let mut requests = 1;
            while started.elapsed() < Duration::from_millis(300) {
                match listener.accept() {
                    Ok((_stream, _)) => requests += 1,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("lease listener failed: {error}"),
                }
            }
            requests
        });
        let config = EmbeddingProviderConfig {
            endpoint,
            bearer_token: "lease-token".to_owned(),
            timeout: Duration::from_secs(2),
        };
        let first_report_path = tmp.path().join("first-refresh.json");
        let second_report_path = tmp.path().join("second-refresh.json");
        let mut first = spawn_semantic_refresh_process(&tree, &config, &first_report_path);
        accepted_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let mut second = spawn_semantic_refresh_process(&tree, &config, &second_report_path);
        thread::sleep(Duration::from_millis(50));
        assert!(second.try_wait().unwrap().is_none());
        release_tx.send(()).unwrap();
        assert!(first.wait().unwrap().success());
        assert!(second.wait().unwrap().success());
        let first_report = read_semantic_refresh_report(&first_report_path);
        let second_report = read_semantic_refresh_report(&second_report_path);
        assert_eq!(
            first_report.rebuilt_folders + second_report.rebuilt_folders,
            1
        );
        assert_eq!(server.join().unwrap(), 1);

        let index_directory = fs::read_dir(tree.join(".finitebrain/search-indexes"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let ready_path = tmp.path().join("build-lock-ready");
        let mut holder = semantic_test_process("tests::semantic_build_lock_process_helper")
            .env(
                "FBRAIN_TEST_BUILD_LOCK",
                index_directory.join("semantic-build.lock"),
            )
            .env("FBRAIN_TEST_BUILD_LOCK_READY", &ready_path)
            .spawn()
            .unwrap();
        let started = Instant::now();
        while !ready_path.exists() && started.elapsed() < Duration::from_secs(3) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(ready_path.exists());

        let recovery_report_path = tmp.path().join("recovery-refresh.json");
        let mut recovery = spawn_semantic_refresh_process(&tree, &config, &recovery_report_path);
        thread::sleep(Duration::from_millis(50));
        assert!(recovery.try_wait().unwrap().is_none());
        holder.kill().unwrap();
        holder.wait().unwrap();
        assert!(recovery.wait().unwrap().success());
        assert_eq!(
            read_semantic_refresh_report(&recovery_report_path).rebuilt_folders,
            0
        );
    }

    #[test]
    fn hybrid_search_beta_fixture_records_quality_and_latency_baseline() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../evaluations/hybrid-wiki-search-beta.json"
        ))
        .unwrap();
        assert_eq!(
            fixture["version"],
            "finitebrain-hybrid-search-evaluation-v1"
        );
        let documents = fixture["documents"].as_array().unwrap();
        let queries = fixture["queries"].as_array().unwrap();
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("Wiki")).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "wiki".to_owned(),
                    source_brain_id: None,
                    path: "Wiki".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        for document in documents {
            let path = tree.join("Wiki").join(document["path"].as_str().unwrap());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                path,
                format!(
                    "# {}\n\n## {}\n\n{}\n\n## Appendix\n\nUnrelated appendix marker.\n",
                    document["title"].as_str().unwrap(),
                    document["heading"].as_str().unwrap(),
                    document["text"].as_str().unwrap()
                ),
            )
            .unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let request_count = queries.len() + 1;
        let server = thread::spawn(move || {
            let started = Instant::now();
            let mut handled = 0;
            while handled < request_count && started.elapsed() < Duration::from_secs(10) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let (_, _, body) = read_http_request_with_headers(&mut stream);
                let request: Value = serde_json::from_str(&body).unwrap();
                let vectors = request["inputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|input| {
                        let searchable = format!(
                            "{} {}",
                            input["pageTitle"].as_str().unwrap_or_default(),
                            input["text"].as_str().unwrap()
                        )
                        .to_ascii_lowercase();
                        let class = if searchable.contains("laptop")
                            || searchable.contains("session tokens")
                        {
                            0
                        } else if searchable.contains("brand new host")
                            || searchable.contains("empty machine")
                        {
                            1
                        } else if searchable.contains("coordinating an outage")
                            || searchable.contains("incident commander")
                        {
                            2
                        } else if searchable.contains("trial users")
                            || searchable.contains("beta participants")
                        {
                            3
                        } else if searchable.contains("outside collaboration")
                            || searchable.contains("partner")
                        {
                            4
                        } else if searchable.contains("vector retrieval")
                            || searchable.contains("agents still search")
                        {
                            5
                        } else {
                            6
                        };
                        let mut embedding = vec![0.0; 6];
                        if class < embedding.len() {
                            embedding[class] = 1.0;
                        }
                        serde_json::json!({
                            "id": input["id"],
                            "embedding": embedding
                        })
                    })
                    .collect::<Vec<_>>();
                let response_body = serde_json::json!({
                    "model": "beta-fixture",
                    "modelVersion": "v1",
                    "dimensions": 6,
                    "vectors": vectors
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                handled += 1;
            }
            handled
        });

        let mut env = env_for(&tmp);
        env.cwd = tree;
        env.embedding_provider = Some(EmbeddingProviderConfig {
            endpoint,
            bearer_token: "evaluation-token".to_owned(),
            timeout: Duration::from_secs(2),
        });
        run_with_env(
            ["daemon", "watch", "--once", "--json"],
            env.clone(),
            &mut Vec::new(),
        )
        .unwrap();

        let mut hybrid_hits = 0;
        let mut lexical_hits = 0;
        let mut hybrid_micros = 0_u128;
        let mut lexical_micros = 0_u128;
        for query in queries {
            let query_text = query["query"].as_str().unwrap();
            let expected = query["expectedPage"].as_str().unwrap();
            let expected_heading = query["expectedHeading"].as_str().unwrap();
            let started = Instant::now();
            let mut hybrid_output = Vec::new();
            run_with_env(
                ["search", query_text, "--limit", "5", "--json"],
                env.clone(),
                &mut hybrid_output,
            )
            .unwrap();
            hybrid_micros += started.elapsed().as_micros();
            let hybrid: Value = serde_json::from_slice(&hybrid_output).unwrap();
            hybrid_hits +=
                usize::from(hybrid["results"].as_array().unwrap().iter().any(|result| {
                    result["pagePath"] == expected && result["heading"] == expected_heading
                }));

            let started = Instant::now();
            let mut lexical_output = Vec::new();
            run_with_env(
                [
                    "search",
                    query_text,
                    "--limit",
                    "5",
                    "--lexical-only",
                    "--json",
                ],
                env.clone(),
                &mut lexical_output,
            )
            .unwrap();
            lexical_micros += started.elapsed().as_micros();
            let lexical: Value = serde_json::from_slice(&lexical_output).unwrap();
            lexical_hits +=
                usize::from(lexical["results"].as_array().unwrap().iter().any(|result| {
                    result["pagePath"] == expected && result["heading"] == expected_heading
                }));
        }
        assert_eq!(server.join().unwrap(), request_count);
        assert_eq!(hybrid_hits, queries.len());
        eprintln!(
            "hybrid-beta-baseline queries={} hybridRecallAt5={:.3} lexicalRecallAt5={:.3} hybridMeanMicros={} lexicalMeanMicros={}",
            queries.len(),
            hybrid_hits as f64 / queries.len() as f64,
            lexical_hits as f64 / queries.len() as f64,
            hybrid_micros / queries.len() as u128,
            lexical_micros / queries.len() as u128,
        );
    }

    #[test]
    fn search_indexes_persist_reconcile_saved_edits_and_rebuild_corrupt_state() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        fs::write(
            tree.join("General/runbook.md"),
            "# Runbook\n\n## Stable procedure\n\nKeep the violet recovery note.\n\n## Active procedure\n\nRestart the amber service.\n",
        )
        .unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();

        let mut output = Vec::new();
        run_with_env(["search", "amber", "--json"], env.clone(), &mut output).unwrap();
        let first: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(first["results"][0]["pagePath"], "runbook.md");
        let index_directories = fs::read_dir(tree.join(".finitebrain/search-indexes"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(index_directories.len(), 1);
        let index_path = index_directories[0].join("index.sqlite3");
        let legacy_index = tree
            .join(".finitebrain/search-indexes")
            .join(format!("{}.sqlite3", "a".repeat(64)));
        let legacy_journal = tree
            .join(".finitebrain/search-indexes")
            .join(format!("{}.sqlite3-journal", "a".repeat(64)));
        fs::write(&legacy_index, b"legacy derived plaintext").unwrap();
        fs::write(&legacy_journal, b"legacy rollback plaintext").unwrap();
        set_private_file_permissions(&legacy_index).unwrap();
        set_private_file_permissions(&legacy_journal).unwrap();
        let stable_rowid = rusqlite::Connection::open(&index_path)
            .unwrap()
            .query_row(
                "SELECT rowid FROM sections WHERE heading = 'Stable procedure'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();

        fs::write(
            tree.join("General/runbook.md"),
            "# Runbook\n\n## Stable procedure\n\nKeep the violet recovery note.\n\n## Active procedure\n\nRestart the cobalt service.\n",
        )
        .unwrap();
        let mut output = Vec::new();
        run_with_env(["search", "cobalt", "--json"], env.clone(), &mut output).unwrap();
        let changed: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(changed["results"][0]["pagePath"], "runbook.md");
        assert!(!legacy_index.exists());
        assert!(!legacy_journal.exists());
        let stable_rowid_after_edit = rusqlite::Connection::open(&index_path)
            .unwrap()
            .query_row(
                "SELECT rowid FROM sections WHERE heading = 'Stable procedure'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(
            stable_rowid_after_edit, stable_rowid,
            "an unrelated Section should not be rewritten"
        );
        let mut output = Vec::new();
        run_with_env(["search", "amber", "--json"], env.clone(), &mut output).unwrap();
        let stale: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(stale["results"], serde_json::json!([]));

        fs::write(&index_path, b"not a sqlite database").unwrap();
        let mut output = Vec::new();
        run_with_env(["search", "cobalt", "--json"], env, &mut output).unwrap();
        let rebuilt: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(rebuilt["results"][0]["pagePath"], "runbook.md");

        let connection = rusqlite::Connection::open(&index_path).unwrap();
        connection
            .execute("DELETE FROM metadata WHERE key = 'version'", [])
            .unwrap();
        drop(connection);
        let mut output = Vec::new();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        run_with_env(["search", "cobalt", "--json"], env, &mut output).unwrap();
        let rebuilt_without_version: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            rebuilt_without_version["results"][0]["pagePath"],
            "runbook.md"
        );

        let connection = rusqlite::Connection::open(&index_path).unwrap();
        connection
            .execute_batch("DROP TABLE pages; CREATE TABLE pages (wrong TEXT);")
            .unwrap();
        drop(connection);
        let mut output = Vec::new();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        run_with_env(["search", "cobalt", "--json"], env, &mut output).unwrap();
        let rebuilt_partial_schema: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            rebuilt_partial_schema["results"][0]["pagePath"],
            "runbook.md"
        );

        let journal_residue = index_directories[0].join("index.sqlite3-journal");
        fs::write(&journal_residue, b"synthetic plaintext rollback residue").unwrap();
        set_private_file_permissions(&journal_residue).unwrap();

        let mut state = read_working_tree_state(&tree).unwrap();
        state.folder_roots[0].can_read = false;
        state.folder_roots[0].metadata_only = true;
        write_json_file(&tree.join(".finitebrain/working-tree-state.json"), &state).unwrap();
        let mut output = Vec::new();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        run_with_env(["search", "cobalt", "--json"], env, &mut output).unwrap();
        let after_revocation: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(after_revocation["results"], serde_json::json!([]));
        assert_eq!(
            fs::read_dir(tree.join(".finitebrain/search-indexes"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn hot_sync_search_reconciliation_reads_only_reported_pages() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        fs::write(
            tree.join("General/stable.md"),
            "# Stable\n\nOld stable text.\n",
        )
        .unwrap();
        fs::write(
            tree.join("General/changed.md"),
            "# Changed\n\n## Stable procedure\n\nKeep violet text.\n\n## Active procedure\n\nAmber text.\n",
        )
        .unwrap();
        fs::write(tree.join("General/z-broken.md"), "# Guard\n\nSmall text.\n").unwrap();
        reconcile_search_indexes(&tree).unwrap();
        let index_directory = fs::read_dir(tree.join(".finitebrain/search-indexes"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let index_path = index_directory.join("index.sqlite3");
        let stable_rowid = rusqlite::Connection::open(&index_path)
            .unwrap()
            .query_row(
                "SELECT rowid FROM sections WHERE page_path = 'changed.md' AND heading = 'Stable procedure'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();

        // A full Folder scan would reject this unrelated Page, making this a
        // high-seam assertion that the hot path follows the sync report only.
        fs::write(
            tree.join("General/stable.md"),
            "x".repeat((4 * 1024 * 1024) + 1),
        )
        .unwrap();
        fs::write(
            tree.join("General/changed.md"),
            "# Changed\n\n## Stable procedure\n\nKeep violet text.\n\n## Active procedure\n\nCobalt text.\n",
        )
        .unwrap();
        let report = SyncOnceReport {
            status: "ok".to_owned(),
            latest_sequence: 1,
            record_count: 1,
            server_url: "http://127.0.0.1".to_owned(),
            local_changes: vec![SyncChangeReport {
                status: "submitted".to_owned(),
                action: "update".to_owned(),
                actor_npub: None,
                sequence: None,
                path: Some("General/changed.md".to_owned()),
                from_path: None,
                folder_id: Some("general".to_owned()),
                source_brain_id: None,
                object_id: Some("changed".to_owned()),
                route: "encrypted-object-write".to_owned(),
                reason: None,
            }],
            remote_changes: Vec::new(),
            conflicts: Vec::new(),
        };

        assert_eq!(reconcile_search_changes(&tree, &report).unwrap(), 1);
        let connection = rusqlite::Connection::open(&index_path).unwrap();
        let changed_body: String = connection
            .query_row(
                "SELECT body FROM sections WHERE page_path = 'changed.md' AND heading = 'Active procedure'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(changed_body.contains("Cobalt"));
        let stable_rowid_after = connection
            .query_row(
                "SELECT rowid FROM sections WHERE page_path = 'changed.md' AND heading = 'Stable procedure'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(stable_rowid_after, stable_rowid);

        fs::write(
            tree.join("General/changed.md"),
            "# Changed\n\n## Stable procedure\n\nKeep violet text.\n\n## Active procedure\n\nEmerald text.\n",
        )
        .unwrap();
        fs::write(
            tree.join("General/z-broken.md"),
            "x".repeat((4 * 1024 * 1024) + 1),
        )
        .unwrap();
        let mut failing_report = report.clone();
        let mut broken_change = failing_report.local_changes[0].clone();
        broken_change.path = Some("General/z-broken.md".to_owned());
        failing_report.local_changes.push(broken_change);
        assert!(reconcile_search_changes(&tree, &failing_report).is_err());
        let body_after_failed_transaction: String = connection
            .query_row(
                "SELECT body FROM sections WHERE page_path = 'changed.md' AND heading = 'Active procedure'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(body_after_failed_transaction.contains("Cobalt"));
        assert!(!body_after_failed_transaction.contains("Emerald"));
        drop(connection);

        fs::write(
            tree.join("General/stable.md"),
            "# Stable\n\nRestored stable text.\n",
        )
        .unwrap();
        fs::write(
            tree.join("General/z-broken.md"),
            "# Guard\n\nSmall again.\n",
        )
        .unwrap();
        fs::write(
            tree.join("General/changed.md"),
            "# Changed\n\n## Active procedure\n\nSilver text.\n",
        )
        .unwrap();
        fs::write(&index_path, b"not a sqlite database").unwrap();
        reconcile_search_changes(&tree, &report).unwrap();
        let connection = rusqlite::Connection::open(&index_path).unwrap();
        let stable_pages: i64 = connection
            .query_row(
                "SELECT count(*) FROM pages WHERE path = 'stable.md'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stable_pages, 1, "corrupt hot index must rebuild every Page");
        connection
            .execute(
                "UPDATE metadata SET value = 'wrong-version' WHERE key = 'version'",
                [],
            )
            .unwrap();
        drop(connection);
        fs::write(
            tree.join("General/changed.md"),
            "# Changed\n\n## Active procedure\n\nGold text.\n",
        )
        .unwrap();
        reconcile_search_changes(&tree, &report).unwrap();
        let stable_pages: i64 = rusqlite::Connection::open(index_path)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM pages WHERE path = 'stable.md'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            stable_pages, 1,
            "incompatible hot index must rebuild every Page"
        );
    }

    #[test]
    fn search_filters_folders_marks_sync_state_and_excludes_generated_or_outside_markdown() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        for folder in ["General", "Research", "Locked"] {
            fs::create_dir_all(tree.join(folder)).unwrap();
        }
        fs::create_dir_all(tree.join("General/_wiki")).unwrap();
        let general_markdown = "# General\n\nA heliotrope marker is synced.\n";
        fs::write(tree.join("General/synced.md"), general_markdown).unwrap();
        fs::write(
            tree.join("Research/conflicted.md"),
            "# Research\n\nA heliotrope marker is conflicted.\n",
        )
        .unwrap();
        fs::write(
            tree.join("General/_wiki/generated.md"),
            "# Generated\n\nheliotrope should not surface\n",
        )
        .unwrap();
        let mut agent = AgentState::new("brain", "2026-06-24T20:46:36Z");
        agent.conflicts.push(ConflictEntry {
            id: "conflict-search".to_owned(),
            folder_id: Some("research".to_owned()),
            path: Some("Research/conflicted.md".to_owned()),
            reason: "test conflict".to_owned(),
            state: ConflictState::Open,
            created_at: "2026-06-24T20:46:36Z".to_owned(),
            resolved_at: None,
        });
        write_agent_state(&tree, &agent).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![
                    WorkingTreeFolderRoot {
                        folder_id: "general".to_owned(),
                        source_brain_id: None,
                        path: "General".to_owned(),
                        can_read: true,
                        metadata_only: false,
                    },
                    WorkingTreeFolderRoot {
                        folder_id: "research".to_owned(),
                        source_brain_id: None,
                        path: "Research".to_owned(),
                        can_read: true,
                        metadata_only: false,
                    },
                    WorkingTreeFolderRoot {
                        folder_id: "locked".to_owned(),
                        source_brain_id: None,
                        path: "Locked".to_owned(),
                        can_read: false,
                        metadata_only: true,
                    },
                ],
                objects: vec![WorkingTreeObjectManifestEntry {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "synced.md".to_owned(),
                    object_id: "obj_synced000001".to_owned(),
                    revision: 1,
                    key_version: 1,
                    content_type: "text/markdown".to_owned(),
                    content_hash: finite_brain_core::sha256_hex(general_markdown.as_bytes()),
                }],
                sync: WorkingTreeSyncState { latest_sequence: 1 },
            },
        )
        .unwrap();
        #[cfg(unix)]
        {
            let outside = tmp.path().join("outside.md");
            fs::write(&outside, "# Outside\n\nheliotrope outside\n").unwrap();
            symlink(&outside, tree.join("General/outside.md")).unwrap();
        }
        let mut env = env_for(&tmp);
        env.cwd = tree;

        let mut output = Vec::new();
        run_with_env(["search", "heliotrope", "--json"], env.clone(), &mut output).unwrap();
        let report: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(report["results"].as_array().unwrap().len(), 2);
        let dispositions = report["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|result| result["disposition"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(dispositions, BTreeSet::from(["conflicted", "synced"]));

        let mut output = Vec::new();
        run_with_env(
            ["search", "heliotrope", "--folder", "General", "--json"],
            env.clone(),
            &mut output,
        )
        .unwrap();
        let filtered: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(filtered["searchedFolders"], serde_json::json!(["general"]));
        assert_eq!(filtered["results"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["results"][0]["disposition"], "synced");

        let error = run_with_env(
            ["search", "heliotrope", "--folder", "Locked", "--json"],
            env,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown or not readable"));
    }

    #[cfg(unix)]
    #[test]
    fn search_refuses_a_folder_root_symlink_instead_of_indexing_outside_content() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            outside.join("secret.md"),
            "# Secret\n\nexternal heliotrope\n",
        )
        .unwrap();
        symlink(&outside, tree.join("Mounted")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "mounted".to_owned(),
                    source_brain_id: None,
                    path: "Mounted".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree;

        let error =
            run_with_env(["search", "heliotrope", "--json"], env, &mut Vec::new()).unwrap_err();

        assert!(matches!(error, CliError::InsecureWorkingTree { .. }));
    }

    #[test]
    fn embedding_provider_adapter_owns_authentication_and_validates_vectors() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (request_line, headers, body) = read_http_request_with_headers(&mut stream);
            let response_body = serde_json::json!({
                "model": "embed-test",
                "modelVersion": "embed-test-v1",
                "dimensions": 3,
                "vectors": [
                    { "id": "section-1", "embedding": [0.25, 0.5, 0.75] },
                    { "id": "query-1", "embedding": [0.1, 0.2, 0.3] }
                ],
                "requestId": "req-test"
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            (request_line, headers, body)
        });
        let adapter = EmbeddingProviderAdapter::new(EmbeddingProviderConfig {
            endpoint,
            bearer_token: "adapter-secret".to_owned(),
            timeout: Duration::from_secs(2),
        })
        .unwrap();

        let response = adapter
            .embed(&[
                EmbeddingProviderInput::section(
                    "section-1",
                    "Runbook",
                    vec!["Operations".to_owned(), "Rotation".to_owned()],
                    "Rotate the token after removing a device.",
                ),
                EmbeddingProviderInput::query("query-1", "How do I rotate credentials?"),
            ])
            .unwrap();

        assert_eq!(response.model, "embed-test");
        assert_eq!(response.model_version, "embed-test-v1");
        assert_eq!(response.dimensions, 3);
        assert_eq!(response.vectors[0].id, "section-1");
        assert_eq!(response.vectors[0].embedding, vec![0.25, 0.5, 0.75]);
        let (request_line, headers, body) = server.join().unwrap();
        assert_eq!(request_line, "POST /v1/embeddings HTTP/1.1");
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("authorization: bearer adapter-secret")
        );
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["inputs"][0]["id"], "section-1");
        assert_eq!(body["inputs"][0]["pageTitle"], "Runbook");
        assert_eq!(
            body["inputs"][1],
            serde_json::json!({
                "id": "query-1",
                "kind": "query",
                "text": "How do I rotate credentials?"
            })
        );
        assert!(body["inputs"][0].get("folderId").is_none());
        assert!(body["inputs"][0].get("path").is_none());
    }

    #[test]
    fn embedding_provider_adapter_rejects_wrong_dimension_vectors() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request_with_headers(&mut stream);
            let response_body = serde_json::json!({
                "model": "embed-test",
                "modelVersion": "embed-test-v1",
                "dimensions": 3,
                "vectors": [{ "id": "query-1", "embedding": [0.1, 0.2] }]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let adapter = EmbeddingProviderAdapter::new(EmbeddingProviderConfig {
            endpoint,
            bearer_token: "adapter-secret".to_owned(),
            timeout: Duration::from_secs(2),
        })
        .unwrap();

        let error = adapter
            .embed(&[EmbeddingProviderInput::query("query-1", "test query")])
            .unwrap_err();

        server.join().unwrap();
        assert!(error.to_string().contains("wrong-dimension"));
    }

    #[test]
    fn embedding_provider_debug_output_never_exposes_credentials() {
        let token = "internal-beta-provider-secret";
        let config = EmbeddingProviderConfig {
            endpoint: "https://specialization.example".to_owned(),
            bearer_token: token.to_owned(),
            timeout: Duration::from_secs(5),
        };
        assert!(!format!("{config:?}").contains(token));

        let adapter = EmbeddingProviderAdapter::new(config).unwrap();
        assert!(!format!("{adapter:?}").contains(token));
    }

    #[test]
    fn embedding_provider_adapter_batches_and_rejects_mixed_generations() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let started = Instant::now();
            let mut batch_sizes = Vec::new();
            while batch_sizes.len() < 2 && started.elapsed() < Duration::from_secs(3) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (_, _, body) = read_http_request_with_headers(&mut stream);
                let request: Value = serde_json::from_str(&body).unwrap();
                let inputs = request["inputs"].as_array().unwrap();
                batch_sizes.push(inputs.len());
                let model_version = if batch_sizes.len() == 1 {
                    "embed-test-v1"
                } else {
                    "embed-test-v2"
                };
                let vectors = inputs
                    .iter()
                    .map(|input| {
                        serde_json::json!({
                            "id": input["id"],
                            "embedding": [0.25, 0.5, 0.75]
                        })
                    })
                    .collect::<Vec<_>>();
                let response_body = serde_json::json!({
                    "model": "embed-test",
                    "modelVersion": model_version,
                    "dimensions": 3,
                    "vectors": vectors
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            batch_sizes
        });
        let adapter = EmbeddingProviderAdapter::new(EmbeddingProviderConfig {
            endpoint,
            bearer_token: "adapter-secret".to_owned(),
            timeout: Duration::from_secs(2),
        })
        .unwrap();
        let inputs = (0..65)
            .map(|index| EmbeddingProviderInput::query(format!("query-{index}"), "find it"))
            .collect::<Vec<_>>();

        let error = adapter.embed(&inputs).unwrap_err();

        assert!(error.to_string().contains("model contract between batches"));
        assert_eq!(server.join().unwrap(), vec![64, 1]);
    }

    fn read_http_request_with_headers(stream: &mut TcpStream) -> (String, String, String) {
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
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => bytes.extend_from_slice(&buffer[..size]),
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
            let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let body_start = header_end + 4;
            if bytes.len() >= body_start + content_length {
                return (
                    headers.lines().next().unwrap_or_default().to_owned(),
                    headers,
                    String::from_utf8_lossy(&bytes[body_start..body_start + content_length])
                        .to_string(),
                );
            }
        }
        panic!("embedding adapter test server did not receive a complete request");
    }

    #[test]
    fn daemon_watch_once_records_blocked_sync_without_crashing() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);

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
    fn daemon_watch_reconciles_search_indexes_when_remote_sync_is_blocked() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        fs::write(
            tree.join("General/local.md"),
            "# Local\n\nIndex this saved change while offline.\n",
        )
        .unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();
        let mut output = Vec::new();

        run_with_env(["daemon", "watch", "--once", "--json"], env, &mut output).unwrap();

        let report: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(report["failures"], 1);
        let indexes = fs::read_dir(tree.join(".finitebrain/search-indexes"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(indexes.len(), 1);
    }

    #[test]
    fn failed_remote_sync_still_indexes_a_local_save_after_cold_start() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        write_agent_state(&tree, &AgentState::new("brain", "2026-06-24T20:46:36Z")).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
                    path: "General".to_owned(),
                    can_read: true,
                    metadata_only: false,
                }],
                objects: Vec::new(),
                sync: WorkingTreeSyncState { latest_sequence: 0 },
            },
        )
        .unwrap();
        fs::write(tree.join("General/local.md"), "# Local\n\nAmber text.\n").unwrap();
        let bootstrap_report = SyncOnceReport {
            status: "bootstrapped".to_owned(),
            latest_sequence: 1,
            record_count: 0,
            server_url: "http://127.0.0.1".to_owned(),
            local_changes: Vec::new(),
            remote_changes: Vec::new(),
            conflicts: Vec::new(),
        };
        assert!(reconcile_search_changes(&tree, &bootstrap_report).unwrap() > 0);
        fs::write(tree.join("General/local.md"), "# Local\n\nCobalt text.\n").unwrap();
        let mut env = env_for(&tmp);
        env.cwd = tree.clone();

        assert!(sync_once(&env, &[], "test.failed-sync").is_err());

        let index_directory = fs::read_dir(tree.join(".finitebrain/search-indexes"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let connection = rusqlite::Connection::open(index_directory.join("index.sqlite3")).unwrap();
        let body: String = connection
            .query_row(
                "SELECT body FROM sections WHERE page_path = 'local.md'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(body.contains("Cobalt"));
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
            export_grant_for_test(&env, "brain", "general", 1, &folder_key, &actor_npub);
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
        assert!(requests[0].contains("/_admin/brains/brain/export"));
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/brains/brain/sync/records?after=0"))
        );
        assert!(
            !requests
                .iter()
                .any(|request| request.contains("/_admin/brains/brain/sync/bootstrap"))
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
            export_grant_for_test(&env, "brain", "general", 1, &folder_key, &actor_npub);
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
        assert!(
            text.contains("- applied create General/compiled/summary.md seq=7 actor=npub-remote")
        );
        assert!(text.contains("conflicts: none"));

        let requests = server.join().unwrap();
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/brains/brain/sync/records?after=0"))
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
            export_grant_for_test(&env, "brain", "general", 1, &folder_key, &actor_npub);
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
                .any(|request| request.contains("/_admin/brains/brain/sync/records?after=2"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/brains/brain/sync/bootstrap"))
        );
    }

    #[test]
    fn two_member_identity_sync_reports_writer_identity_to_receiver() {
        let tmp = TempDir::new().unwrap();
        let agent_a_nsec = "0000000000000000000000000000000000000000000000000000000000000001";
        let agent_b_nsec = "0000000000000000000000000000000000000000000000000000000000000002";
        let agent_a_config = tmp.path().join("agent-a-config");
        let agent_b_config = tmp.path().join("agent-b-config");
        let agent_a_tree = setup_incremental_tree_named(&tmp, "agent-a", 0);
        let agent_b_tree = setup_incremental_tree_named(&tmp, "agent-b", 0);
        let env_a = CliEnvironment {
            cwd: agent_a_tree.clone(),
            config_dir: agent_a_config.clone(),
            working_tree_root: None,
            now: Some("2026-06-24T20:46:36Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(tmp.path().join("finite-home-a")),
            embedding_provider: None,
        };
        let env_b = CliEnvironment {
            cwd: agent_b_tree.clone(),
            config_dir: agent_b_config,
            working_tree_root: None,
            now: Some("2026-06-24T20:46:36Z".to_owned()),
            identity_authority_url: None,
            finite_home: Some(tmp.path().join("finite-home-b")),
            embedding_provider: None,
        };
        import_identity_for(&env_a, agent_a_nsec);
        import_identity_for(&env_b, agent_b_nsec);
        fs::write(agent_b_tree.join("General/shared.md"), "# Shared\n").unwrap();

        let agent_a_npub = load_signer(&env_a).unwrap().npub;
        let agent_b_npub = load_signer(&env_b).unwrap().npub;
        assert_ne!(agent_a_npub, agent_b_npub);
        let folder_key = FolderKey::from_bytes([6; 32]);
        let export_grants = vec![
            export_grant_for_test(&env_b, "brain", "general", 1, &folder_key, &agent_a_npub),
            export_grant_for_test(&env_b, "brain", "general", 1, &folder_key, &agent_b_npub),
        ];
        let (server_url, server) = start_two_member_identity_incremental_sync_server(export_grants);
        let mut output_b = Vec::new();
        run_with_env(
            ["sync", "now", "--server", &server_url, "--json"],
            env_b.clone(),
            &mut output_b,
        )
        .unwrap();
        let json_b: Value = serde_json::from_slice(&output_b).unwrap();
        assert_eq!(json_b["status"], "pushed-local-changes");
        assert_eq!(json_b["localChanges"].as_array().unwrap().len(), 1);

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
        assert_eq!(json_a["remoteChanges"][0]["actorNpub"], agent_b_npub);
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
                .any(|request| request.contains("/_admin/brains/brain/sync/bootstrap"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/brains/brain/sync/records?after=0"))
        );
    }

    #[test]
    fn sync_now_records_server_write_conflicts_through_public_command() {
        let tmp = TempDir::new().unwrap();
        import_identity_secret(
            &tmp,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        fs::write(tree.join("General/new.md"), "# New\n").unwrap();
        let folder_key = FolderKey::from_bytes([9; 32]);

        let now = "2026-06-24T20:46:36Z";
        let mut state = AgentState::new("brain", now);
        state.server_url = Some("http://127.0.0.1:9".to_owned());
        state.daemon.state = DaemonRunState::Running;
        write_agent_state(&tree, &state).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
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
            export_grant_for_test(&env, "brain", "general", 1, &folder_key, &actor_npub);
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
        assert!(requests[0].contains("/_admin/brains/brain/export"));
        assert!(requests.iter().any(|request| {
            request.starts_with("PUT /_admin/brains/brain/folders/general/objects/obj_")
        }));
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/_admin/brains/brain/sync/bootstrap"))
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
        let tree = tmp.path().join("brain");
        initialize_private_working_tree(&tree).unwrap();
        fs::create_dir_all(tree.join("General")).unwrap();
        fs::write(tree.join("General/a.md"), "# Accepted\n").unwrap();
        fs::write(tree.join("General/b.md"), "# Conflict\n").unwrap();
        let folder_key = FolderKey::from_bytes([9; 32]);

        let now = "2026-06-24T20:46:36Z";
        let mut state = AgentState::new("brain", now);
        state.server_url = Some("http://127.0.0.1:9".to_owned());
        state.daemon.state = DaemonRunState::Running;
        write_agent_state(&tree, &state).unwrap();
        write_json_file(
            &tree.join(".finitebrain/working-tree-state.json"),
            &BrainWorkingTreeStateManifest {
                version: WORKING_TREE_STATE_VERSION.to_owned(),
                folder_roots: vec![WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: None,
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
            export_grant_for_test(&env, "brain", "general", 1, &folder_key, &actor_npub);
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
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "brain", tree.to_str().unwrap()]);
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
                .any(|request| request.contains("/_admin/brains/brain/sync/records?after=0"))
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
        let body = br#"{"brainId":"agent"}"#;
        let url = "http://127.0.0.1:3015/_admin/brains";
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
    fn management_parser_uses_current_brain_not_target_positional() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("brain");
        run(&tmp, &["open", "agent-brain", tree.to_str().unwrap()]);

        let mut env = env_for(&tmp);
        env.cwd = tree;
        let args = vec!["add-member".to_owned(), "npub-target".to_owned()];

        assert_eq!(command_brain_id(&args, &env).unwrap(), "agent-brain");
    }

    #[test]
    fn folder_required_recipients_follow_access_mode() {
        let metadata = BrainMetadataView {
            brain_id: "org".to_owned(),
            kind: "organization".to_owned(),
            name: "Org".to_owned(),
            owner_user_id: None,
            personal_agent: None,
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

        let personal_metadata = BrainMetadataView {
            brain_id: "personal".to_owned(),
            kind: "personal".to_owned(),
            name: "Personal".to_owned(),
            owner_user_id: Some("npub-owner".to_owned()),
            personal_agent: Some(PersonalAgentView {
                agent_npub: "npub-agent".to_owned(),
            }),
            members: Vec::new(),
            admins: Vec::new(),
            folders: Vec::new(),
            mounted_folders: Vec::new(),
            grant_count: 0,
        };
        assert_eq!(
            folder_required_recipients(&personal_metadata, "restricted", &[]).unwrap(),
            vec!["npub-agent".to_owned(), "npub-owner".to_owned()]
        );
    }

    #[test]
    fn cli_folder_grants_open_through_the_canonical_hosted_contract() {
        let tmp = TempDir::new().unwrap();
        let env = env_for(&tmp);
        import_identity_for(
            &env,
            "0000000000000000000000000000000000000000000000000000000000000001",
        );
        let issuer = load_signer(&env).unwrap();
        let recipient =
            Keys::parse("0000000000000000000000000000000000000000000000000000000000000002")
                .unwrap();
        let recipient_npub = NostrPublicKey::from_protocol(recipient.public_key())
            .to_npub()
            .unwrap();
        let folder_key = FolderKey::generate();
        let grant = folder_key_grant_request(
            &issuer,
            "personal-test",
            "agent-notes",
            1,
            &recipient_npub,
            &folder_key,
            &env,
        )
        .unwrap();
        let opened = open_folder_key_grant(
            &recipient,
            &BrainGrantIntent {
                purpose: "folder-key-grant".to_owned(),
                brain_id: "personal-test".to_owned(),
                recipient_npub,
                folder_id: Some("agent-notes".to_owned()),
                key_version: Some(1),
            },
            grant["wrappedEventJson"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(opened.folder_key, folder_key.to_base64());
    }
}
