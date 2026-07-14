//! `fsite` — the agent-facing CLI for Finite Sites.
//!
//! Commands hide nostr, keys, manifests, and blob mechanics; the agent only
//! sees names, paths, emails, and URLs:
//!
//!   fsite whoami
//!   fsite auth status [--output json]
//!   fsite auth import [--file PATH] [--output json]
//!   fsite describe workflow publish-static-site --output json
//!   fsite describe workflow publish-stateful-app --output json
//!   fsite auth register --output json
//!   fsite project init --config finite.toml --dry-run --output json
//!   fsite project grant PROJECT --email EDITOR_EMAIL --send-invite --output json
//!   fsite project share PROJECT OUTPUT --public --yes-public --output json
//!   fsite auth git PROJECT [--email EMAIL] [--store] [--output json]
//!   fsite project status PROJECT --output json
//!   fsite project list --output json
//!   fsite view URL_OR_NAME --output json
//!
//! Server address comes from FINITE_SITES_API (default https://api.finite.chat).

mod api;
mod keys;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use thiserror::Error;

use finitesites_proto::dto::{
    ERROR_GIT_REPOSITORY_SETUP_FAILED, ERROR_GIT_UNAVAILABLE, EmailRedeemResponse, GitAuthRequest,
    GitAuthResponse, ProjectGrantRequest, ProjectInitRequest, ProjectRevokeRequest, SharingRequest,
};
use finitesites_proto::npub;
use finitesites_proto::project_config::parse_project_config_toml;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error("key error: {0}")]
    Key(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("server error: {0}")]
    Api(String),
    #[error("server error: {method} {path}: {status}: {message}")]
    ApiStatus {
        method: String,
        path: String,
        status: u16,
        code: Option<String>,
        message: String,
    },
    #[error("network error: {0}")]
    Http(String),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fsite: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), CliError> {
    let Some(command) = args.first() else {
        return Err(CliError::Usage(usage()));
    };
    match command.as_str() {
        "whoami" => no_args_or_help(&args[1..], "fsite whoami", whoami_help(), whoami),
        "describe" => describe(&args[1..]),
        "project" => project_command(&args[1..]),
        "auth" => auth_command(&args[1..]),
        "view" => view(&args[1..]),
        "--version" | "-V" | "version" => version(&args[1..]),
        "email-login" | "email-redeem" | "email-claim" | "status" | "list" | "share" | "claim"
        | "publish" | "publish-app" | "source" => Err(CliError::Usage(
            removed_site_first_command_help(command.as_str()),
        )),
        "--help" | "help" => {
            println!("{}", usage());
            Ok(())
        }
        other => Err(CliError::Usage(format!(
            "unknown command `{other}`\n{}",
            usage()
        ))),
    }
}

fn is_help_arg(arg: &str) -> bool {
    arg == "--help" || arg == "-h" || arg == "help"
}

fn help_requested(args: &[String]) -> bool {
    args.iter().any(|arg| is_help_arg(arg))
}

fn print_help(text: &str) -> Result<(), CliError> {
    println!("{text}");
    Ok(())
}

fn version(args: &[String]) -> Result<(), CliError> {
    if !args.is_empty() {
        return Err(CliError::Usage("usage: fsite --version".to_string()));
    }
    println!("fsite {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

fn no_args_or_help(
    args: &[String],
    command: &str,
    help: &str,
    action: fn() -> Result<(), CliError>,
) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(help);
    }
    if !args.is_empty() {
        return Err(CliError::Usage(format!("usage: {command}")));
    }
    action()
}

fn usage() -> String {
    "Finite Sites shares the whole source tree through a Project Repository \
     for authorized collaborators. The Project's finite.toml selects which \
     committed path becomes the served website; source outside that path is \
     cloneable by collaborators but not served as ordinary web assets.\n\n\
     Agent quick start for a static site:\n  \
     fsite describe workflow publish-static-site --output json\n  \
     fsite describe workflow publish-stateful-app --output json\n  \
     fsite auth register --output json\n  \
     fsite project init --config finite.toml --dry-run --output json\n  \
     fsite project init --config finite.toml --output json\n  \
     fsite auth git PROJECT --store --output json\n  \
     git clone https://git.finite.chat/PROJECT.git\n  \
     # commit finite.toml plus deploy bytes, then push the Deploy Branch\n\n\
     Commands:\n  fsite whoami\n  \
     fsite describe [workflow NAME] [--output json]\n  \
     fsite project init --config finite.toml [--owner-viewer-npub NPUB] [--dry-run] [--output json]\n  \
     fsite project grant PROJECT --email EMAIL [--role editor] [--send-invite] [--output json]\n  \
     fsite project revoke PROJECT --email EMAIL [--output json]\n  \
     fsite project share PROJECT OUTPUT [--public --yes-public|--shared|--private] [--add-email EMAIL]... [--remove-email EMAIL]... [--add-npub NPUB]... [--remove-npub NPUB]... [--send-invite] [--output json]\n  \
     fsite project status PROJECT [--output json]\n  \
     fsite project list [--output json]\n  \
     fsite auth status [--output json]\n  \
     fsite auth import [--file PATH] [--output json]\n  \
     fsite auth register [--output json]\n  \
     fsite auth link-email EMAIL [--output json]\n  \
     fsite auth login EMAIL\n  \
     fsite auth redeem EMAIL TOKEN [--link-native] [--output json]\n  \
     fsite auth git PROJECT [--email EMAIL] [--store] [--output json]\n  \
     fsite view URL_OR_NAME [--output json]"
        .to_string()
}

fn removed_site_first_command_help(command: &str) -> String {
    match command {
        "share" => {
            "`fsite share` is not part of the current Project Repository model.\n\n\
             Use Project Output sharing instead:\n  \
             fsite project status PROJECT --output json\n  \
             fsite project share PROJECT OUTPUT --shared --add-email VIEWER_EMAIL --send-invite --output json\n  \
             fsite project share PROJECT OUTPUT --public --yes-public --output json\n  \
             fsite project share PROJECT OUTPUT --private --output json"
                .to_string()
        }
        "status" => {
            "`fsite status` is not part of the current Project Repository model.\n\n\
             Use Project Status instead:\n  \
             fsite project status PROJECT --output json"
                .to_string()
        }
        "list" => {
            "`fsite list` is not part of the current Project Repository model.\n\n\
             Use Project List instead:\n  \
             fsite project list --output json"
                .to_string()
        }
        "email-login" | "email-redeem" => format!(
            "`fsite {command}` has moved under the auth product verb.\n\n\
             Use:\n  \
             fsite auth login EMAIL\n  \
             fsite auth redeem EMAIL TOKEN"
        ),
        _ => format!(
            "`fsite {command}` is not part of the current Project Repository model.\n\n\
             Use the explicit primitives instead:\n  \
             fsite describe workflow publish-static-site --output json\n  \
             fsite describe workflow publish-stateful-app --output json\n  \
             fsite project init --config finite.toml --dry-run --output json\n  \
             fsite project init --config finite.toml --output json\n  \
             fsite project grant PROJECT --email EDITOR_EMAIL --send-invite --output json\n  \
             fsite auth git PROJECT --store --output json\n  \
             git clone https://git.finite.chat/PROJECT.git\n  \
             # edit, commit, and push the configured Deploy Branch"
        ),
    }
}

fn whoami_help() -> &'static str {
    "usage: fsite whoami\n\nPrint the local User Key npub and identity file path. The User Key is the shared Finite identity (~/.finite/identity/identity.json, or $FINITE_HOME/identity/identity.json); it is created if no Finite tool has minted one yet."
}

fn auth_status_help() -> &'static str {
    "usage: fsite auth status [--output json]\n\nShow the shared Finite identity (User Key) used by all Finite tools without creating or changing anything: npub, identity file path, created_by, and created_at. If no identity exists yet, says so and points at the normal first-run mint or fsite auth import. The identity lives at $FINITE_HOME/identity/identity.json when FINITE_HOME is set and ~/.finite/identity/identity.json otherwise."
}

fn auth_import_help() -> &'static str {
    "usage: fsite auth import [--file PATH] [--output json]\n\nAdopt an existing secret as the shared Finite identity used by all Finite tools. The secret is an nsec1... string or 64-char hex, read from stdin by default or from --file PATH; it is never accepted as a flag value because argv leaks into ps output and shell history. A --file pointing at a legacy fsite identity.env (FINITE_SITES_USER_SECRET=hex) imports that value; any other file content is treated as the secret itself. Fails if a shared identity already exists, because other Finite tools may already be using it."
}

fn describe_help() -> &'static str {
    "usage: fsite describe [workflow NAME] [--output json]\n\nMachine-readable command and workflow discovery. Workflows: register-and-publish, project-config, publish-static-site, publish-stateful-app, publish-document, edit-shared-project, share-output, grant-collaborator, revoke-collaborator."
}

fn project_help() -> &'static str {
    "usage:\n  fsite project init --config finite.toml [--owner-viewer-npub NPUB] [--dry-run] [--output json]\n  fsite project grant PROJECT --email EMAIL [--role editor] [--send-invite] [--output json]\n  fsite project revoke PROJECT --email EMAIL [--output json]\n  fsite project share PROJECT OUTPUT [--public --yes-public|--shared|--private] [--add-email EMAIL]... [--remove-email EMAIL]... [--add-npub NPUB]... [--remove-npub NPUB]... [--send-invite] [--output json]\n  fsite project status PROJECT [--output json]\n  fsite project list [--output json]\n\nProject is the source primitive: init creates the Project Repository and any declared outputs; a [project]-only finite.toml creates a source-only repository. Git edits and publishes content; grant/revoke manage Project edit access; share manages viewer access for one Project Output."
}

fn project_init_help() -> &'static str {
    "usage: fsite project init --config finite.toml [--owner-viewer-npub NPUB] [--dry-run] [--output json]\n\nInitialize one Project Repository from finite.toml. When an authenticated human asked an Agent Principal to publish, --owner-viewer-npub creates that human's explicit revocable Native Principal Share atomically with every declared output; never infer it from message text. A [project]-only config creates a source-only repository with no served output yet. Declared outputs reserve their routing names; init does not deploy bytes. Replay is safe when existing outputs match, and adding missing outputs to the same Project is allowed. To publish an output, commit finite.toml plus the selected output path and push the Deploy Branch."
}

fn project_grant_help() -> &'static str {
    "usage: fsite project grant PROJECT --email EMAIL [--role editor] [--send-invite] [--output json]\n\nGrant Project Repository edit access to an External Principal email. Use --send-invite to email agent-facing auth/git instructions."
}

fn project_revoke_help() -> &'static str {
    "usage: fsite project revoke PROJECT --email EMAIL [--output json]\n\nRemove Project Repository edit access for an External Principal email and revoke active Git Credentials. Safe to replay: removed=false means the collaborator was already inactive or unknown."
}

fn project_share_help() -> &'static str {
    "usage: fsite project share PROJECT OUTPUT [--public --yes-public|--shared|--private] [--add-email EMAIL]... [--remove-email EMAIL]... [--add-npub NPUB]... [--remove-npub NPUB]... [--send-invite] [--output json]\n\nManage revocable viewer Shares for one Project Output. Email viewers use magic links; Native Principal npubs use bounded Sites viewer sessions without email. This is separate from Project Repository edit access. Use OUTPUT from finite.toml or fsite project status. Public sharing requires --yes-public."
}

fn project_status_help() -> &'static str {
    "usage: fsite project status PROJECT [--output json]\n\nShow Project Repository control-plane state: git remote, actor role, repository visibility, declared outputs, output URLs, branch/path, output visibility, and active version."
}

fn project_list_help() -> &'static str {
    "usage: fsite project list [--output json]\n\nList Project Repositories this actor owns or may edit."
}

fn auth_help() -> &'static str {
    "usage:\n  fsite auth status [--output json]\n  fsite auth import [--file PATH] [--output json]\n  fsite auth register [--output json]\n  fsite auth link-email EMAIL [--output json]\n  fsite auth login EMAIL\n  fsite auth redeem EMAIL TOKEN [--link-native] [--output json]\n  fsite auth git PROJECT [--email EMAIL] [--store] [--output json]\n\nAuthenticate this machine for Finite Sites. Status shows the shared Finite identity (User Key) every Finite tool uses; import adopts an existing secret as that identity. Register creates a native Publishing Principal for the local User Key. Link-email pairs a verified email with that native Principal. Email login remains the External Principal fallback. Git auth mints a scoped HTTPS Git Credential for one Project Repository."
}

fn auth_register_help() -> &'static str {
    "usage: fsite auth register [--output json]\n\nSelf-register the local User Key as a Publishing Principal. This creates or replays a self-registered publish grant with the default output limit."
}

fn auth_link_email_help() -> &'static str {
    "usage: fsite auth link-email EMAIL [--output json]\n\nSend a verification token so `fsite auth redeem EMAIL TOKEN` can link that email to the local native Principal. Run `fsite auth register` first when you want the email paired with this npub."
}

fn auth_git_help() -> &'static str {
    "usage: fsite auth git PROJECT [--email EMAIL] [--store] [--output json]\n\nReturns git_remote_url, username, and password for standard git clone/push against one Project Repository. With --store, saves the scoped credential to Finite's file-backed git credential store ($FINITE_HOME/git-credentials, else ~/.finite/git-credentials) and configures git to use only that store for the Finite Git host; the OS keychain is never touched, so no interactive credential UI can appear. The password is omitted from output."
}

fn auth_login_help() -> &'static str {
    "usage: fsite auth login EMAIL\n\nRequest a one-time email verification token for an External Principal."
}

fn auth_redeem_help() -> &'static str {
    "usage: fsite auth redeem EMAIL TOKEN [--link-native] [--output json]\n\nVerify this machine for an email token. By default this verifies an Email Key for the External Principal fallback. Use --link-native after `fsite auth register` to link the email to the local native Principal instead."
}

fn view_help() -> &'static str {
    "usage: fsite view URL_OR_NAME [--output json]\n\nInspect a served Project Output URL or routing name. This is read-only; project editing happens through git after fsite auth git."
}

fn describe(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(describe_help());
    }
    let mut positionals: Vec<&String> = Vec::new();
    let mut output_json = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag `{other}`")));
            }
            _ => {
                positionals.push(&args[index]);
                index += 1;
            }
        }
    }

    let value = match positionals.as_slice() {
        [] => describe_commands(),
        [workflow, name] if workflow.as_str() == "workflow" => describe_workflow(name)?,
        _ => {
            return Err(CliError::Usage(
                "usage: fsite describe [workflow NAME] [--output json]".to_string(),
            ));
        }
    };
    if output_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("describe json serializes")
        );
    } else {
        println!("machine-readable description (use --output json in agent workflows):");
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("describe json serializes")
        );
    }
    Ok(())
}

fn describe_commands() -> serde_json::Value {
    serde_json::json!({
        "commands": [
            {
                "name": "project init",
                "summary": "Initialize a Project Repository and any finite.toml-described outputs.",
                "usage": "fsite project init --config finite.toml [--owner-viewer-npub NPUB] [--dry-run] [--output json]"
            },
            {
                "name": "project grant",
                "summary": "Grant Project Repository edit access to an External Principal email.",
                "usage": "fsite project grant PROJECT --email EMAIL [--role editor] [--send-invite] [--output json]"
            },
            {
                "name": "project revoke",
                "summary": "Remove Project Repository edit access and revoke active Git Credentials for that Principal.",
                "usage": "fsite project revoke PROJECT --email EMAIL [--output json]"
            },
            {
                "name": "project share",
                "summary": "Manage viewer access for one Project Output.",
                "usage": "fsite project share PROJECT OUTPUT [--public --yes-public|--shared|--private] [--add-email EMAIL]... [--remove-email EMAIL]... [--add-npub NPUB]... [--remove-npub NPUB]... [--send-invite] [--output json]"
            },
            {
                "name": "project status",
                "summary": "Show Project Repository, output, and deploy state.",
                "usage": "fsite project status PROJECT [--output json]"
            },
            {
                "name": "project list",
                "summary": "List Project Repositories this actor owns or may edit.",
                "usage": "fsite project list [--output json]"
            },
            {
                "name": "auth status",
                "summary": "Show the shared Finite identity (User Key) npub and file location, minting it on first use.",
                "usage": "fsite auth status [--output json]"
            },
            {
                "name": "auth import",
                "summary": "Adopt an existing nsec1.../hex secret (from stdin or --file, e.g. a legacy identity.env) as the shared Finite identity.",
                "usage": "fsite auth import [--file PATH] [--output json]"
            },
            {
                "name": "auth register",
                "summary": "Self-register the local User Key as a native Publishing Principal.",
                "usage": "fsite auth register [--output json]"
            },
            {
                "name": "auth link-email",
                "summary": "Send a verification token to link an email to the local native Principal.",
                "usage": "fsite auth link-email EMAIL [--output json]"
            },
            {
                "name": "auth login",
                "summary": "Request an email verification token for an External Principal.",
                "usage": "fsite auth login EMAIL"
            },
            {
                "name": "auth redeem",
                "summary": "Verify this machine's Email Key and link the email when using a native Principal.",
                "usage": "fsite auth redeem EMAIL TOKEN [--link-native] [--output json]"
            },
            {
                "name": "auth git",
                "summary": "Mint a scoped HTTPS Git Credential for a native Project Collaborator or verified External Principal.",
                "usage": "fsite auth git PROJECT [--email EMAIL] [--store] [--output json]"
            },
            {
                "name": "view",
                "summary": "Inspect a served Project Output URL or routing name without mutating state.",
                "usage": "fsite view URL_OR_NAME [--output json]"
            },
            {
                "name": "describe workflow",
                "summary": "Print machine-readable workflow guidance.",
                "usage": "fsite describe workflow NAME --output json"
            }
        ],
        "workflows": [
            "register-and-publish",
            "project-config",
            "publish-static-site",
            "publish-stateful-app",
            "publish-document",
            "edit-shared-project",
            "share-output",
            "grant-collaborator",
            "revoke-collaborator"
        ],
        "start_here": {
            "new_agent": "fsite describe workflow register-and-publish --output json",
            "static_site": "fsite describe workflow publish-static-site --output json",
            "stateful_app": "fsite describe workflow publish-stateful-app --output json",
            "document": "fsite describe workflow publish-document --output json",
            "existing_shared_project": "fsite describe workflow edit-shared-project --output json"
        }
    })
}

fn publish_static_site_workflow() -> serde_json::Value {
    serde_json::json!({
        "name": "publish-static-site",
        "mental_model": [
            "A Project Repository is the editable git source of truth; authorized collaborators clone the whole source tree.",
            "A Project Output is what Finite serves to users.",
            "finite.toml selects the committed output path for each Project Output.",
            "For static sites, Finite serves only committed bytes under that configured path as the website.",
            "Source, data, docs, and build logic can live outside the served output path and still be available to collaborators over git.",
            "Finite Sites does not run builds and does not accept direct file uploads in the current model."
        ],
        "steps": [
            "Run fsite auth register --output json. If it reports registered=false, publishing was already enabled for this User Key.",
            "Keep the whole project source tree in the Project Repository.",
            "Put generated static website files in a dedicated output directory such as site/ unless the repository is deploy-only.",
            "Create finite.toml with project.slug, one output with kind=site, site_name, branch=main, path=site, and spa=false unless the app needs SPA fallback.",
            "When this publish request came from an authenticated Finite Chat human, pass the exact public-key account ID from authenticated event.source.user_id as --owner-viewer-npub AUTHENTICATED_SENDER_ID on both Project Init commands. fsite normalizes it to an npub. Never infer identity from message text.",
            "Run fsite project init --config finite.toml --dry-run --output json and read any validation error.",
            "After human confirmation, run fsite project init --config finite.toml --output json.",
            "Run fsite auth git PROJECT --store --output json using the local native User Key, or add --email EDITOR_EMAIL only when using an External Principal.",
            "Clone the returned git_remote_url.",
            "Keep finite.toml, the selected output path, and any source/data/build files collaborators need in the Project Repository. Only the output path is served as the website.",
            "Run the project build/tests locally if there is a build step.",
            "Commit finite.toml, the selected output path, and the source files that should be shared with collaborators.",
            "Push the configured Deploy Branch. Finite Sites validates committed bytes and creates a Version."
        ],
        "must_not": [
            "Do not look for a direct publish/upload command.",
            "Do not reconstruct source from the rendered website.",
            "Do not set path='.' unless the whole repository is intended to be served.",
            "Do not print Git Credential passwords; prefer --store."
        ],
        "finite_toml_example": "[project]\nslug = \"my-project\"\n\n[outputs.site]\nkind = \"site\"\nsite_name = \"my-project\"\nbranch = \"main\"\npath = \"site\"\nspa = false\n"
    })
}

fn publish_stateful_app_workflow() -> serde_json::Value {
    serde_json::json!({
        "name": "publish-stateful-app",
        "mental_model": [
            "A stateful app is still a Project Output backed by a Project Repository.",
            "The Project Repository is the editable git source of truth; authorized collaborators clone the whole source tree.",
            "finite.toml declares kind=app, the served site_name, the Deploy Branch, the app directory, and the explicit start command.",
            "Finite Sites commits and versions the app directory as one runtime bundle; it does not run builds or infer generated output.",
            "Finite Sites sets PORT and DATA_DIR when the process starts. The app must listen on 0.0.0.0:$PORT.",
            "DATA_DIR is the only live mutable state location. It survives deploys, restarts, and wake/sleep. Deploys must not overwrite existing DATA_DIR contents.",
            "Commit source, migrations, seed data, and explicit runtime payload to git. Write user/live state under DATA_DIR at runtime."
        ],
        "steps": [
            "Run fsite auth register --output json. If it reports registered=false, publishing was already enabled for this User Key.",
            "Put the app runtime files in a dedicated directory such as app/. The directory must contain everything the start command needs, or code that explicitly initializes dependencies under DATA_DIR at runtime.",
            "Create finite.toml with project.slug, one output with kind=app, site_name, branch=main, path=app, and start=\"bun server.ts\" or another supported command beginning with node, bun, or uv.",
            "When this publish request came from an authenticated Finite Chat human, pass the exact public-key account ID from authenticated event.source.user_id as --owner-viewer-npub AUTHENTICATED_SENDER_ID on both Project Init commands. fsite normalizes it to an npub. Never infer identity from message text.",
            "Run fsite project init --config finite.toml --dry-run --output json and read any validation error.",
            "After human confirmation, run fsite project init --config finite.toml --output json.",
            "Run fsite auth git PROJECT --store --output json using the local native User Key, or add --email EDITOR_EMAIL only when using an External Principal.",
            "Clone the returned git_remote_url.",
            "Commit finite.toml, app source, migrations, seed data, and any explicit runtime payload.",
            "Push the configured Deploy Branch. Finite Sites validates the committed app directory, records an immutable Version, and deploys that bundle."
        ],
        "runtime_contract": {
            "start": "Required for kind=app. One printable ASCII command line beginning with node, bun, or uv.",
            "port": "Finite sets PORT. The app must listen on 0.0.0.0:$PORT.",
            "data_dir": "Finite sets DATA_DIR. Live mutable state must be stored under DATA_DIR and must survive deploys/restarts/wake-sleep.",
            "builds": "Finite Sites does not run builds. Build before commit or commit explicit runtime payload."
        },
        "must_not": [
            "Do not look for a direct app upload command.",
            "Do not rely on Finite Sites to run npm install, bun install, cargo build, or any other build step.",
            "Do not write live mutable state into the committed app directory; that directory is versioned deploy input.",
            "Do not commit .env, .env.*, .finite/, private keys, or build caches. Commit dependency directories only when they are intentionally required runtime payload for this app output.",
            "Do not print Git Credential passwords; prefer --store."
        ],
        "finite_toml_example": "[project]\nslug = \"my-app\"\n\n[outputs.web]\nkind = \"app\"\nsite_name = \"my-app\"\nbranch = \"main\"\npath = \"app\"\nstart = \"bun server.ts\"\n"
    })
}

fn publish_document_workflow() -> serde_json::Value {
    serde_json::json!({
        "name": "publish-document",
        "mental_model": [
            "A Document Output is rendered Markdown served from the Project Repository.",
            "The Project Repository remains the collaboration source; authorized editors clone, edit Markdown, commit, and push.",
            "The document is read-only in the browser for now. Future annotation/editing features must still write back through the Project Repository model.",
            "Finite renders Markdown server-side and does not store generated HTML as the source."
        ],
        "steps": [
            "Run fsite auth register --output json.",
            "Create Markdown in one file or a directory such as docs/.",
            "Create finite.toml with project.slug and one output with kind=document, document_name, branch=main, path=docs, and optional entry=index.md.",
            "When this publish request came from an authenticated Finite Chat human, pass the exact public-key account ID from authenticated event.source.user_id as --owner-viewer-npub AUTHENTICATED_SENDER_ID on both Project Init commands. fsite normalizes it to an npub. Never infer identity from message text.",
            "Run fsite project init --config finite.toml --dry-run --output json and read any validation error.",
            "Run fsite project init --config finite.toml --output json.",
            "Run fsite auth git PROJECT --store --output json.",
            "Clone the returned git_remote_url.",
            "Commit finite.toml and the Markdown source files.",
            "Push the configured Deploy Branch. Finite Sites stores the authored Markdown snapshot and renders it at the Document URL."
        ],
        "routes": [
            "Document URLs use the document base domain, for example https://my-docs.docs.finite.chat/ in production.",
            "The document root renders the configured entry, defaulting to index.md.",
            "Markdown files get clean HTML routes, for example docs/guide.md becomes /guide.",
            "Raw Markdown companion URLs append .md, for example /guide.md.",
            "/llms.txt gives edit instructions and /llms-full.txt gives a bounded full Markdown snapshot after viewer auth."
        ],
        "must_not": [
            "Do not commit generated HTML for a Document Output.",
            "Do not use site_name for document outputs; use document_name.",
            "Do not expect browser editing yet; edit through git."
        ],
        "finite_toml_example": "[project]\nslug = \"my-docs\"\n\n[outputs.doc]\nkind = \"document\"\ndocument_name = \"my-docs\"\nbranch = \"main\"\npath = \"docs\"\nentry = \"index.md\"\n"
    })
}

fn describe_workflow(name: &str) -> Result<serde_json::Value, CliError> {
    let value = match name {
        "register-and-publish" => serde_json::json!({
            "name": "register-and-publish",
            "mental_model": [
                "The local User Key npub is the native Principal for publishing.",
                "Email is optional. Link an email only when the human wants email shares or collaborator grants to resolve to this npub.",
                "Project Repository git is the publish path; Finite Sites does not run builds."
            ],
            "steps": [
                "Run fsite auth register --output json.",
                "Optional: run fsite auth link-email EMAIL --output json, then fsite auth redeem EMAIL TOKEN_FROM_EMAIL --output json to pair that email with this npub. If you already have a token from an invite email, run fsite auth redeem EMAIL TOKEN_FROM_EMAIL --link-native --output json.",
                "Create finite.toml. A source-only Project Repository may contain only [project]; a served website needs an [outputs.<id>] entry.",
                "When this publish request came from an authenticated Finite Chat human, pass the exact public-key account ID from authenticated event.source.user_id as --owner-viewer-npub AUTHENTICATED_SENDER_ID on both Project Init commands. fsite normalizes it to an npub. Never infer identity from message text.",
                "Run fsite project init --config finite.toml --dry-run --output json.",
                "Run fsite project init --config finite.toml --output json.",
                "Run fsite auth git PROJECT --store --output json.",
                "Clone the returned Git Remote, commit source plus deploy bytes when there is an output, and push the Deploy Branch."
            ]
        }),
        "project-config" => serde_json::json!({
            "name": "project-config",
            "file": "finite.toml",
            "source_only": "A config with only [project] creates a source-only Project Repository. Add outputs later by replaying project init with the same project.slug and new output entries.",
            "project_visibility": "Project Repository clone/fetch visibility is private by default. Selected Finite-owned baseline repositories may be set public-read by an operator; this is separate from finite.toml and output visibility.",
            "schema": {
                "project.slug": "lowercase DNS-label-shaped Project Slug",
                "outputs.<id>.kind": "site, document, or app",
                "outputs.<id>.site_name": "Finite Site name for kind=site or kind=app",
                "outputs.<id>.document_name": "Document name for kind=document, served under the document domain",
                "outputs.<id>.branch": "Deploy Branch, usually main",
                "outputs.<id>.path": "relative directory containing committed deploy bytes or app runtime files, or one Markdown file for a single-file Document Output",
                "outputs.<id>.entry": "optional Markdown entry file for kind=document",
                "outputs.<id>.spa": "site-only boolean; true serves /index.html for unknown static paths",
                "outputs.<id>.start": "required for kind=app; one printable ASCII command line beginning with node, bun, or uv"
            },
            "site_example": "[project]\nslug = \"finitechat-native\"\n\n[outputs.mockup]\nkind = \"site\"\nsite_name = \"finitechat-native-mockup\"\nbranch = \"main\"\npath = \".\"\nspa = false\n",
            "document_example": "[project]\nslug = \"hermes-notes\"\n\n[outputs.doc]\nkind = \"document\"\ndocument_name = \"hermes\"\nbranch = \"main\"\npath = \"docs\"\nentry = \"index.md\"\n",
            "app_example": "[project]\nslug = \"tiny-crm\"\n\n[outputs.web]\nkind = \"app\"\nsite_name = \"tiny-crm\"\nbranch = \"main\"\npath = \"app\"\nstart = \"bun server.ts\"\n"
        }),
        "publish-static-site" => publish_static_site_workflow(),
        "publish-stateful-app" => publish_stateful_app_workflow(),
        "publish-document" => publish_document_workflow(),
        "edit-shared-project" => serde_json::json!({
            "name": "edit-shared-project",
            "steps": [
                "If you are a native Project Collaborator, run fsite auth git PROJECT --store --output json.",
                "If you are using an External Principal email, run fsite auth login EDITOR_EMAIL if this machine is not verified.",
                "For email-only auth, run fsite auth redeem EDITOR_EMAIL TOKEN_FROM_EMAIL.",
                "If you want this email to resolve to the local npub, first run fsite auth register --output json, then fsite auth redeem EDITOR_EMAIL TOKEN_FROM_EMAIL --link-native --output json.",
                "For email auth, run fsite auth git PROJECT --email EDITOR_EMAIL --store --output json.",
                "Clone using the returned git_remote_url; the password is stored in Finite's file-backed git credential store and is not printed.",
                "Clone, edit source, run the project's tests/build, commit deploy bytes, and push the Deploy Branch."
            ]
        }),
        "grant-collaborator" => serde_json::json!({
            "name": "grant-collaborator",
            "steps": [
                "Use the Project owner identity, not the collaborator email key.",
                "Run fsite project grant PROJECT --email COLLABORATOR_EMAIL --role editor --send-invite --output json.",
                "Preferred native path: the collaborator runs fsite auth register --output json, then fsite auth redeem COLLABORATOR_EMAIL TOKEN_FROM_EMAIL --link-native --output json, then fsite auth git PROJECT --store --output json.",
                "Email-only fallback: the collaborator runs fsite auth redeem COLLABORATOR_EMAIL TOKEN_FROM_EMAIL --output json, then fsite auth git PROJECT --email COLLABORATOR_EMAIL --store --output json."
            ]
        }),
        "share-output" => serde_json::json!({
            "name": "share-output",
            "mental_model": [
                "Project Repository edit access and Project Output viewer access are separate.",
                "Use the Project owner identity to change viewer access.",
                "OUTPUT is the output id from finite.toml or fsite project status, not the site DNS name."
            ],
            "steps": [
                "Run fsite project status PROJECT --output json and choose the output_id to share.",
                "For public viewer access, run fsite project share PROJECT OUTPUT --public --yes-public --output json.",
                "For email-gated viewer access, run fsite project share PROJECT OUTPUT --shared --add-email VIEWER_EMAIL --send-invite --output json.",
                "For native Finite viewer access without email, run fsite project share PROJECT OUTPUT --add-npub VIEWER_NPUB --output json. Remove it with --remove-npub.",
                "For private viewer access, run fsite project share PROJECT OUTPUT --private --output json."
            ]
        }),
        "revoke-collaborator" => serde_json::json!({
            "name": "revoke-collaborator",
            "steps": [
                "Use the Project owner identity, not the collaborator email key.",
                "Run fsite project revoke PROJECT --email COLLABORATOR_EMAIL --output json.",
                "Check removed and revoked_git_credentials in the JSON response."
            ]
        }),
        other => {
            return Err(CliError::Usage(format!(
                "unknown workflow `{other}` (register-and-publish|project-config|publish-static-site|publish-stateful-app|publish-document|edit-shared-project|share-output|grant-collaborator|revoke-collaborator)"
            )));
        }
    };
    Ok(value)
}

fn project_command(args: &[String]) -> Result<(), CliError> {
    let Some((subcommand, rest)) = args.split_first() else {
        return Err(CliError::Usage(project_help().to_string()));
    };
    match subcommand.as_str() {
        value if is_help_arg(value) => print_help(project_help()),
        "init" => project_init(rest),
        "grant" => project_grant(rest),
        "revoke" => project_revoke(rest),
        "share" => project_share(rest),
        "status" => project_status(rest),
        "list" => project_list(rest),
        "apply" | "collaborator" => Err(CliError::Usage(removed_site_first_command_help(
            &format!("project {subcommand}"),
        ))),
        other => Err(CliError::Usage(format!(
            "unknown project command `{other}`"
        ))),
    }
}

fn project_init(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(project_init_help());
    }
    let mut config_path: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut owner_viewer_npub: Option<String> = None;
    let mut output_json = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--config" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--config needs a path".to_string()))?;
                config_path = Some(PathBuf::from(value));
                index += 2;
            }
            "--dry-run" => {
                dry_run = true;
                index += 1;
            }
            "--owner-viewer-npub" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    CliError::Usage("--owner-viewer-npub needs an npub".to_string())
                })?;
                if owner_viewer_npub.replace(value.clone()).is_some() {
                    return Err(CliError::Usage(
                        "--owner-viewer-npub may be supplied only once".to_string(),
                    ));
                }
                index += 2;
            }
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            other => return Err(CliError::Usage(format!("unknown flag `{other}`"))),
        }
    }

    let config_path =
        config_path.ok_or_else(|| CliError::Usage(project_init_help().to_string()))?;
    let config = read_project_config_file(&config_path)?;
    let request = ProjectInitRequest {
        config,
        dry_run,
        owner_viewer_npub,
    };
    request
        .config
        .validate()
        .map_err(|error| CliError::Usage(error.to_string()))?;

    let identity = keys::load_or_generate_user_key()?;
    let client = api::Client::from_env();
    let recovery_owner_viewer_npub = request.owner_viewer_npub.clone();
    let response = client.init_project(&identity, &request).map_err(|error| {
        project_init_recovery_error(error, &config_path, recovery_owner_viewer_npub.as_deref())
    })?;
    if output_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).expect("response serializes")
        );
    } else {
        println!("project: {}", response.slug);
        println!("source:  {}", response.project_visibility);
        println!("git:     {}", response.git_remote_url);
        if let Some(npub) = &response.owner_viewer_npub {
            println!("owner viewer: {npub} (shared)");
        }
        if response.outputs.is_empty() {
            println!("outputs: none (source-only Project Repository)");
        }
        for output in &response.outputs {
            println!(
                "output:  {} {} -> {} ({}:{})",
                output.output_id,
                output.kind,
                display_output_url(output),
                output.branch,
                output.path
            );
        }
        if response.dry_run {
            println!("dry-run: no server state changed");
        } else {
            println!(
                "next:    fsite auth git {} --store --output json",
                response.slug
            );
            if response.outputs.is_empty() {
                println!(
                    "source:  commit {} and project source, then push git normally; no output will serve until finite.toml declares one",
                    config_path.display()
                );
            } else {
                println!(
                    "publish: commit {} and push the Deploy Branch",
                    config_path.display()
                );
            }
        }
    }
    Ok(())
}

fn project_init_recovery_error(
    error: CliError,
    config_path: &Path,
    owner_viewer_npub: Option<&str>,
) -> CliError {
    let CliError::ApiStatus {
        method,
        path,
        status,
        code,
        mut message,
    } = error
    else {
        return error;
    };

    let owner_viewer_arg = owner_viewer_npub
        .map(|npub| format!(" --owner-viewer-npub {npub}"))
        .unwrap_or_default();
    match code.as_deref() {
        Some(ERROR_GIT_REPOSITORY_SETUP_FAILED) => message.push_str(&format!(
            "\n\nThe Project may already exist; do not change its slug or discard local source. After the service operator reports that Git repository setup is repaired, run this repair replay exactly once:\n  fsite project init --config {}{} --output json",
            config_path.display(), owner_viewer_arg
        )),
        Some(ERROR_GIT_UNAVAILABLE) => message.push_str(&format!(
            "\n\nNo Project Init state changed. After service health has recovered, retry exactly once:\n  fsite project init --config {}{} --output json",
            config_path.display(), owner_viewer_arg
        )),
        _ => {}
    }

    CliError::ApiStatus {
        method,
        path,
        status,
        code,
        message,
    }
}

fn read_project_config_file(
    path: &Path,
) -> Result<finitesites_proto::project_config::ProjectConfig, CliError> {
    let existing = std::fs::read_to_string(path)
        .map_err(|error| CliError::Io(format!("cannot read {}: {error}", path.display())))?;
    parse_project_config_toml(&existing)
        .map_err(|error| CliError::Usage(format!("{} is invalid: {error}", path.display())))
}

fn project_grant(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(project_grant_help());
    }
    let mut project: Option<String> = None;
    let mut email: Option<String> = None;
    let mut role = "editor".to_string();
    let mut send_invite = false;
    let mut output_json = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--email" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--email needs a value".to_string()))?;
                email = Some(value.clone());
                index += 2;
            }
            "--role" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--role needs a value".to_string()))?;
                role = value.clone();
                index += 2;
            }
            "--send-invite" => {
                send_invite = true;
                index += 1;
            }
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag `{other}`")));
            }
            value => {
                if project.is_some() {
                    return Err(CliError::Usage(project_grant_help().to_string()));
                }
                project = Some(value.to_string());
                index += 1;
            }
        }
    }
    let project = project.ok_or_else(|| CliError::Usage(project_grant_help().to_string()))?;
    let email = email.ok_or_else(|| CliError::Usage(project_grant_help().to_string()))?;
    let identity = keys::load_or_generate_user_key()?;
    let client = api::Client::from_env();
    let response = client.grant_project(
        &identity,
        &project,
        &ProjectGrantRequest { email, role },
        send_invite,
    )?;
    if output_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).expect("response serializes")
        );
    } else {
        println!("project: {}", response.project_slug);
        println!("email:   {}", response.collaborator.email);
        println!("role:    {}", response.collaborator.role);
        println!("created: {}", response.collaborator.created);
        if !response.invited_emails.is_empty() {
            println!("invited: {}", response.invited_emails.join(", "));
        }
    }
    Ok(())
}

fn project_revoke(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(project_revoke_help());
    }
    let mut project: Option<String> = None;
    let mut email: Option<String> = None;
    let mut output_json = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--email" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--email needs a value".to_string()))?;
                email = Some(value.clone());
                index += 2;
            }
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag `{other}`")));
            }
            value => {
                if project.is_some() {
                    return Err(CliError::Usage(project_revoke_help().to_string()));
                }
                project = Some(value.to_string());
                index += 1;
            }
        }
    }

    let project = project.ok_or_else(|| CliError::Usage(project_revoke_help().to_string()))?;
    let email = email.ok_or_else(|| CliError::Usage(project_revoke_help().to_string()))?;
    let identity = keys::load_or_generate_user_key()?;
    let client = api::Client::from_env();
    let response = client.revoke_project(&identity, &project, &ProjectRevokeRequest { email })?;
    if output_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).expect("response serializes")
        );
    } else {
        println!("project: {}", response.project_slug);
        println!("email:   {}", response.email);
        println!("removed: {}", response.removed);
        println!(
            "revoked git credentials: {}",
            response.revoked_git_credentials
        );
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ProjectShareOptions {
    project: String,
    output_id: String,
    visibility: Option<String>,
    confirm_public: bool,
    add_emails: Vec<String>,
    remove_emails: Vec<String>,
    add_npubs: Vec<String>,
    remove_npubs: Vec<String>,
    send_invite: bool,
    output_json: bool,
}

fn project_share(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(project_share_help());
    }
    let options = parse_project_share_args(args)?;
    let identity = keys::load_or_generate_user_key()?;
    let client = api::Client::from_env();
    let response = client.share_project_output(
        &identity,
        &options.project,
        &options.output_id,
        &SharingRequest {
            visibility: options.visibility,
            confirm_public: options.confirm_public,
            add_emails: options.add_emails,
            remove_emails: options.remove_emails,
            add_npubs: options.add_npubs,
            remove_npubs: options.remove_npubs,
        },
        options.send_invite,
    )?;
    if options.output_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).expect("response serializes")
        );
    } else {
        println!("project:    {}", response.project_slug);
        println!("output:     {}", response.output_id);
        println!("visibility: {}", response.visibility);
        if response.shared_emails.is_empty() && response.shared_npubs.is_empty() {
            println!("shared with: none");
        } else {
            let mut viewers = response.shared_emails.clone();
            viewers.extend(response.shared_npubs.clone());
            println!("shared with: {}", viewers.join(", "));
        }
        if !response.invited_emails.is_empty() {
            println!("invited: {}", response.invited_emails.join(", "));
        }
    }
    Ok(())
}

fn parse_project_share_args(args: &[String]) -> Result<ProjectShareOptions, CliError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut visibility: Option<String> = None;
    let mut confirm_public = false;
    let mut add_emails = Vec::new();
    let mut remove_emails = Vec::new();
    let mut add_npubs = Vec::new();
    let mut remove_npubs = Vec::new();
    let mut send_invite = false;
    let mut output_json = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--public" => {
                set_visibility_once(&mut visibility, "public")?;
                index += 1;
            }
            "--shared" => {
                set_visibility_once(&mut visibility, "shared")?;
                index += 1;
            }
            "--private" => {
                set_visibility_once(&mut visibility, "private")?;
                index += 1;
            }
            "--yes-public" => {
                confirm_public = true;
                index += 1;
            }
            "--send-invite" => {
                send_invite = true;
                index += 1;
            }
            "--add-email" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--add-email needs an email".to_string()))?;
                add_emails.push(value.clone());
                index += 2;
            }
            "--remove-email" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--remove-email needs an email".to_string()))?;
                remove_emails.push(value.clone());
                index += 2;
            }
            "--add-npub" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--add-npub needs an npub".to_string()))?;
                add_npubs.push(value.clone());
                index += 2;
            }
            "--remove-npub" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--remove-npub needs an npub".to_string()))?;
                remove_npubs.push(value.clone());
                index += 2;
            }
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag `{other}`")));
            }
            value => {
                positionals.push(value.to_string());
                index += 1;
            }
        }
    }
    if positionals.len() != 2 {
        return Err(CliError::Usage(project_share_help().to_string()));
    }
    if confirm_public && visibility.as_deref() != Some("public") {
        return Err(CliError::Usage(
            "--yes-public is only valid with --public".to_string(),
        ));
    }
    if visibility.as_deref() == Some("public") && !confirm_public {
        return Err(CliError::Usage(
            "--public requires --yes-public to confirm public viewer access".to_string(),
        ));
    }
    if send_invite {
        if visibility.as_deref() != Some("shared") {
            return Err(CliError::Usage(
                "--send-invite requires --shared".to_string(),
            ));
        }
        if add_emails.is_empty() {
            return Err(CliError::Usage(
                "--send-invite requires at least one --add-email".to_string(),
            ));
        }
    }
    Ok(ProjectShareOptions {
        project: positionals.remove(0),
        output_id: positionals.remove(0),
        visibility,
        confirm_public,
        add_emails,
        remove_emails,
        add_npubs,
        remove_npubs,
        send_invite,
        output_json,
    })
}

fn set_visibility_once(target: &mut Option<String>, value: &str) -> Result<(), CliError> {
    if target.is_some() {
        return Err(CliError::Usage(
            "choose only one visibility flag".to_string(),
        ));
    }
    *target = Some(value.to_string());
    Ok(())
}

fn project_status(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(project_status_help());
    }
    let (project, output_json) = parse_project_read_args(args, project_status_help())?;
    let identity = keys::load_or_generate_user_key()?;
    let client = api::Client::from_env();
    let response = client.project_status(&identity, &project)?;
    if output_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).expect("response serializes")
        );
    } else {
        println!("project: {}", response.slug);
        println!("role:    {}", response.role);
        println!("source:  {}", response.project_visibility);
        println!("git:     {}", response.git_remote_url);
        print_project_outputs(&response.outputs);
        if !response.collaborators.is_empty() {
            println!("collaborators:");
            for collaborator in &response.collaborators {
                println!("  {} {}", collaborator.role, collaborator.email);
            }
        }
    }
    Ok(())
}

fn project_list(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(project_list_help());
    }
    let output_json = parse_output_json_only(args, project_list_help())?;
    let identity = keys::load_or_generate_user_key()?;
    let client = api::Client::from_env();
    let response = client.project_list(&identity)?;
    if output_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).expect("response serializes")
        );
    } else if response.projects.is_empty() {
        println!(
            "no projects yet; initialize one with `fsite project init --config finite.toml --dry-run --output json`"
        );
    } else {
        for project in &response.projects {
            println!(
                "{:<24} {:<8} {:<11} {}",
                project.slug, project.role, project.project_visibility, project.git_remote_url
            );
        }
    }
    Ok(())
}

fn parse_project_read_args(args: &[String], help: &str) -> Result<(String, bool), CliError> {
    let mut project: Option<String> = None;
    let mut output_json = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag `{other}`")));
            }
            value => {
                if project.is_some() {
                    return Err(CliError::Usage(help.to_string()));
                }
                project = Some(value.to_string());
                index += 1;
            }
        }
    }
    let project = project.ok_or_else(|| CliError::Usage(help.to_string()))?;
    Ok((project, output_json))
}

fn parse_output_json_only(args: &[String], help: &str) -> Result<bool, CliError> {
    let mut output_json = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            other => return Err(CliError::Usage(format!("{help}\nunknown flag `{other}`"))),
        }
    }
    Ok(output_json)
}

fn parse_email_and_output_json(args: &[String], help: &str) -> Result<(String, bool), CliError> {
    let mut email: Option<String> = None;
    let mut output_json = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag `{other}`")));
            }
            value => {
                if email.is_some() {
                    return Err(CliError::Usage(help.to_string()));
                }
                email = Some(value.to_string());
                index += 1;
            }
        }
    }
    let email = email.ok_or_else(|| CliError::Usage(help.to_string()))?;
    Ok((email, output_json))
}

fn print_project_outputs(outputs: &[finitesites_proto::dto::ProjectOutputSummary]) {
    if outputs.is_empty() {
        println!("outputs: none (source-only Project Repository)");
        return;
    }
    for output in outputs {
        let version = output
            .active_version
            .map(|value| format!("v{value}"))
            .unwrap_or_else(|| "unpublished".to_string());
        println!(
            "output:  {} {} {} {} {}:{}",
            output.output_id, output.kind, output.visibility, version, output.branch, output.path
        );
        println!("url:     {}", display_output_url(output));
    }
}

fn display_output_url(output: &finitesites_proto::dto::ProjectOutputSummary) -> &str {
    if output.output_url.is_empty() {
        &output.site_url
    } else {
        &output.output_url
    }
}

fn auth_command(args: &[String]) -> Result<(), CliError> {
    let Some((subcommand, rest)) = args.split_first() else {
        return Err(CliError::Usage(auth_help().to_string()));
    };
    match subcommand.as_str() {
        value if is_help_arg(value) => print_help(auth_help()),
        "status" => auth_status(rest),
        "import" => auth_import(rest),
        "register" => auth_register(rest),
        "link-email" => auth_link_email(rest),
        "login" => auth_login(rest),
        "redeem" => auth_redeem(rest),
        "git" => auth_git(rest),
        other => Err(CliError::Usage(format!("unknown auth command `{other}`"))),
    }
}

fn auth_register(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(auth_register_help());
    }
    let output_json = parse_output_json_only(args, auth_register_help())?;
    let identity = keys::load_or_generate_user_key()?;
    let client = api::Client::from_env();
    let response = client.register_auth(&identity)?;
    if output_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).expect("response serializes")
        );
    } else {
        println!("npub:       {}", response.npub);
        println!("principal:  {}", response.principal_id);
        println!("grant:      {}", response.grant_source);
        println!("registered: {}", response.registered);
        println!("limit:      {} outputs", response.output_limit);
    }
    Ok(())
}

fn auth_link_email(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(auth_link_email_help());
    }
    let (email, output_json) = parse_email_and_output_json(args, auth_link_email_help())?;
    let identity = keys::load_or_generate_user_key()?;
    let display =
        npub::encode_npub(&identity.pubkey).map_err(|error| CliError::Key(error.to_string()))?;
    let response = match api::IdentityAuthorityClient::from_env() {
        Some(identity_authority) => identity_authority.request_email_challenge(&email)?,
        None => api::Client::from_env().request_email_login(&email)?,
    };
    keys::write_pending_email_link(&response.email, &identity.pubkey)?;
    if output_json {
        let value = serde_json::json!({
            "email": response.email,
            "npub": display,
            "linking_native_principal": true,
            "next": format!("fsite auth redeem {} TOKEN_FROM_EMAIL --output json", response.email)
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("response serializes")
        );
    } else {
        println!("sent email verification for {}", response.email);
        println!("npub: {display}");
        println!("link: pending native email link stored on this machine");
        println!(
            "next: fsite auth redeem {} TOKEN_FROM_EMAIL --output json",
            response.email
        );
    }
    Ok(())
}

fn auth_git(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(auth_git_help());
    }
    let options = parse_auth_git_args(args)?;
    let key = match (&options.email, api::IdentityAuthorityClient::from_env()) {
        (Some(_), Some(_)) | (None, _) => keys::load_or_generate_user_key()?,
        (Some(email), None) => keys::load_or_create_email_key(email)?,
    };
    let client = api::Client::from_env();
    let response = client.auth_git(
        &key,
        &options.project,
        &GitAuthRequest {
            email: options.email.clone(),
        },
    )?;
    if options.store {
        store_git_credential(&response)?;
    }
    print_git_auth_response(&response, options.output_json, options.store)
}

#[derive(Debug, PartialEq, Eq)]
struct AuthGitOptions {
    project: String,
    email: Option<String>,
    output_json: bool,
    store: bool,
}

fn parse_auth_git_args(args: &[String]) -> Result<AuthGitOptions, CliError> {
    let mut positionals: Vec<&String> = Vec::new();
    let mut email: Option<String> = None;
    let mut output_json = false;
    let mut store = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--email" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--email needs a value".to_string()))?;
                email = Some(value.clone());
                index += 2;
            }
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            "--store" => {
                store = true;
                index += 1;
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag `{other}`")));
            }
            _ => {
                positionals.push(&args[index]);
                index += 1;
            }
        }
    }
    let [project] = positionals.as_slice() else {
        return Err(CliError::Usage(
            "usage: fsite auth git PROJECT [--email EMAIL] [--store] [--output json]".to_string(),
        ));
    };
    Ok(AuthGitOptions {
        project: (*project).clone(),
        email,
        output_json,
        store,
    })
}

fn print_git_auth_response(
    response: &GitAuthResponse,
    output_json: bool,
    stored: bool,
) -> Result<(), CliError> {
    if output_json {
        if stored {
            let value = serde_json::json!({
                "project_slug": response.project_slug,
                "git_remote_url": response.git_remote_url,
                "credential_id": response.credential_id,
                "username": response.username,
                "expires_at": response.expires_at,
                "stored": true
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("response serializes")
            );
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&response).expect("response serializes")
            );
        }
    } else {
        println!("git:      {}", response.git_remote_url);
        println!("username: {}", response.username);
        if stored {
            println!("password: stored in the Finite git credential store (never the OS keychain)");
            println!("clone:    git clone {}", response.git_remote_url);
        } else {
            println!("password: {}", response.password);
            println!("tip:      rerun with --store to save it without printing it");
        }
        println!("scope:    this credential works for this project only");
        println!("expires:  never, unless the Project Collaborator is removed");
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct GitCredentialContext {
    protocol: String,
    host: String,
    path: String,
}

fn git_credential_context(remote_url: &str) -> Result<GitCredentialContext, CliError> {
    let Some((protocol, rest)) = remote_url.split_once("://") else {
        return Err(CliError::Usage(
            "git_remote_url must include http:// or https://".to_string(),
        ));
    };
    if protocol != "http" && protocol != "https" {
        return Err(CliError::Usage(
            "git_remote_url must use http or https".to_string(),
        ));
    }
    let Some((host, raw_path)) = rest.split_once('/') else {
        return Err(CliError::Usage(
            "git_remote_url must include a repository path".to_string(),
        ));
    };
    if host.is_empty() || raw_path.is_empty() {
        return Err(CliError::Usage(
            "git_remote_url must include a host and repository path".to_string(),
        ));
    }
    Ok(GitCredentialContext {
        protocol: protocol.to_string(),
        host: host.to_string(),
        path: raw_path.to_string(),
    })
}

fn credential_store_path() -> Result<PathBuf, CliError> {
    // Same root convention as the shared identity (FINITE_HOME else
    // ~/.finite): agents get one predictable, file-backed credential store
    // that never touches an OS keychain.
    let paths = keys::identity_paths()?;
    let finite_root = paths
        .root()
        .parent()
        .ok_or_else(|| CliError::Key("identity root has no parent directory".to_string()))?;
    Ok(finite_root.join("git-credentials"))
}

fn set_private_file_permissions(path: &Path) -> Result<(), CliError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if path.exists() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                |error| CliError::Io(format!("cannot chmod {}: {error}", path.display())),
            )?;
        }
    }
    Ok(())
}

fn ensure_private_parent(path: &Path) -> Result<(), CliError> {
    let parent = path
        .parent()
        .ok_or_else(|| CliError::Io(format!("{} has no parent directory", path.display())))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| CliError::Io(format!("cannot create {}: {error}", parent.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| CliError::Io(format!("cannot chmod {}: {error}", parent.display())))?;
    }
    Ok(())
}

fn run_git_config(args: &[&str]) -> Result<(), CliError> {
    let command_args = git_config_command_args(args);
    let output = Command::new("git")
        .args(&command_args)
        .output()
        .map_err(|error| CliError::Io(format!("cannot run git config: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(CliError::Io(format!(
        "git config failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

fn git_config_command_args(args: &[&str]) -> Vec<String> {
    let mut command_args = Vec::with_capacity(args.len() + 1);
    command_args.push("config".to_string());
    // Bounded by the fixed git config invocations in this CLI.
    for arg in args {
        command_args.push((*arg).to_string());
    }
    command_args
}

fn configure_git_credential_storage(context: &GitCredentialContext) -> Result<PathBuf, CliError> {
    let store_path = credential_store_path()?;
    ensure_private_parent(&store_path)?;
    for op in git_credential_config_ops(context, &store_path) {
        let args: Vec<&str> = op.iter().map(String::as_str).collect();
        run_git_config(&args)?;
    }
    Ok(store_path)
}

/// Git accumulates credential helpers across system/global/local config, so
/// on macOS `git credential approve|fill` would also hit the system-level
/// osxkeychain helper, which can pop interactive keychain UI that no agent
/// can answer. The leading empty helper entry clears the inherited list for
/// the Finite git host, making our file-backed store the ONLY helper there.
fn git_credential_config_ops(
    context: &GitCredentialContext,
    store_path: &Path,
) -> Vec<Vec<String>> {
    let url = format!("{}://{}", context.protocol, context.host);
    let helper_key = format!("credential.{url}.helper");
    let path_key = format!("credential.{url}.useHttpPath");
    let helper_value = format!("store --file {}", store_path.display());
    vec![
        vec![
            "--global".to_string(),
            "--replace-all".to_string(),
            helper_key.clone(),
            String::new(),
        ],
        vec![
            "--global".to_string(),
            "--add".to_string(),
            helper_key,
            helper_value,
        ],
        vec![
            "--global".to_string(),
            "--replace-all".to_string(),
            path_key,
            "true".to_string(),
        ],
    ]
}

/// One git-credential-store line: `protocol://user:pass@host/path`.
/// Userinfo is percent-encoded so arbitrary credential bytes cannot break
/// the line format.
fn credential_store_line(context: &GitCredentialContext, username: &str, password: &str) -> String {
    format!(
        "{}://{}:{}@{}/{}",
        context.protocol,
        percent_encode_userinfo(username),
        percent_encode_userinfo(password),
        context.host,
        context.path
    )
}

fn percent_encode_userinfo(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    // Bounded by the input length.
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            other => {
                encoded.push('%');
                encoded.push_str(&format!("{other:02X}"));
            }
        }
    }
    encoded
}

/// Keep every stored credential except any previous one for the same
/// host+path, then append the new line.
fn merged_credential_store(existing: &str, line: &str, context: &GitCredentialContext) -> String {
    let target_suffix = format!("@{}/{}", context.host, context.path);
    let mut merged = String::new();
    // Bounded by the existing store size.
    for entry in existing.lines() {
        if entry.trim().is_empty() || entry.ends_with(&target_suffix) {
            continue;
        }
        merged.push_str(entry);
        merged.push('\n');
    }
    merged.push_str(line);
    merged.push('\n');
    merged
}

fn credential_fill_input(context: &GitCredentialContext) -> String {
    format!(
        "protocol={}\nhost={}\npath={}\n\n",
        context.protocol, context.host, context.path
    )
}

fn run_git_credential(command: &str, input: &str) -> Result<String, CliError> {
    let mut child = Command::new("git")
        .args(["credential", command])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| CliError::Io(format!("cannot run git credential {command}: {error}")))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| CliError::Io("cannot open git credential stdin".to_string()))?;
        stdin
            .write_all(input.as_bytes())
            .map_err(|error| CliError::Io(format!("cannot write git credential input: {error}")))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| CliError::Io(format!("git credential {command} failed: {error}")))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    Err(CliError::Io(format!(
        "git credential {command} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

fn credential_output_value(output: &str, key: &str) -> Option<String> {
    // Bounded by git credential's short key-value output.
    for line in output.lines() {
        if let Some(value) = line.strip_prefix(key)
            && let Some(value) = value.strip_prefix('=')
        {
            return Some(value.to_string());
        }
    }
    None
}

fn store_git_credential(response: &GitAuthResponse) -> Result<(), CliError> {
    let context = git_credential_context(&response.git_remote_url)?;
    let store_path = configure_git_credential_storage(&context)?;
    // Write the git-credential-store file directly instead of shelling to
    // `git credential approve`: approve fans out to every configured helper
    // (system osxkeychain included), and secrets must never leave our
    // file-backed store or trigger interactive credential UI.
    let existing = match std::fs::read_to_string(&store_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(CliError::Io(format!(
                "cannot read {}: {error}",
                store_path.display()
            )));
        }
    };
    let line = credential_store_line(&context, &response.username, &response.password);
    let merged = merged_credential_store(&existing, &line, &context);
    std::fs::write(&store_path, merged)
        .map_err(|error| CliError::Io(format!("cannot write {}: {error}", store_path.display())))?;
    set_private_file_permissions(&store_path)?;
    let filled = run_git_credential("fill", &credential_fill_input(&context))?;
    let filled_username = credential_output_value(&filled, "username")
        .ok_or_else(|| CliError::Io("git credential helper did not return username".to_string()))?;
    let filled_password = credential_output_value(&filled, "password")
        .ok_or_else(|| CliError::Io("git credential helper did not return password".to_string()))?;
    if filled_username != response.username || filled_password != response.password {
        return Err(CliError::Io(
            "git credential helper returned a different credential".to_string(),
        ));
    }
    Ok(())
}

fn whoami() -> Result<(), CliError> {
    let paths = keys::identity_paths()?;
    let identity = keys::load_or_generate_identity(&paths)?;
    println!("npub:   {}", identity.npub());
    println!("pubkey: {}", identity.public_key_hex());
    println!("file:   {}", paths.identity_file().display());
    println!();
    println!("publishing bootstrap: fsite auth register --output json");
    Ok(())
}

fn auth_status(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(auth_status_help());
    }
    let output_json = parse_output_json_only(args, auth_status_help())?;
    let paths = keys::identity_paths()?;
    let file = paths.identity_file();
    let Some(identity) = keys::load_identity(&paths)? else {
        // Status never mints (CLI-CONVENTIONS.md): report and point at the
        // normal first-run mint or auth import.
        if output_json {
            let value = serde_json::json!({
                "exists": false,
                "file": file.display().to_string(),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("identity json serializes")
            );
        } else {
            println!("no Finite identity yet");
            println!("file:  {} (not created)", file.display());
            println!("mint:  run any fsite command that publishes, or bring your own:");
            println!("       fsite auth import");
        }
        return Ok(());
    };
    if output_json {
        let value = serde_json::json!({
            "exists": true,
            "npub": identity.npub(),
            "pubkey": identity.public_key_hex(),
            "file": file.display().to_string(),
            "created_by": identity.created_by(),
            "created_at": identity.created_at(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("identity json serializes")
        );
    } else {
        println!("npub:       {}", identity.npub());
        println!("pubkey:     {}", identity.public_key_hex());
        println!("file:       {}", file.display());
        println!("created by: {}", identity.created_by());
        println!("created at: {}", identity.created_at());
        println!("shared:     every Finite tool uses this identity");
    }
    Ok(())
}

fn auth_import(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(auth_import_help());
    }
    let mut secret_file: Option<PathBuf> = None;
    let mut output_json = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--file" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--file needs a path".to_string()))?;
                secret_file = Some(PathBuf::from(value));
                index += 2;
            }
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            other => return Err(CliError::Usage(format!("unknown flag `{other}`"))),
        }
    }
    // The secret comes from stdin or a file, never argv: flag values leak
    // into ps output and shell history (finite-identity CLI-CONVENTIONS.md).
    let secret_text = match &secret_file {
        Some(path) => {
            let content = std::fs::read_to_string(path).map_err(|error| {
                CliError::Io(format!("cannot read {}: {error}", path.display()))
            })?;
            keys::import_secret_text_from_file(&content)
        }
        None => read_secret_line_from_stdin()?,
    };
    let paths = keys::identity_paths()?;
    let identity = keys::import_identity(&paths, &secret_text)?;
    let file = paths.identity_file();
    if output_json {
        let mut value = serde_json::json!({
            "npub": identity.npub(),
            "pubkey": identity.public_key_hex(),
            "file": file.display().to_string(),
        });
        if let Some(path) = &secret_file {
            value["imported_from"] = serde_json::json!(path.display().to_string());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("identity json serializes")
        );
    } else {
        if let Some(path) = &secret_file {
            println!("imported: {}", path.display());
            println!("note:     delete the old secret file once nothing else needs it");
        }
        println!("npub:     {}", identity.npub());
        println!("file:     {}", file.display());
    }
    Ok(())
}

/// Read one secret line from stdin for `fsite auth import`. Prompts on
/// stderr only when stdin is a terminal, so piped input stays silent and
/// stdout stays machine-parseable.
fn read_secret_line_from_stdin() -> Result<String, CliError> {
    use std::io::{BufRead as _, IsTerminal as _};
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        eprint!("secret (nsec1... or 64-char hex): ");
    }
    let mut line = String::new();
    let read = stdin
        .lock()
        .read_line(&mut line)
        .map_err(|error| CliError::Io(format!("cannot read secret from stdin: {error}")))?;
    if read == 0 {
        return Err(CliError::Usage(
            "no secret on stdin; pipe an nsec1... or 64-char hex secret, or use --file PATH"
                .to_string(),
        ));
    }
    Ok(line)
}

fn auth_login(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(auth_login_help());
    }
    let [email] = args else {
        return Err(CliError::Usage(auth_login_help().to_string()));
    };
    let response = match api::IdentityAuthorityClient::from_env() {
        Some(identity_authority) => identity_authority.request_email_challenge(email)?,
        None => api::Client::from_env().request_email_login(email)?,
    };
    println!("sent email login for {}", response.email);
    println!("run the fsite auth redeem command from the email to verify this machine");
    Ok(())
}

fn auth_redeem(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(auth_redeem_help());
    }
    let (email, token, link_native, output_json) = parse_redeem_args(args)?;
    let pending_link_pubkey = keys::pending_email_link_pubkey(&email)?;
    if let Some(identity_authority) = api::IdentityAuthorityClient::from_env() {
        let key = keys::load_or_generate_user_key()?;
        if let Some(pubkey) = &pending_link_pubkey
            && key.pubkey != *pubkey
        {
            return Err(CliError::Key(
                "pending email link belongs to a different local User Key; run `fsite auth link-email EMAIL` again from the machine that should own this email".to_string(),
            ));
        }
        let response =
            if (link_native || pending_link_pubkey.is_some()) && is_finite_vip_email(&email) {
                identity_authority.redeem_vip_email(&key, &email, &token)?
            } else {
                identity_authority.redeem_email_only(&key, &email, &token)?
            };
        if pending_link_pubkey.is_some() {
            keys::clear_pending_email_link(&email)?;
        }
        return print_email_redeem_response(&response, output_json);
    }
    let key = if link_native {
        keys::load_or_generate_user_key()?
    } else {
        match &pending_link_pubkey {
            Some(pubkey) => {
                let identity = keys::load_or_generate_user_key()?;
                if &identity.pubkey != pubkey {
                    return Err(CliError::Key(
                        "pending email link belongs to a different local User Key; run `fsite auth link-email EMAIL` again from the machine that should own this email".to_string(),
                    ));
                }
                identity
            }
            None => keys::load_or_create_email_key(&email)?,
        }
    };
    if link_native
        && let Some(pubkey) = &pending_link_pubkey
        && key.pubkey != *pubkey
    {
        return Err(CliError::Key(
            "pending email link belongs to a different local User Key; run `fsite auth link-email EMAIL` again from the machine that should own this email".to_string(),
        ));
    }
    let client = api::Client::from_env();
    let response = client.redeem_email_login(&key, &email, &token)?;
    if (pending_link_pubkey.is_some() || link_native) && !response.linked_to_native_principal {
        return Err(CliError::Api(
            "email token was verified, but the server did not link it to the native Principal. Run `fsite auth register --output json`, then `fsite auth link-email EMAIL --output json` and redeem the new token.".to_string(),
        ));
    }
    if response.linked_to_native_principal {
        keys::clear_pending_email_link(&email)?;
    }
    print_email_redeem_response(&response, output_json)
}

fn print_email_redeem_response(
    response: &EmailRedeemResponse,
    output_json: bool,
) -> Result<(), CliError> {
    if output_json {
        println!(
            "{}",
            serde_json::to_string_pretty(response).expect("response serializes")
        );
    } else {
        println!("verified {} for publishing", response.email);
        if response.linked_to_native_principal {
            println!("linked:   email now resolves to this native Principal");
        }
    }
    Ok(())
}

fn is_finite_vip_email(email: &str) -> bool {
    email.trim().to_ascii_lowercase().ends_with("@finite.vip")
}

fn parse_redeem_args(args: &[String]) -> Result<(String, String, bool, bool), CliError> {
    let mut positionals = Vec::new();
    let mut link_native = false;
    let mut output_json = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--link-native" => {
                link_native = true;
                index += 1;
            }
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag `{other}`")));
            }
            value => {
                positionals.push(value.to_string());
                index += 1;
            }
        }
    }
    let [email, token] = positionals.as_slice() else {
        return Err(CliError::Usage(auth_redeem_help().to_string()));
    };
    Ok((email.clone(), token.clone(), link_native, output_json))
}

fn view(args: &[String]) -> Result<(), CliError> {
    if help_requested(args) {
        return print_help(view_help());
    }
    let mut target: Option<String> = None;
    let mut output_json = false;
    let mut index: usize = 0;
    // Bounded by argv length.
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage("--output needs a value".to_string()))?;
                if value != "json" {
                    return Err(CliError::Usage(
                        "only --output json is supported".to_string(),
                    ));
                }
                output_json = true;
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag `{other}`")));
            }
            value => {
                if target.is_some() {
                    return Err(CliError::Usage(view_help().to_string()));
                }
                target = Some(value.to_string());
                index += 1;
            }
        }
    }
    let target = target.ok_or_else(|| CliError::Usage(view_help().to_string()))?;
    let discovered_url = if is_http_url(&target) {
        None
    } else {
        let client = api::Client::from_env();
        let paths = keys::identity_paths()?;
        match keys::load_identity(&paths)? {
            Some(identity) => {
                let key = keys::user_key_for(&identity)?;
                match client.project_status(&key, &target) {
                    Ok(project) => Some(served_output_url(&project, &target)?),
                    Err(_) if client.uses_default_production() => None,
                    Err(error) => return Err(error),
                }
            }
            None if client.uses_default_production() => None,
            None => {
                return Err(CliError::Key(
                    "a local Finite identity is required to resolve a site name against this Finite Sites server; run `fsite auth register` or pass the served URL from `fsite project status`"
                        .to_string(),
                ));
            }
        }
    };
    let url = view_target_url(&target, discovered_url.as_deref());
    let llms_url = append_url_path(&url, "llms.txt");
    if output_json {
        let value = serde_json::json!({
            "url": url,
            "llms_txt": llms_url,
            "read_only": true,
            "edit_hint": "Use fsite project status/list plus fsite auth git if you have Project Repository edit access."
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("view json serializes")
        );
    } else {
        println!("url:      {url}");
        println!("llms.txt: {llms_url}");
        println!("note:     view is read-only; edit through the Project Repository with git");
    }
    Ok(())
}

fn served_output_url(
    project: &finitesites_proto::dto::ProjectStatusResponse,
    target: &str,
) -> Result<String, CliError> {
    let output = project
        .outputs
        .iter()
        .find(|output| {
            output.output_id == target
                || output.output_name == target
                || output.site_name == target
        })
        .or_else(|| (project.outputs.len() == 1).then(|| &project.outputs[0]))
        .ok_or_else(|| {
            CliError::Api(format!(
                "Project `{}` does not have one unambiguous served output; pass the output URL from `fsite project status {}`",
                project.slug, project.slug
            ))
        })?;
    let url = display_output_url(output).trim();
    if url.is_empty() {
        return Err(CliError::Api(format!(
            "Project `{}` returned an empty served output URL",
            project.slug
        )));
    }
    Ok(url.to_string())
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn view_target_url(target: &str, discovered_url: Option<&str>) -> String {
    let value = discovered_url.unwrap_or(target);
    if is_http_url(value) {
        if value.ends_with('/') {
            return value.to_string();
        }
        return format!("{value}/");
    }
    format!("https://{target}.finite.chat/")
}

fn append_url_path(base: &str, path: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    format!("{trimmed}/{path}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn project_init_repository_failure_has_one_bounded_repair_replay() {
        let error = super::CliError::ApiStatus {
            method: "POST".to_string(),
            path: "/api/v1/projects/init".to_string(),
            status: 503,
            code: Some(finitesites_proto::dto::ERROR_GIT_REPOSITORY_SETUP_FAILED.to_string()),
            message: "Project registry state was saved".to_string(),
        };

        let repaired = super::project_init_recovery_error(
            error,
            std::path::Path::new("workspace/finite.toml"),
            None,
        );
        let super::CliError::ApiStatus { message, .. } = repaired else {
            panic!("expected API status error");
        };
        assert!(message.contains("Project may already exist"));
        assert!(message.contains("do not change its slug"));
        assert!(
            message.contains("fsite project init --config workspace/finite.toml --output json")
        );
        assert_eq!(message.matches("exactly once").count(), 1);
    }

    #[test]
    fn project_init_preflight_failure_does_not_claim_partial_state() {
        let error = super::CliError::ApiStatus {
            method: "POST".to_string(),
            path: "/api/v1/projects/init".to_string(),
            status: 503,
            code: Some(finitesites_proto::dto::ERROR_GIT_UNAVAILABLE.to_string()),
            message: "Git publishing is temporarily unavailable".to_string(),
        };

        let retry =
            super::project_init_recovery_error(error, std::path::Path::new("finite.toml"), None);
        let super::CliError::ApiStatus { message, .. } = retry else {
            panic!("expected API status error");
        };
        assert!(message.contains("No Project Init state changed"));
        assert_eq!(message.matches("retry exactly once").count(), 1);
    }

    #[test]
    fn credential_config_ops_reset_helpers_before_adding_ours() {
        let context = super::GitCredentialContext {
            protocol: "https".to_string(),
            host: "git.finite.chat".to_string(),
            path: "demo.git".to_string(),
        };
        let ops = super::git_credential_config_ops(
            &context,
            std::path::Path::new("/home/agent/.finite/git-credentials"),
        );
        // The empty helper entry MUST come first: it clears inherited
        // helpers (e.g. the macOS system osxkeychain) for this host so no
        // interactive credential UI can ever appear.
        assert_eq!(
            ops[0],
            vec![
                "--global",
                "--replace-all",
                "credential.https://git.finite.chat.helper",
                ""
            ]
        );
        assert_eq!(
            ops[1],
            vec![
                "--global",
                "--add",
                "credential.https://git.finite.chat.helper",
                "store --file /home/agent/.finite/git-credentials"
            ]
        );
        assert_eq!(
            ops[2],
            vec![
                "--global",
                "--replace-all",
                "credential.https://git.finite.chat.useHttpPath",
                "true"
            ]
        );
    }

    #[test]
    fn credential_store_line_percent_encodes_userinfo() {
        let context = super::GitCredentialContext {
            protocol: "https".to_string(),
            host: "git.finite.chat".to_string(),
            path: "demo.git".to_string(),
        };
        assert_eq!(
            super::credential_store_line(&context, "gcred_abc", "p@ss:w/rd"),
            "https://gcred_abc:p%40ss%3Aw%2Frd@git.finite.chat/demo.git"
        );
    }

    #[test]
    fn merged_credential_store_replaces_same_project_and_keeps_others() {
        let context = super::GitCredentialContext {
            protocol: "https".to_string(),
            host: "git.finite.chat".to_string(),
            path: "demo.git".to_string(),
        };
        let existing = "https://old:secret@git.finite.chat/demo.git\nhttps://keep:me@git.finite.chat/other.git\n";
        let line = super::credential_store_line(&context, "new", "cred");
        let merged = super::merged_credential_store(existing, &line, &context);
        assert!(!merged.contains("old:secret"));
        assert!(merged.contains("https://keep:me@git.finite.chat/other.git"));
        assert!(merged.ends_with("https://new:cred@git.finite.chat/demo.git\n"));
    }

    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn help_is_read_only_for_agent_probe_paths() {
        let commands = [
            &["--help"][..],
            &["--version"][..],
            &["-V"][..],
            &["version"][..],
            &["whoami", "--help"][..],
            &["describe", "--help"],
            &["project", "--help"],
            &["project", "init", "--help"],
            &["project", "grant", "--help"],
            &["project", "revoke", "--help"],
            &["project", "share", "--help"],
            &["project", "status", "--help"],
            &["project", "list", "--help"],
            &["auth", "--help"],
            &["auth", "status", "--help"],
            &["auth", "import", "--help"],
            &["auth", "register", "--help"],
            &["auth", "link-email", "--help"],
            &["auth", "login", "--help"],
            &["auth", "redeem", "--help"],
            &["auth", "git", "--help"],
            &["view", "--help"],
        ];
        // Bounded by the explicit command table above.
        for command in commands {
            run(&args(command)).unwrap();
        }
    }

    #[test]
    fn top_level_usage_is_project_first() {
        let text = usage();
        assert!(text.contains("fsite auth status [--output json]"));
        assert!(text.contains("fsite auth import [--file PATH] [--output json]"));
        assert!(!text.contains("fsite identity"));
        assert!(text.contains("shares the whole source tree through a Project Repository"));
        assert!(text.contains("cloneable by collaborators but not served as ordinary web assets"));
        assert!(text.contains("fsite describe workflow publish-static-site --output json"));
        assert!(text.contains("fsite describe workflow publish-stateful-app --output json"));
        assert!(text.contains("fsite project init --config finite.toml"));
        assert!(text.contains("fsite project grant"));
        assert!(text.contains("fsite project share"));
        assert!(text.contains("fsite project status"));
        assert!(text.contains("fsite auth register"));
        assert!(text.contains("fsite auth git"));
        assert!(text.contains("fsite view"));
        assert!(!text.contains("fsite email-login"));
        assert!(!text.contains("fsite project apply"));
        assert!(!text.contains("fsite share"));
        assert!(!text.contains("fsite claim"));
        assert!(!text.contains("fsite publish "));
        assert!(!text.contains("fsite publish-app"));
        assert!(!text.contains("fsite editors"));
        assert!(!text.contains("fsite source"));
    }

    #[test]
    fn publish_static_site_workflow_guides_agents_to_git_deploy_bytes() {
        let value = describe_workflow("publish-static-site").unwrap();
        let text = serde_json::to_string(&value).unwrap();
        assert!(text.contains("authorized collaborators clone the whole source tree"));
        assert!(text.contains("fsite project init --config finite.toml --dry-run --output json"));
        assert!(text.contains("For static sites, Finite serves only committed bytes"));
        assert!(text.contains("Source, data, docs, and build logic can live outside"));
        assert!(text.contains("Do not look for a direct publish/upload command"));
        assert!(text.contains("fsite auth git PROJECT --store --output json"));
    }

    #[test]
    fn publish_stateful_app_workflow_documents_runtime_contract() {
        let value = describe_workflow("publish-stateful-app").unwrap();
        let text = serde_json::to_string(&value).unwrap();
        assert!(text.contains("kind = \\\"app\\\""));
        assert!(text.contains("start = \\\"bun server.ts\\\""));
        assert!(text.contains("Finite Sites sets PORT and DATA_DIR"));
        assert!(text.contains("0.0.0.0:$PORT"));
        assert!(text.contains("Do not rely on Finite Sites to run npm install"));
        assert!(text.contains("fsite auth git PROJECT --store --output json"));
    }

    #[test]
    fn project_config_workflow_documents_app_schema() {
        let value = describe_workflow("project-config").unwrap();
        let text = serde_json::to_string(&value).unwrap();
        assert!(text.contains("site, document, or app"));
        assert!(text.contains("outputs.<id>.start"));
        assert!(text.contains("kind = \\\"app\\\""));
        assert!(text.contains("start = \\\"bun server.ts\\\""));
    }

    #[test]
    fn publish_document_workflow_documents_markdown_outputs() {
        let value = describe_workflow("publish-document").unwrap();
        let text = serde_json::to_string(&value).unwrap();
        assert!(text.contains("kind = \\\"document\\\""));
        assert!(text.contains("document_name = \\\"my-docs\\\""));
        assert!(text.contains("Raw Markdown companion URLs append .md"));
        assert!(text.contains("Do not commit generated HTML"));
    }

    #[test]
    fn register_and_publish_workflow_documents_source_only_projects() {
        let value = describe_workflow("register-and-publish").unwrap();
        let text = serde_json::to_string(&value).unwrap();
        assert!(text.contains("fsite auth register --output json"));
        assert!(text.contains("A source-only Project Repository may contain only [project]"));
        assert!(text.contains("served website needs an [outputs.<id>] entry"));
        assert!(text.contains("event.source.user_id"));
        assert!(text.contains("--owner-viewer-npub AUTHENTICATED_SENDER_ID"));
    }

    #[test]
    fn project_share_parser_requires_explicit_public_confirmation() {
        assert!(matches!(
            parse_project_share_args(&args(&["demo", "site", "--public"])),
            Err(CliError::Usage(message)) if message.contains("--yes-public")
        ));
        let parsed = parse_project_share_args(&args(&[
            "demo",
            "site",
            "--public",
            "--yes-public",
            "--output",
            "json",
        ]))
        .unwrap();
        assert_eq!(parsed.project, "demo");
        assert_eq!(parsed.output_id, "site");
        assert_eq!(parsed.visibility.as_deref(), Some("public"));
        assert!(parsed.confirm_public);
        assert!(parsed.output_json);
    }

    #[test]
    fn project_share_parser_validates_invites_and_visibility() {
        assert!(matches!(
            parse_project_share_args(&args(&[
                "demo",
                "site",
                "--private",
                "--send-invite",
                "--add-email",
                "viewer@example.com",
            ])),
            Err(CliError::Usage(message)) if message.contains("--shared")
        ));
        assert!(matches!(
            parse_project_share_args(&args(&["demo", "site", "--shared", "--send-invite"])),
            Err(CliError::Usage(message)) if message.contains("--add-email")
        ));
        assert!(matches!(
            parse_project_share_args(&args(&["demo", "site", "--shared", "--private"])),
            Err(CliError::Usage(message)) if message.contains("only one visibility")
        ));
        let parsed = parse_project_share_args(&args(&[
            "demo",
            "site",
            "--shared",
            "--add-email",
            "viewer@example.com",
            "--send-invite",
        ]))
        .unwrap();
        assert_eq!(parsed.visibility.as_deref(), Some("shared"));
        assert_eq!(parsed.add_emails, vec!["viewer@example.com"]);
        assert!(parsed.send_invite);

        let native = parse_project_share_args(&args(&[
            "demo",
            "site",
            "--add-npub",
            "npub1viewer",
            "--remove-npub",
            "npub1former",
        ]))
        .unwrap();
        assert_eq!(native.add_npubs, vec!["npub1viewer"]);
        assert_eq!(native.remove_npubs, vec!["npub1former"]);
    }

    #[test]
    fn old_site_first_commands_point_to_project_repository_workflow() {
        assert!(matches!(
            run(&args(&["publish"])),
            Err(CliError::Usage(message))
                if message.contains("not part of the current Project Repository model")
                    && message.contains("fsite project init --config finite.toml")
                    && message.contains("Deploy Branch")
        ));
        assert!(matches!(
            run(&args(&["publish-app"])),
            Err(CliError::Usage(message))
                if message.contains("not part of the current Project Repository model")
                    && message.contains("fsite auth git PROJECT --store --output json")
        ));
        assert!(matches!(
            run(&args(&["share"])),
            Err(CliError::Usage(message))
                if message.contains("Project Output sharing")
                    && message.contains("fsite project share PROJECT OUTPUT")
                    && !message.contains("fsite share SITE_NAME")
        ));
        assert!(matches!(
            run(&args(&["status"])),
            Err(CliError::Usage(message))
                if message.contains("Use Project Status")
                    && message.contains("fsite project status PROJECT --output json")
        ));
        assert!(matches!(
            run(&args(&["list"])),
            Err(CliError::Usage(message))
                if message.contains("Use Project List")
                    && message.contains("fsite project list --output json")
        ));
    }

    #[test]
    fn no_arg_commands_reject_extra_non_help_arguments() {
        assert!(matches!(
            run(&args(&["whoami", "extra"])),
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            run(&args(&["project", "list", "extra"])),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn auth_git_parses_native_and_external_store_modes() {
        let native =
            parse_auth_git_args(&args(&["finite-curriculum", "--store", "--output", "json"]))
                .unwrap();
        assert_eq!(
            native,
            AuthGitOptions {
                project: "finite-curriculum".to_string(),
                email: None,
                output_json: true,
                store: true,
            }
        );

        let external =
            parse_auth_git_args(&args(&["finite-curriculum", "--email", "paul@finite.vip"]))
                .unwrap();
        assert_eq!(external.email.as_deref(), Some("paul@finite.vip"));
        assert!(!external.store);
    }

    #[test]
    fn auth_redeem_parser_accepts_json_output_and_rejects_unknown_flags() {
        let parsed = parse_redeem_args(&args(&[
            "skyler@example.com",
            "token123",
            "--output",
            "json",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            (
                "skyler@example.com".to_string(),
                "token123".to_string(),
                false,
                true
            )
        );
        let native_link = parse_redeem_args(&args(&[
            "skyler@example.com",
            "token123",
            "--link-native",
            "--output",
            "json",
        ]))
        .unwrap();
        assert_eq!(
            native_link,
            (
                "skyler@example.com".to_string(),
                "token123".to_string(),
                true,
                true
            )
        );
        assert!(matches!(
            parse_redeem_args(&args(&["skyler@example.com", "token123", "--store"])),
            Err(CliError::Usage(message)) if message.contains("unknown flag")
        ));
    }

    #[test]
    fn git_credential_context_is_path_aware() {
        let context =
            git_credential_context("https://git.finite.chat/finite-curriculum.git").unwrap();
        assert_eq!(context.protocol, "https");
        assert_eq!(context.host, "git.finite.chat");
        assert_eq!(context.path, "finite-curriculum.git");
        let line = credential_store_line(&context, "user", "secret");
        assert_eq!(
            line,
            "https://user:secret@git.finite.chat/finite-curriculum.git"
        );
    }

    #[test]
    fn git_config_command_uses_config_subcommand() {
        assert_eq!(
            git_config_command_args(&[
                "--global",
                "credential.https://git.finite.chat.useHttpPath",
                "true"
            ]),
            vec![
                "config".to_string(),
                "--global".to_string(),
                "credential.https://git.finite.chat.useHttpPath".to_string(),
                "true".to_string(),
            ]
        );
    }

    #[test]
    fn removed_commands_point_to_current_primitives() {
        assert!(matches!(
            run(&args(&["share", "demo", "--send-invite"])),
            Err(CliError::Usage(message)) if message.contains("not part of the current Project Repository model")
        ));
        assert!(matches!(
            run(&args(&["project", "apply", "--help"])),
            Err(CliError::Usage(message)) if message.contains("fsite project init --config finite.toml")
        ));
    }

    #[test]
    fn project_example_fixture_matches_committed_config() {
        let examples = [
            (
                "finitechat-native",
                include_str!("../../../examples/finitechat-native-mockup/finite.toml"),
            ),
            (
                "finite-hello-site",
                include_str!("../../../examples/hello-site/finite.toml"),
            ),
            (
                "finite-spa-pushstate",
                include_str!("../../../examples/spa-pushstate/finite.toml"),
            ),
            (
                "finite-react-bun-spa",
                include_str!("../../../examples/react-bun-spa/finite.toml"),
            ),
            (
                "finite-nextjs-demo",
                include_str!("../../../examples/nextjs-demo/finite.toml"),
            ),
            (
                "finite-fasthtml-demo",
                include_str!("../../../examples/fasthtml-demo/finite.toml"),
            ),
            (
                "finite-docs-demo",
                include_str!("../../../examples/docs-demo/finite.toml"),
            ),
        ];
        for (slug, raw) in examples {
            let config = parse_project_config_toml(raw).unwrap();
            assert_eq!(config.project.slug, slug);
            assert_eq!(config.outputs.len(), 1);
        }
    }

    #[test]
    fn view_target_url_supports_url_or_name() {
        assert_eq!(
            view_target_url("finitechat-native-mockup", None),
            "https://finitechat-native-mockup.finite.chat/"
        );
        assert_eq!(
            view_target_url(
                "finitechat-native-mockup",
                Some("http://finitechat-native-mockup.sites.localhost:8787/")
            ),
            "http://finitechat-native-mockup.sites.localhost:8787/"
        );
        assert_eq!(
            append_url_path("https://finitechat-native-mockup.finite.chat/", "llms.txt"),
            "https://finitechat-native-mockup.finite.chat/llms.txt"
        );
    }
}
