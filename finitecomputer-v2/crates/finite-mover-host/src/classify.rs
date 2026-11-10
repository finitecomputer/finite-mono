//! The one classifier. Every path under `kata/<durable_state_id>/` gets
//! exactly one [`Class`], and `sizes`, `stream` and the mover's uploader
//! all ask this module, so the roster's numbers are what gets staged and
//! the uploader's second check is the same rule.
//!
//! Paths are relative to the durable tree, which the Runtime mounts at
//! `/data` (`runtime.Dockerfile`: `FINITE_HOME=/data/agent`,
//! `HERMES_HOME=/data/agent/hermes-home`, `FINITECHAT_WORKSPACE=/data/workspace`).
//!
//! Rules apply in this order; the first that matches decides:
//!
//! 1. `secret`: never staged. Each rule names the code that writes the file.
//! 2. `cache`: anything under a dependency or cache directory, or under a
//!    build folder beside the project file that builds it.
//! 3. A brain working tree the mover did not name as owned: `runtime`.
//! 4. Links, sockets and devices, and names a zip can't carry: `runtime`.
//! 5. `history`: the allow-listed parts of the Hermes home.
//! 6. `work`: the workspace, but its hidden folders and earlier exports;
//!    and `sites/<slug>/`, the agent's own sites, cloned beside the tree.
//! 7. Everything else: `runtime`.
//!
//! `config` is never a source path: it is the allow-listed copy that
//! [`crate::config_copy`] makes of `config.yaml`, which itself is secret.
//! Nor is `simplex`: it is the SimpleX conversation [`crate::simplex`]
//! carries out of `agent/simplex/`, which itself stays secret.
//! A text file classified `history` or `work` becomes `secret` when its
//! contents hold a credential ([`crate::shapes`]); [`crate::walk`] applies
//! that, since it needs the file.

use serde::Serialize;

/// What a path is, for the move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Secret,
    Config,
    Cache,
    Runtime,
    History,
    Work,
    Simplex,
}

impl Class {
    pub const ALL: [Class; 7] = [
        Class::Secret,
        Class::Config,
        Class::Cache,
        Class::Runtime,
        Class::History,
        Class::Work,
        Class::Simplex,
    ];

    /// Whether this class goes into the stage.
    pub fn staged(self) -> bool {
        matches!(
            self,
            Class::Config | Class::History | Class::Work | Class::Simplex
        )
    }
}

/// Why a path has its class. Printed in `left` so the person and the
/// importer can say what stayed behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Why {
    /// `agent/identity/`: `identity.json` holds `secret_hex`, the agent's
    /// Nostr key (finite-identity/src/paths.rs:47, src/identity.rs:24-35).
    NostrIdentity,
    /// `agent/hermes-home/.env`: `OPENROUTER_API_KEY`
    /// (finite-agentd/src/connections.rs:165-181, 707-709).
    HermesEnv,
    /// `agent/hermes-home/config.yaml`, and its temporary and backup
    /// copies: `gateway.platforms.telegram.token`
    /// (finite-agentd/src/connections.rs:399-402) and `model.api_key`
    /// (connections.rs:320-330). Its allow-listed copy is `config`.
    HermesConfig,
    /// `agent/agentd/`: `agentd.sqlite3`'s `config_history.before_bytes`
    /// keeps every replaced config value, old tokens included
    /// (finite-agentd/src/ledger.rs:59-67, src/daemon.rs:166-168,205), and
    /// `google-workspace.json` the connected account
    /// (finite-agentd/src/connections.rs:703-705).
    Agentd,
    /// `agent/client.sqlite3*`: the Finite Chat client store, device state
    /// and its writer lease (finitechat-client/src/lib.rs:3787-3790,
    /// 8040-8060).
    ChatClientStore,
    /// `google_token.json`, `google_client_secret.json`,
    /// `google_oauth_pending.json` (finite-agentd/src/connections.rs:513-575).
    GoogleCredentials,
    /// `agent/hermes-home/gws/`: the Google Workspace CLI's configuration
    /// and credentials (runtime.Dockerfile:162-163).
    GoogleWorkspaceCli,
    /// `agent/simplex/`: the SimpleX identity databases
    /// (finite-agentd/simplex_runtime.py:82-91). Its conversation with the
    /// owner is carried for the bound agent only when that is safe
    /// ([`crate::simplex`]); the folder itself never is.
    Simplex,
    /// `agent/hermes-home/pairing/` and `.../platforms/pairing/`: approved
    /// and pending contacts (finite-agentd/src/connections.rs:650-657,
    /// simplex_runtime.py:405-407); and `pairing/*-approved.json` anywhere.
    Pairing,
    /// A name that holds, or may hold, a credential: `SECRET_NAMES`,
    /// `SECRET_SUFFIXES` and `SECRET_NAME_SHAPES` of the export
    /// (finite-agentd/integrations/hermes/finite-export/dashboard/export.py:197-223),
    /// plus ssh keys and the `.ssh`, `.gnupg` and `.aws` directories.
    CredentialName,
    /// A text file holding a credential-shaped string (export.py:225-243,
    /// 386-403).
    TokenInText,
    /// A text file too large to hold while it is read for credentials
    /// (over `shapes::TEXT_SNAPSHOT_MAX`), or with a run too long to check
    /// (export.py:497-501).
    UncheckableText,
    /// The allow-listed copy of `config.yaml`.
    ConfigCopy,
    /// A source file at the path the allow-listed copy takes.
    ReplacedByCopy,
    /// Dependencies, caches and version control (`SKIP_DIRS`,
    /// export.py:66-84). They stay in the Borg backup.
    CacheDir,
    /// A build folder beside the project file that builds it, in the
    /// workspace (`BUILD_DIRS`, `PROJECT_FILES`, export.py:85-115, 2003-2020).
    BuildOutput,
    /// A brain working tree the mover did not name as owned by this
    /// person, or one whose `.finitebrain/brain-directory.json` can't be
    /// read.
    NotOwnedBrain,
    /// A link (symbolic, or a file with more than one name), socket, pipe
    /// or device: never followed, never read, never staged.
    NotAFile,
    /// A name that is not UTF-8, or holds a control character or a
    /// backslash, which a zip can't carry faithfully.
    UncarriedName,
    /// The allow-listed parts of the Hermes home and the managed skills'
    /// diff base.
    History,
    /// The workspace.
    Workspace,
    /// A hidden folder in the workspace: tool state, often credentials the
    /// text check can't see. The export and the importer never bring one
    /// (export.py:2047, `workspace_skipper`).
    HiddenFolder,
    /// `workspace/finite-export/`: earlier exports (export.py:2247).
    EarlierExport,
    /// `sites/<slug>/`: a site the agent owns, cloned from Finite Sites
    /// beside the tree (the mover, and export.py's `clone_site`).
    Site,
    /// The runtime: everything that is neither history, work nor cache.
    Runtime,
}

/// The kind of directory entry being classified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    /// A link, socket, pipe, device, or a directory on another filesystem.
    Other,
}

/// Whether the entry sits inside a brain working tree, and whether the
/// mover named that brain as owned. Only the walk can know this (it reads
/// `.finitebrain/brain-directory.json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrainScope {
    None,
    /// `depth` is the number of components of the working tree's root.
    Brain {
        depth: usize,
        owned: bool,
    },
}

/// Where an entry sits, as only the walk can know: in which brain working
/// tree, and under which build folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub brain: BrainScope,
    /// The number of components of the build folder the entry is under.
    pub build_output: Option<usize>,
}

impl From<BrainScope> for Scope {
    fn from(brain: BrainScope) -> Self {
        Scope {
            brain,
            build_output: None,
        }
    }
}

/// A path's class, why, and how many leading components decided it. The
/// summary's `left` groups files by that prefix, so a whole cache
/// directory is one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub class: Class,
    pub why: Why,
    pub depth: usize,
}

const fn verdict(class: Class, why: Why, depth: usize) -> Verdict {
    Verdict { class, why, depth }
}

/// The directory the allow-listed copy of `config.yaml` is written in, and
/// its name.
pub const CONFIG_COPY_PATH: [&str; 3] = ["agent", "hermes-home", "config.public.yaml"];

/// Folders a tool builds beside a project file, and the project files
/// (`BUILD_DIRS`, `PROJECT_FILES`, `PROJECT_SUFFIXES`, export.py:85-115).
pub const BUILD_DIRS: [&str; 9] = [
    "dist", "build", "out", "target", "_build", "obj", "deps", "vendor", "coverage",
];
pub const PROJECT_FILES: [&str; 18] = [
    "package.json",
    "deno.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "setup.py",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "Makefile",
    "CMakeLists.txt",
    "mix.exs",
    "Gemfile",
    "composer.json",
    "pubspec.yaml",
    "Package.swift",
    "build.zig",
    "stack.yaml",
];
pub const PROJECT_SUFFIXES: [&str; 4] = [".csproj", ".fsproj", ".sln", ".cabal"];

/// Whether `name` is a project file a build folder beside it comes from.
pub fn project_file(name: &[u8]) -> bool {
    PROJECT_FILES.iter().any(|file| name == file.as_bytes())
        || PROJECT_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix.as_bytes()))
}

/// Directories no stage wants, by name, wherever they are (`SKIP_DIRS`,
/// export.py:66-84).
const CACHE_DIRS: [&str; 17] = [
    ".git",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    ".next",
    ".finitebrain",
    ".mypy_cache",
    ".pytest_cache",
    ".tox",
    ".gradle",
    "bower_components",
    "site-packages",
    "Pods",
    "DerivedData",
    "elm-stuff",
];

/// Files that hold, or may hold, a credential, by name, anywhere
/// (`SECRET_NAMES`, export.py:197-221, and ssh keys).
const SECRET_NAMES: [&str; 28] = [
    ".env",
    ".envrc",
    "auth.json",
    "google_token.json",
    "google_client_secret.json",
    "google_oauth_pending.json",
    "identity.json",
    "git-credentials",
    ".git-credentials",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".pgpass",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "id_ecdsa_sk",
    "id_ed25519_sk",
    "credentials",
    "credentials.json",
    "client_secret.json",
    "token.json",
    "secrets.yaml",
    "secrets.yml",
    "secrets.json",
    ".htpasswd",
    "_netrc",
];

/// `SECRET_SUFFIXES` (export.py:222).
const SECRET_SUFFIXES: [&str; 6] = [".pem", ".key", ".p12", ".pfx", ".keystore", ".env"];

/// Directories that hold credentials, by name, anywhere.
const SECRET_DIRS: [&str; 3] = [".ssh", ".gnupg", ".aws"];

/// The Hermes home's history, by top-level name: files, databases (each
/// with its SQLite sidecars), then directories.
const HISTORY_FILES: [&str; 1] = ["SOUL.md"];
const HISTORY_DATABASES: [&str; 2] = ["state.db", "memory_store.db"];
const HISTORY_DIRS: [&str; 4] = ["memories", "skills", "cron", "scripts"];
const SQLITE_SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];

/// Classify one path, relative to the durable tree, by its components.
///
/// This is the rule the mover's uploader repeats on every name it
/// receives: a `secret` class there refuses the whole stage.
pub fn classify(parts: &[&[u8]], kind: EntryKind, scope: impl Into<Scope>) -> Verdict {
    let scope = scope.into();
    let len = parts.len();
    if let Some(secret) = secret_rule(parts) {
        return secret;
    }
    if let Some(index) = parts[..len.saturating_sub(1)]
        .iter()
        .position(|part| CACHE_DIRS.iter().any(|name| part == &name.as_bytes()))
    {
        return verdict(Class::Cache, Why::CacheDir, index + 1);
    }
    if let BrainScope::Brain {
        depth,
        owned: false,
    } = scope.brain
    {
        return verdict(Class::Runtime, Why::NotOwnedBrain, depth);
    }
    if let Some(depth) = scope.build_output {
        return verdict(Class::Cache, Why::BuildOutput, depth);
    }
    if kind == EntryKind::Other {
        return verdict(Class::Runtime, Why::NotAFile, len);
    }
    if let Some(index) = parts.iter().position(|part| !carried_name(part)) {
        return verdict(Class::Runtime, Why::UncarriedName, index + 1);
    }
    allow_list(parts)
}

/// The secret rules, in the order of the design's list. Directory rules
/// return the directory's depth; name rules the whole path.
fn secret_rule(parts: &[&[u8]]) -> Option<Verdict> {
    let len = parts.len();
    let at = |index: usize, name: &str| {
        parts
            .get(index)
            .is_some_and(|part| *part == name.as_bytes())
    };
    let secret = |why, depth| Some(verdict(Class::Secret, why, depth));
    if at(0, "agent") {
        if at(1, "identity") && len > 2 {
            return secret(Why::NostrIdentity, 2);
        }
        if at(1, "simplex") && len > 2 {
            return secret(Why::Simplex, 2);
        }
        if len == 2 && parts[1].starts_with(b"client.sqlite3") {
            return secret(Why::ChatClientStore, 2);
        }
        if at(1, "agentd") && len > 2 {
            return secret(Why::Agentd, 2);
        }
        if at(1, "hermes-home") && len > 2 {
            let name = parts[2];
            if len == 3 && name == b".env" {
                return secret(Why::HermesEnv, 3);
            }
            if len == 3 && (name.starts_with(b"config.yaml") || name.starts_with(b".config.yaml")) {
                return secret(Why::HermesConfig, 3);
            }
            if at(2, "gws") && len > 3 {
                return secret(Why::GoogleWorkspaceCli, 3);
            }
            if at(2, "pairing") && len > 3 {
                return secret(Why::Pairing, 3);
            }
            if at(2, "platforms") && at(3, "pairing") && len > 4 {
                return secret(Why::Pairing, 4);
            }
        }
    }
    if let Some(index) = parts[..len.saturating_sub(1)].iter().position(|part| {
        SECRET_DIRS
            .iter()
            .any(|dir| part.eq_ignore_ascii_case(dir.as_bytes()))
    }) {
        return secret(Why::CredentialName, index + 1);
    }
    let name = parts.last()?;
    if len >= 2 && parts[len - 2] == b"pairing" && name.ends_with(b"-approved.json") {
        return secret(Why::Pairing, len);
    }
    if [
        "google_token.json",
        "google_client_secret.json",
        "google_oauth_pending.json",
    ]
    .iter()
    .any(|google| name.eq_ignore_ascii_case(google.as_bytes()))
    {
        return secret(Why::GoogleCredentials, len);
    }
    if secret_name(name) {
        return secret(Why::CredentialName, len);
    }
    None
}

/// `secret_reason` of the export (export.py:375-383), ignoring case.
pub fn secret_name(name: &[u8]) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_NAMES.iter().any(|secret| lower == secret.as_bytes())
        || lower.starts_with(b".env.")
        || SECRET_SUFFIXES
            .iter()
            .any(|suffix| lower.ends_with(suffix.as_bytes()))
        || (lower.ends_with(b".json")
            && (lower.starts_with(b"client_secret") || service_account(&lower)))
}

/// `^service[-_]?account.*\.json$` (export.py:223), on a lowercased name.
fn service_account(lower: &[u8]) -> bool {
    let Some(rest) = lower.strip_prefix(b"service") else {
        return false;
    };
    let rest = rest
        .strip_prefix(b"-")
        .or_else(|| rest.strip_prefix(b"_"))
        .unwrap_or(rest);
    rest.starts_with(b"account")
}

/// A name a zip entry carries faithfully: UTF-8, no control character,
/// no backslash (which some readers take for a separator).
fn carried_name(part: &[u8]) -> bool {
    std::str::from_utf8(part)
        .is_ok_and(|name| !name.is_empty() && !name.chars().any(|c| c.is_control() || c == '\\'))
}

/// Rules 5-7: history, work, and the runtime by default.
fn allow_list(parts: &[&[u8]]) -> Verdict {
    let len = parts.len();
    let at = |index: usize, name: &str| {
        parts
            .get(index)
            .is_some_and(|part| *part == name.as_bytes())
    };
    let runtime = |depth: usize| verdict(Class::Runtime, Why::Runtime, depth.min(len));
    if at(0, "workspace") && len > 1 {
        if at(1, "finite-export") && len > 2 {
            return verdict(Class::Runtime, Why::EarlierExport, 2);
        }
        if let Some(index) = parts[1..len - 1]
            .iter()
            .position(|part| part.starts_with(b"."))
        {
            return verdict(Class::Runtime, Why::HiddenFolder, index + 2);
        }
        return verdict(Class::Work, Why::Workspace, 1);
    }
    if at(0, "sites") && len > 2 {
        return verdict(Class::Work, Why::Site, 1);
    }
    if !at(0, "agent") {
        return runtime(1);
    }
    if at(1, "hermes-home") && len > 2 {
        let name = parts[2];
        if len == 3 {
            if parts[..] == CONFIG_COPY_PATH.map(str::as_bytes)[..] {
                return verdict(Class::Runtime, Why::ReplacedByCopy, 3);
            }
            if HISTORY_FILES.iter().any(|file| name == file.as_bytes()) || history_database(name) {
                return verdict(Class::History, Why::History, 3);
            }
        }
        if len > 3 && HISTORY_DIRS.iter().any(|dir| name == dir.as_bytes()) {
            return verdict(Class::History, Why::History, 3);
        }
        return runtime(3);
    }
    if at(1, "managed-skills") {
        if !at(2, "finite") {
            return runtime(3);
        }
        if !at(3, "current") {
            return runtime(4);
        }
        if len > 4 {
            return verdict(Class::History, Why::History, 4);
        }
        return runtime(4);
    }
    runtime(2)
}

/// A history database or one of its SQLite sidecars: a database copied
/// from a stopped agent is only whole with its write-ahead log.
fn history_database(name: &[u8]) -> bool {
    HISTORY_DATABASES.iter().any(|database| {
        name.strip_prefix(database.as_bytes()).is_some_and(|rest| {
            rest.is_empty()
                || SQLITE_SIDECARS
                    .iter()
                    .any(|sidecar| rest == sidecar.as_bytes())
        })
    })
}

#[cfg(test)]
mod tests;
