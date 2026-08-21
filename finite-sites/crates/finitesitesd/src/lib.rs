//! `finitesitesd` — the Finite Sites server.
//!
//! Subcommands:
//!   serve     run the API + site-serving HTTP server
//!   allow     add an operator publish grant for a pubkey (hex or npub)
//!   disallow  revoke an operator publish grant
//!   allowed   list active publishing grants
//!   project-visibility  set repository clone/fetch visibility
//!   disable-site  stop serving one site without releasing its name
//!   delete-site  stop serving one site as deleted without releasing its name
//!   pre-user-reset  wipe product state during pre-user development
//!   git-post-receive  internal hook helper for Project Repositories
//!
//! All subcommands take `--data DIR`; the registry database, blob store,
//! cookie secret, and dev-mail outbox live under that directory.

pub mod api;
pub mod apps;
pub mod content_type;
pub mod documents;
pub mod git;
pub mod identity;
pub mod limiter;
pub mod llms;
pub mod mailer;
pub mod pages;
pub mod proxy;
pub mod server;
pub mod sites;
mod tar_safety;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use finitesites_blob::BlobStore;
use finitesites_engine::{Engine, EngineConfig};
use finitesites_proto::{hex, ids, npub};
use finitesites_store::{
    ProjectVisibility, ProjectVisibilityUpdate, PublishGrantSource, SiteStatus, SiteStatusUpdate,
    Store,
};

const IDENTITY_AUTHORITY_ENV: &str = "FINITE_IDENTITY_AUTHORITY";
const DEFAULT_IDENTITY_AUTHORITY_URL: &str = "https://identity.finite.vip";
const CORE_API_BASE_URL_ENV: &str = "FC_CORE_API_BASE_URL";
const CORE_API_TOKEN_ENV: &str = "FC_CORE_API_TOKEN";
const VIEWER_SESSION_SERVICE_TOKEN_ENV: &str = "FINITE_SITES_VIEWER_SESSION_TOKEN";

pub struct ServeOptions {
    pub data_dir: PathBuf,
    pub listen: SocketAddr,
    pub base_domain: String,
    pub document_base_domain: String,
    pub api_url: String,
    pub git_base_url: String,
    /// Dedicated account-boundary credential for the internal viewer-session
    /// exchange. It comes from the environment, never argv.
    pub viewer_session_service_token: Option<String>,
    pub git_hook_helper_path: PathBuf,
    pub git_auto_reconcile: bool,
    pub site_url_scheme: String,
    pub site_url_port: Option<u16>,
    /// `None` = DevMailer (outbox files), selected with `--mailer dev`.
    /// Omitting `--mailer` is an error; there is no implicit default.
    /// The API key for an HTTP provider comes from its environment variable,
    /// never from argv.
    pub mail_provider: Option<mailer::MailerKind>,
    pub mail_from: Option<String>,
    /// How tier-2 apps are isolated and run.
    pub app_runner_kind: AppRunnerKind,
    /// Exact operator-owned sudo executable used only by the Kata app runner.
    pub app_sudo_path: PathBuf,
    /// Exact operator-owned nerdctl executable authorized by sudoers.
    pub app_nerdctl_path: PathBuf,
    /// Exact CNI plugin directory forwarded to nerdctl after sudo resets env.
    pub app_cni_path: PathBuf,
    /// Apps with no requests for this long are stopped to free memory and
    /// woken on the next request (the density mechanism).
    pub idle_timeout_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppRunnerKind {
    /// Record app publishes but run nothing (local dev, tests).
    Disabled,
    /// systemd DynamicUser sandbox — kernel isolation (ADR-0014).
    Systemd,
    /// Kata Containers microVM — hardware isolation (ADR-0015).
    Kata,
}

pub fn run(args: Vec<String>) -> Result<(), String> {
    let Some(command) = args.first() else {
        return Err(usage());
    };
    match command.as_str() {
        "serve" => {
            let options = parse_serve_options(&args[1..])?;
            serve(options)
        }
        "allow" => allowlist_mutate(&args[1..], true),
        "disallow" => allowlist_mutate(&args[1..], false),
        "allowed" => allowlist_list(&args[1..]),
        "project-visibility" => project_visibility_mutate(&args[1..]),
        "disable-site" => site_status_mutate(&args[1..], SiteStatus::Disabled, "site_disabled"),
        "delete-site" => delete_site(&args[1..]),
        "pre-user-reset" => pre_user_reset(&args[1..]),
        "reconcile-identity" => reconcile_identity(&args[1..]),
        "git-post-receive" => git_post_receive(),
        "--version" | "-V" | "version" => version(&args[1..]),
        "--help" | "help" => {
            println!("{}", usage());
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n{}", usage())),
    }
}

fn usage() -> String {
    "usage:\n  finitesitesd serve --data DIR [--listen 127.0.0.1:8787] \
     [--base-domain sites.localhost] [--api-url http://127.0.0.1:8787] \
     [--document-base-domain docs.sites.localhost] \
     [--git-url http://git.sites.localhost:8787] \
     [--git-hook-helper PATH] [--git-auto-reconcile true|false] \
     [--site-scheme http] [--site-port PORT|none] \
     --mailer dev|resend [--mail-from ADDR] \
     [--app-runner none|systemd|kata] [--app-sudo-path PATH] \
     [--app-nerdctl-path PATH] [--app-cni-path PATH] \
     [--app-idle-timeout SECONDS]\n  \
     finitesitesd allow --data DIR PUBKEY_OR_NPUB [--note TEXT]\n  \
     finitesitesd disallow --data DIR PUBKEY_OR_NPUB\n  \
     finitesitesd allowed --data DIR\n  \
     finitesitesd project-visibility --data DIR PROJECT private|public-read\n  \
     finitesitesd disable-site --data DIR SITE_NAME\n  \
     finitesitesd delete-site --data DIR SITE_NAME --confirm-delete-site yes\n  \
     finitesitesd pre-user-reset --data DIR --confirm-wipe-product-data yes\n  \
     finitesitesd reconcile-identity --data DIR [--apply yes] \
       [--identity-authority-url https://identity.finite.vip] \
       [--core-api-url URL]\n  \
     finitesitesd git-post-receive"
        .to_string()
}

fn version(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err("usage: finitesitesd --version".to_string());
    }
    println!("finitesitesd {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

type ParsedFlags = (Vec<(String, String)>, Vec<String>);

/// Tiny explicit flag parser: `--flag value` pairs plus positionals.
/// We parse by hand instead of adding a CLI dependency; the surface is small.
fn parse_flags(args: &[String]) -> Result<ParsedFlags, String> {
    let mut flags = Vec::new();
    let mut positionals = Vec::new();
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        let arg = &args[index];
        if let Some(name) = arg.strip_prefix("--") {
            let value = args
                .get(index + 1)
                .ok_or_else(|| format!("flag --{name} needs a value"))?;
            flags.push((name.to_string(), value.clone()));
            index += 2;
        } else {
            positionals.push(arg.clone());
            index += 1;
        }
    }
    Ok((flags, positionals))
}

fn flag_value<'a>(flags: &'a [(String, String)], name: &str) -> Option<&'a str> {
    flags
        .iter()
        .find(|(flag, _)| flag == name)
        .map(|(_, value)| value.as_str())
}

fn parse_serve_options(args: &[String]) -> Result<ServeOptions, String> {
    let (flags, positionals) = parse_flags(args)?;
    if !positionals.is_empty() {
        return Err(format!("unexpected argument `{}`", positionals[0]));
    }
    let data_dir = flag_value(&flags, "data").ok_or("--data DIR is required")?;
    let mail_provider = match flag_value(&flags, "mailer") {
        None => {
            return Err("--mailer is required (dev|resend)".to_string());
        }
        Some("dev") => None,
        Some(raw) => Some(
            mailer::MailerKind::parse(raw)
                .ok_or_else(|| format!("unknown --mailer `{raw}` (dev|resend)"))?,
        ),
    };
    let mail_from = flag_value(&flags, "mail-from").map(str::to_string);
    if mail_provider.is_some() && mail_from.is_none() {
        return Err("--mailer resend requires --mail-from".to_string());
    }
    let listen: SocketAddr = flag_value(&flags, "listen")
        .unwrap_or("127.0.0.1:8787")
        .parse()
        .map_err(|_| "invalid --listen address".to_string())?;
    let base_domain = flag_value(&flags, "base-domain")
        .unwrap_or("sites.localhost")
        .to_string();
    if base_domain.is_empty() || base_domain.contains(':') || base_domain.contains('/') {
        return Err("--base-domain must be a bare domain".to_string());
    }
    let document_base_domain = flag_value(&flags, "document-base-domain")
        .map(str::to_string)
        .unwrap_or_else(|| format!("docs.{base_domain}"));
    if document_base_domain.is_empty()
        || document_base_domain.contains(':')
        || document_base_domain.contains('/')
    {
        return Err("--document-base-domain must be a bare domain".to_string());
    }
    let api_url = flag_value(&flags, "api-url")
        .map(str::to_string)
        .unwrap_or_else(|| format!("http://{listen}"));
    if api_url.ends_with('/') {
        return Err("--api-url must not end with /".to_string());
    }
    let site_url_scheme = flag_value(&flags, "site-scheme")
        .unwrap_or("http")
        .to_string();
    // Default the site-URL port to the listen port: in local dev the same
    // process serves both planes. Behind a real proxy pass `--site-port none`.
    let site_url_port = match flag_value(&flags, "site-port") {
        None => Some(listen.port()),
        Some("none") => None,
        Some(raw) => Some(
            raw.parse::<u16>()
                .map_err(|_| "invalid --site-port".to_string())?,
        ),
    };
    let git_base_url = match flag_value(&flags, "git-url") {
        Some(raw) => {
            if raw.ends_with('/') {
                return Err("--git-url must not end with /".to_string());
            }
            raw.to_string()
        }
        None => {
            let port_part = match site_url_port {
                Some(port) => format!(":{port}"),
                None => String::new(),
            };
            format!("{site_url_scheme}://git.{base_domain}{port_part}")
        }
    };
    let viewer_session_service_token = std::env::var(VIEWER_SESSION_SERVICE_TOKEN_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    validate_viewer_session_service_token(viewer_session_service_token.as_deref())?;
    let git_hook_helper_path = match flag_value(&flags, "git-hook-helper") {
        Some(raw) => PathBuf::from(raw),
        None => std::env::current_exe()
            .map_err(|error| format!("cannot determine current executable: {error}"))?,
    };
    let git_auto_reconcile = match flag_value(&flags, "git-auto-reconcile") {
        None | Some("true") => true,
        Some("false") => false,
        Some(other) => {
            return Err(format!(
                "unknown --git-auto-reconcile `{other}` (true|false)"
            ));
        }
    };
    let app_runner_kind = match flag_value(&flags, "app-runner") {
        None | Some("none") => AppRunnerKind::Disabled,
        Some("systemd") => AppRunnerKind::Systemd,
        Some("kata") => AppRunnerKind::Kata,
        Some(other) => {
            return Err(format!(
                "unknown --app-runner `{other}` (none|systemd|kata)"
            ));
        }
    };
    let app_sudo_path = nonempty_operator_path(&flags, "app-sudo-path", "sudo")?;
    let app_nerdctl_path = nonempty_operator_path(&flags, "app-nerdctl-path", "nerdctl")?;
    let app_cni_path = nonempty_operator_path(&flags, "app-cni-path", "/opt/cni/bin")?;
    let idle_timeout_seconds = match flag_value(&flags, "app-idle-timeout") {
        None => apps::DEFAULT_IDLE_TIMEOUT_SECONDS,
        Some(raw) => raw
            .parse::<u64>()
            .ok()
            .filter(|seconds| *seconds > 0)
            .ok_or("--app-idle-timeout must be a positive number of seconds")?,
    };
    Ok(ServeOptions {
        data_dir: PathBuf::from(data_dir),
        listen,
        base_domain,
        document_base_domain,
        api_url,
        git_base_url,
        viewer_session_service_token,
        git_hook_helper_path,
        git_auto_reconcile,
        site_url_scheme,
        site_url_port,
        mail_provider,
        mail_from,
        app_runner_kind,
        app_sudo_path,
        app_nerdctl_path,
        app_cni_path,
        idle_timeout_seconds,
    })
}

fn nonempty_operator_path(
    flags: &[(String, String)],
    name: &str,
    default: &str,
) -> Result<PathBuf, String> {
    let raw = flag_value(flags, name).unwrap_or(default);
    if raw.trim().is_empty() {
        return Err(format!("--{name} must not be empty"));
    }
    Ok(PathBuf::from(raw))
}

pub(crate) fn validate_viewer_session_service_token(token: Option<&str>) -> Result<(), String> {
    let Some(token) = token else {
        // Absent means the internal endpoint is deliberately disabled.
        return Ok(());
    };
    if !hex::is_hex32(token) {
        return Err(format!(
            "{VIEWER_SESSION_SERVICE_TOKEN_ENV} must be exactly 64 lowercase hex characters"
        ));
    }
    Ok(())
}

/// NIP-05 Directory base URL for the operator-only `reconcile-identity`
/// command. The serve path never talks to the Directory.
fn parse_identity_authority_url(
    flag_value: Option<&str>,
    env_value: Option<&str>,
) -> Result<Option<String>, String> {
    let (raw, source) = match flag_value {
        Some(raw) => (raw.trim(), "--identity-authority-url"),
        None => match env_value.map(str::trim).filter(|value| !value.is_empty()) {
            Some(raw) => (raw, IDENTITY_AUTHORITY_ENV),
            None => return Ok(Some(DEFAULT_IDENTITY_AUTHORITY_URL.to_string())),
        },
    };
    if raw.ends_with('/') {
        return Err(format!("{source} must not end with /"));
    }
    Ok(Some(raw.to_string()))
}

fn git_post_receive() -> Result<(), String> {
    crate::git::run_post_receive_hook_from_env()
}

fn open_store(data_dir: &Path) -> Result<Store, String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|error| format!("cannot create data dir: {error}"))?;
    Store::open(&data_dir.join("registry.db"))
        .map_err(|error| format!("cannot open registry: {error}"))
}

/// Load or create the 32-byte cookie secret at `DATA/cookie-secret`.
fn load_cookie_secret(data_dir: &Path) -> Result<[u8; 32], String> {
    let path = data_dir.join("cookie-secret");
    if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read cookie secret: {error}"))?;
        let bytes = hex::decode32(raw.trim())
            .map_err(|_| "cookie-secret file is corrupt (expected 64 hex chars)".to_string())?;
        return Ok(bytes);
    }
    let secret = ids::random_32();
    std::fs::write(&path, hex::encode(&secret))
        .map_err(|error| format!("cannot write cookie secret: {error}"))?;
    Ok(secret)
}

fn serve(options: ServeOptions) -> Result<(), String> {
    let store = open_store(&options.data_dir)?;
    let blobs = BlobStore::open(&options.data_dir.join("blobs"))
        .map_err(|error| format!("cannot open blob store: {error}"))?;
    let cookie_secret = load_cookie_secret(&options.data_dir)?;
    let engine_config = EngineConfig {
        base_domain: options.base_domain.clone(),
        document_base_domain: options.document_base_domain.clone(),
        site_url_scheme: options.site_url_scheme.clone(),
        site_url_port: options.site_url_port,
    };
    let engine = Engine::new(store, blobs, cookie_secret, engine_config);
    let mail: Box<dyn mailer::Mailer> = match options.mail_provider {
        None => Box::new(
            mailer::DevMailer::new(options.data_dir.join("outbox"))
                .map_err(|error| format!("cannot open outbox: {error}"))?,
        ),
        Some(provider) => {
            let env_var = match provider {
                mailer::MailerKind::Dev => unreachable!("dev is represented as None"),
                mailer::MailerKind::Resend => mailer::RESEND_API_KEY_ENV_VAR,
            };
            let api_key = std::env::var(env_var)
                .map_err(|_| format!("--mailer requires the {env_var} environment variable"))?;
            let from_address = options
                .mail_from
                .clone()
                .expect("mail_from is validated alongside mail_provider");
            Box::new(mailer::HttpMailer::new(api_key, from_address))
        }
    };

    let app_runner: Box<dyn apps::AppRunner> = match options.app_runner_kind {
        AppRunnerKind::Disabled => Box::new(apps::DisabledRunner),
        AppRunnerKind::Systemd => Box::new(
            apps::SystemdAppRunner::new(options.data_dir.join("apps"))
                .map_err(|error| format!("cannot set up systemd app runner: {error}"))?,
        ),
        AppRunnerKind::Kata => Box::new(
            apps::KataAppRunner::new(
                options.data_dir.join("apps"),
                options.app_sudo_path.clone(),
                options.app_nerdctl_path.clone(),
                options.app_cni_path.clone(),
            )
            .map_err(|error| format!("cannot set up kata app runner: {error}"))?,
        ),
    };
    let supervisor = apps::Supervisor::new(app_runner, options.idle_timeout_seconds);

    let runtime =
        tokio::runtime::Runtime::new().map_err(|error| format!("cannot start runtime: {error}"))?;
    runtime.block_on(server::serve(engine, mail, supervisor, options))
}

fn allowlist_mutate(args: &[String], allow: bool) -> Result<(), String> {
    let (flags, positionals) = parse_flags(args)?;
    let data_dir = flag_value(&flags, "data").ok_or("--data DIR is required")?;
    let [key_input] = positionals.as_slice() else {
        return Err("expected exactly one PUBKEY_OR_NPUB argument".to_string());
    };
    let pubkey = npub::pubkey_from_hex_or_npub(key_input)
        .map_err(|error| format!("invalid pubkey: {error}"))?;
    let mut store = open_store(Path::new(data_dir))?;
    if allow {
        let note = flag_value(&flags, "note").unwrap_or("");
        store
            .allow_pubkey(&pubkey, note, server::now_unix())
            .map_err(|error| format!("allow failed: {error}"))?;
        println!(
            "allowed {}",
            npub::encode_npub(&pubkey).expect("valid pubkey")
        );
    } else {
        let removed = store
            .revoke_publish_access(&pubkey, PublishGrantSource::Operator, server::now_unix())
            .map_err(|error| format!("disallow failed: {error}"))?;
        if removed {
            println!(
                "disallowed {}",
                npub::encode_npub(&pubkey).expect("valid pubkey")
            );
        } else {
            println!("pubkey had no operator publishing grant");
        }
    }
    Ok(())
}

fn reconcile_identity(args: &[String]) -> Result<(), String> {
    let (flags, positionals) = parse_flags(args)?;
    if !positionals.is_empty() {
        return Err("reconcile-identity accepts flags only".to_string());
    }
    let data_dir = flag_value(&flags, "data").ok_or("--data DIR is required")?;
    let apply = reconciliation_apply_requested(&flags)?;
    let authority_url = parse_identity_authority_url(
        flag_value(&flags, "identity-authority-url"),
        std::env::var(IDENTITY_AUTHORITY_ENV).ok().as_deref(),
    )?
    .expect("Identity Authority has a compiled production default");
    let authority = identity::IdentityAuthority::new(authority_url);
    let core_url = flag_value(&flags, "core-api-url")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            std::env::var(CORE_API_BASE_URL_ENV)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
    let core_token = std::env::var(CORE_API_TOKEN_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let core_authority = match (core_url, core_token) {
        (Some(url), Some(token)) => Some(identity::CoreAccountAuthority::new(url, token)),
        (None, None) => None,
        _ => {
            return Err(format!(
                "{CORE_API_BASE_URL_ENV}/--core-api-url and {CORE_API_TOKEN_ENV} must be configured together"
            ));
        }
    };
    let mut store = if apply {
        open_store(Path::new(data_dir))?
    } else {
        Store::open_reconciliation_preview(&Path::new(data_dir).join("registry.db"))
            .map_err(|error| format!("cannot open read-only reconciliation preview: {error}"))?
    };
    let mut report = store
        .reconcile_sites_identity()
        .map_err(|error| format!("identity reconciliation failed: {error}"))?;
    let candidates = store
        .legacy_email_grant_candidates()
        .map_err(|error| format!("identity reconciliation failed: {error}"))?;
    for candidate in candidates {
        let resolution = authority
            .resolve_nip05(&candidate.email)
            .map_err(|error| format!("identity reconciliation lookup failed: {error}"))?;
        let Some(resolution) = resolution else {
            continue;
        };
        if resolution.kind != "managed_agent" {
            continue;
        }
        if !hex::is_hex32(&resolution.pubkey) {
            return Err("Identity Authority returned an invalid Managed Agent pubkey".to_string());
        }
        if let Some(core) = core_authority.as_ref() {
            let account = core
                .managed_agent_account(&candidate.email)
                .map_err(|error| format!("Core account reconciliation lookup failed: {error}"))?;
            if let Some(account) = account {
                if store
                    .reconcile_verified_core_agent_key(
                        &account.verified_email,
                        &resolution.pubkey,
                        server::now_unix(),
                    )
                    .map_err(|error| format!("identity reconciliation failed: {error}"))?
                {
                    report.migrated += 1;
                } else {
                    report.unchanged += 1;
                }
            }
        }
        let changed = store
            .add_native_grants_for_legacy_email(
                &candidate.email,
                &resolution.pubkey,
                server::now_unix(),
            )
            .map_err(|error| format!("identity reconciliation failed: {error}"))?;
        if changed == 0 {
            report.unchanged += 1;
        } else {
            report.migrated += changed;
        }
    }
    let final_report = store
        .reconcile_sites_identity()
        .map_err(|error| format!("identity reconciliation failed: {error}"))?;
    report.conflicts = final_report.conflicts;
    report.needs_proof = final_report.needs_proof;
    println!(
        "{}",
        serde_json::json!({
            "mode": if apply { "apply" } else { "preview" },
            "applied": apply,
            "migrated": report.migrated,
            "unchanged": report.unchanged,
            "conflict": report.conflicts,
            "needs_proof": report.needs_proof,
        })
    );
    Ok(())
}

fn reconciliation_apply_requested(flags: &[(String, String)]) -> Result<bool, String> {
    match flag_value(flags, "apply") {
        None => Ok(false),
        Some("yes") => Ok(true),
        Some(_) => Err("reconcile-identity mutates durable authorization; pass --apply yes only after reviewing a preview and approving the backup/rollback boundary".to_string()),
    }
}

fn allowlist_list(args: &[String]) -> Result<(), String> {
    let (flags, _positionals) = parse_flags(args)?;
    let data_dir = flag_value(&flags, "data").ok_or("--data DIR is required")?;
    let store = open_store(Path::new(data_dir))?;
    let grants = store
        .list_publish_grants(server::now_unix())
        .map_err(|error| format!("list failed: {error}"))?;
    if grants.is_empty() {
        println!("publishing grant cache is empty");
        return Ok(());
    }
    // Bounded: operator/Core curated list.
    for grant in grants {
        let display = npub::encode_npub(&grant.pubkey).unwrap_or(grant.pubkey);
        let source = grant.source.as_str();
        let expires = match grant.expires_at {
            Some(expires_at) => format!(", expires_at={expires_at}"),
            None => String::new(),
        };
        if grant.note.is_empty() {
            println!("{display}  # source={source}{expires}");
        } else {
            println!("{display}  # source={source}{expires}, {}", grant.note);
        }
    }
    Ok(())
}

fn project_visibility_mutate(args: &[String]) -> Result<(), String> {
    let (flags, positionals) = parse_flags(args)?;
    let data_dir = flag_value(&flags, "data").ok_or("--data DIR is required")?;
    let [project_slug, visibility_raw] = positionals.as_slice() else {
        return Err("expected PROJECT and VISIBILITY arguments (private|public-read)".to_string());
    };
    let visibility = ProjectVisibility::parse(visibility_raw)
        .ok_or("VISIBILITY must be private or public-read")?;
    let mut store = open_store(Path::new(data_dir))?;
    let update = store
        .set_project_visibility_by_slug(project_slug, visibility, server::now_unix())
        .map_err(|error| format!("project-visibility failed: {error}"))?;
    print_project_visibility_update(&update);
    Ok(())
}

fn print_project_visibility_update(update: &ProjectVisibilityUpdate) {
    println!("project:    {}", update.slug);
    println!("project_id: {}", update.project_id);
    println!("from:       {}", update.previous_visibility.as_str());
    println!("to:         {}", update.visibility.as_str());
    println!("changed:    {}", update.changed);
}

fn site_status_mutate(
    args: &[String],
    status: SiteStatus,
    action: &'static str,
) -> Result<(), String> {
    let (flags, positionals) = parse_flags(args)?;
    let data_dir = flag_value(&flags, "data").ok_or("--data DIR is required")?;
    let [site_name] = positionals.as_slice() else {
        return Err("expected exactly one SITE_NAME argument".to_string());
    };
    let mut store = open_store(Path::new(data_dir))?;
    let update = store
        .set_site_status_by_name(site_name, status, action, server::now_unix())
        .map_err(|error| format!("{action} failed: {error}"))?;
    print_site_status_update(&update);
    Ok(())
}

fn delete_site(args: &[String]) -> Result<(), String> {
    let (flags, positionals) = parse_flags(args)?;
    if flag_value(&flags, "confirm-delete-site") != Some("yes") {
        return Err("delete-site is destructive; pass --confirm-delete-site yes".to_string());
    }
    let data_dir = flag_value(&flags, "data").ok_or("--data DIR is required")?;
    let [site_name] = positionals.as_slice() else {
        return Err("expected exactly one SITE_NAME argument".to_string());
    };
    let mut store = open_store(Path::new(data_dir))?;
    let update = store
        .set_site_status_by_name(
            site_name,
            SiteStatus::Deleted,
            "site_deleted",
            server::now_unix(),
        )
        .map_err(|error| format!("delete-site failed: {error}"))?;
    print_site_status_update(&update);
    Ok(())
}

fn print_site_status_update(update: &SiteStatusUpdate) {
    println!("site:     {}", update.name);
    println!("site_id:  {}", update.site_id);
    println!("from:     {}", update.previous_status.as_str());
    println!("to:       {}", update.status.as_str());
    println!("changed:  {}", update.changed);
}

fn pre_user_reset(args: &[String]) -> Result<(), String> {
    let (flags, positionals) = parse_flags(args)?;
    if !positionals.is_empty() {
        return Err(format!("unexpected argument `{}`", positionals[0]));
    }
    let data_dir = flag_value(&flags, "data").ok_or("--data DIR is required")?;
    let confirmed = flag_value(&flags, "confirm-wipe-product-data") == Some("yes");
    if !confirmed {
        return Err(
            "pre-user-reset is destructive; pass --confirm-wipe-product-data yes".to_string(),
        );
    }
    let wiped = reset_product_data(Path::new(data_dir))?;
    if wiped.is_empty() {
        println!("no Finite Sites product data found under {data_dir}");
    } else {
        println!("wiped Finite Sites product data under {data_dir}:");
        // Bounded by the fixed reset path list.
        for item in wiped {
            println!("- {item}");
        }
    }
    println!("preserved host/runtime config such as cookie-secret and deployment files");
    Ok(())
}

fn reset_product_data(data_dir: &Path) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|error| format!("cannot create data dir: {error}"))?;
    let product_entries = [
        "registry.db",
        "registry.db-wal",
        "registry.db-shm",
        "blobs",
        "outbox",
        "apps",
        "git",
    ];
    let mut wiped = Vec::new();
    // Bounded by product_entries above.
    for entry in product_entries {
        let path = data_dir.join(entry);
        if !path.exists() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        if metadata.is_dir() {
            std::fs::remove_dir_all(&path)
                .map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
        } else {
            std::fs::remove_file(&path)
                .map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
        }
        wiped.push(entry.to_string());
    }
    Ok(wiped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_probe_is_read_only() {
        run(vec!["--version".to_string()]).unwrap();
        run(vec!["-V".to_string()]).unwrap();
        run(vec!["version".to_string()]).unwrap();
        assert_eq!(
            run(vec!["--version".to_string(), "extra".to_string()]).unwrap_err(),
            "usage: finitesitesd --version"
        );
    }

    #[test]
    fn operator_disable_and_delete_site_commands() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut store = Store::open(&dir.path().join("registry.db")).unwrap();
            store
                .create_site_with_claim(
                    "site_1",
                    "claim_1",
                    "hello",
                    "1111111111111111111111111111111111111111111111111111111111111111",
                    1,
                )
                .unwrap();
        }
        assert_eq!(
            run(vec![
                "delete-site".to_string(),
                "--data".to_string(),
                dir.path().display().to_string(),
                "hello".to_string(),
            ])
            .unwrap_err(),
            "delete-site is destructive; pass --confirm-delete-site yes"
        );
        run(vec![
            "disable-site".to_string(),
            "--data".to_string(),
            dir.path().display().to_string(),
            "hello".to_string(),
        ])
        .unwrap();
        {
            let store = Store::open(&dir.path().join("registry.db")).unwrap();
            assert_eq!(
                store.site_by_name("hello").unwrap().unwrap().status,
                SiteStatus::Disabled
            );
        }
        run(vec![
            "delete-site".to_string(),
            "--data".to_string(),
            dir.path().display().to_string(),
            "--confirm-delete-site".to_string(),
            "yes".to_string(),
            "hello".to_string(),
        ])
        .unwrap();
        let store = Store::open(&dir.path().join("registry.db")).unwrap();
        assert_eq!(
            store.site_by_name("hello").unwrap().unwrap().status,
            SiteStatus::Deleted
        );
    }

    #[test]
    fn operator_project_visibility_command_updates_and_replays() {
        let dir = tempfile::tempdir().unwrap();
        let owner = "1111111111111111111111111111111111111111111111111111111111111111";
        {
            let mut store = Store::open(&dir.path().join("registry.db")).unwrap();
            store
                .init_project(owner, "finite-skills", &[], server::now_unix())
                .unwrap();
        }

        run(vec![
            "project-visibility".to_string(),
            "--data".to_string(),
            dir.path().display().to_string(),
            "finite-skills".to_string(),
            "public-read".to_string(),
        ])
        .unwrap();
        {
            let store = Store::open(&dir.path().join("registry.db")).unwrap();
            assert_eq!(
                store
                    .project_by_slug("finite-skills")
                    .unwrap()
                    .unwrap()
                    .visibility,
                ProjectVisibility::PublicRead
            );
        }

        run(vec![
            "project-visibility".to_string(),
            "--data".to_string(),
            dir.path().display().to_string(),
            "finite-skills".to_string(),
            "public-read".to_string(),
        ])
        .unwrap();

        assert_eq!(
            run(vec![
                "project-visibility".to_string(),
                "--data".to_string(),
                dir.path().display().to_string(),
                "finite-skills".to_string(),
                "public".to_string(),
            ])
            .unwrap_err(),
            "VISIBILITY must be private or public-read"
        );
    }

    #[test]
    fn pre_user_reset_wipes_product_data_and_preserves_runtime_secret() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("registry.db"), b"db").unwrap();
        std::fs::write(dir.path().join("registry.db-wal"), b"wal").unwrap();
        std::fs::create_dir(dir.path().join("blobs")).unwrap();
        std::fs::write(dir.path().join("blobs").join("blob"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("git")).unwrap();
        std::fs::write(dir.path().join("cookie-secret"), b"secret").unwrap();

        let wiped = reset_product_data(dir.path()).unwrap();
        assert!(wiped.contains(&"registry.db".to_string()));
        assert!(wiped.contains(&"registry.db-wal".to_string()));
        assert!(wiped.contains(&"blobs".to_string()));
        assert!(wiped.contains(&"git".to_string()));
        assert!(!dir.path().join("registry.db").exists());
        assert!(!dir.path().join("blobs").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("cookie-secret")).unwrap(),
            "secret"
        );
    }

    #[test]
    fn identity_authority_url_prefers_flag_and_ignores_empty_env() {
        assert_eq!(
            parse_identity_authority_url(None, None).unwrap(),
            Some(DEFAULT_IDENTITY_AUTHORITY_URL.to_string())
        );
        assert_eq!(
            parse_identity_authority_url(None, Some("  ")).unwrap(),
            Some(DEFAULT_IDENTITY_AUTHORITY_URL.to_string())
        );
        assert_eq!(
            parse_identity_authority_url(None, Some(" https://identity.example ")).unwrap(),
            Some("https://identity.example".to_string())
        );
        assert_eq!(
            parse_identity_authority_url(
                Some("https://identity.flag.example"),
                Some("https://identity.env.example")
            )
            .unwrap(),
            Some("https://identity.flag.example".to_string())
        );
        assert_eq!(
            parse_identity_authority_url(Some("https://identity.flag.example/"), None).unwrap_err(),
            "--identity-authority-url must not end with /"
        );
        assert_eq!(
            parse_identity_authority_url(None, Some("https://identity.env.example/")).unwrap_err(),
            "FINITE_IDENTITY_AUTHORITY must not end with /"
        );
    }

    #[test]
    fn identity_reconciliation_requires_explicit_apply_confirmation() {
        assert!(!reconciliation_apply_requested(&[]).unwrap());
        assert!(
            reconciliation_apply_requested(&[("apply".to_string(), "yes".to_string())]).unwrap()
        );
        assert_eq!(
            reconciliation_apply_requested(&[("apply".to_string(), "true".to_string())])
                .unwrap_err(),
            "reconcile-identity mutates durable authorization; pass --apply yes only after reviewing a preview and approving the backup/rollback boundary"
        );
    }

    #[test]
    fn viewer_session_service_token_is_a_bounded_32_byte_secret() {
        assert!(validate_viewer_session_service_token(None).is_ok());
        assert!(
            validate_viewer_session_service_token(Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            ))
            .is_ok()
        );
        for invalid in [
            "",
            "short",
            "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0",
        ] {
            let error = validate_viewer_session_service_token(Some(invalid)).unwrap_err();
            assert_eq!(
                error,
                "FINITE_SITES_VIEWER_SESSION_TOKEN must be exactly 64 lowercase hex characters"
            );
            if !invalid.is_empty() {
                assert!(!error.contains(invalid));
            }
        }
    }

    #[test]
    fn serve_requires_explicit_mailer() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().display().to_string();

        let omitted = match parse_serve_options(&["--data".to_string(), data.clone()]) {
            Ok(_) => panic!("expected omitted --mailer to fail"),
            Err(error) => error,
        };
        assert!(
            omitted.contains("--mailer"),
            "omitted --mailer should name the flag, got {omitted}"
        );

        let resend_without_from = match parse_serve_options(&[
            "--data".to_string(),
            data.clone(),
            "--mailer".to_string(),
            "resend".to_string(),
        ]) {
            Ok(_) => panic!("expected --mailer resend without --mail-from to fail"),
            Err(error) => error,
        };
        assert!(
            resend_without_from.contains("--mail-from"),
            "resend without --mail-from should name the flag, got {resend_without_from}"
        );

        let hook = dir.path().join("git-hook-helper").display().to_string();
        let options = match parse_serve_options(&[
            "--data".to_string(),
            data,
            "--mailer".to_string(),
            "dev".to_string(),
            "--git-hook-helper".to_string(),
            hook,
        ]) {
            Ok(options) => options,
            Err(error) => panic!("--mailer dev should select DevMailer, got {error}"),
        };
        assert!(options.mail_provider.is_none());
    }

    #[test]
    fn kata_operator_paths_have_safe_nonempty_defaults_and_reject_empty_overrides() {
        assert_eq!(
            nonempty_operator_path(&[], "app-sudo-path", "sudo").unwrap(),
            PathBuf::from("sudo")
        );
        assert_eq!(
            nonempty_operator_path(&[], "app-nerdctl-path", "nerdctl").unwrap(),
            PathBuf::from("nerdctl")
        );
        assert_eq!(
            nonempty_operator_path(&[], "app-cni-path", "/opt/cni/bin").unwrap(),
            PathBuf::from("/opt/cni/bin")
        );
        assert_eq!(
            nonempty_operator_path(
                &[("app-nerdctl-path".to_string(), "  ".to_string())],
                "app-nerdctl-path",
                "nerdctl"
            )
            .unwrap_err(),
            "--app-nerdctl-path must not be empty"
        );
    }
}
