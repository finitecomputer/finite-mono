//! FiniteBrain Portable v1 core domain and validation logic.

pub mod portability;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, OsRng, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use finite_nostr::{NostrPublicKey, verify_event_integrity};
use nostr::{Event, Kind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

const RESERVED_TOP_LEVEL_NAMES: [&str; 3] = [".finitebrain", "_admin", ".git"];
const FOLDER_OBJECT_VERSION: &str = "finite-folder-object-v1";
const CIPHER_AES_256_GCM: &str = "AES-256-GCM";
const APP_SPECIFIC_KIND: u16 = 30_078;
const MAX_USER_ID_LEN: usize = 128;
const MAX_DISPLAY_NAME_LEN: usize = 128;
const MAX_SAFE_RELATIVE_PATH_LEN: usize = 1024;

/// Default markdown Page materialized in a starter Vault Folder.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DefaultVaultPage {
    /// Starter Folder receiving the default Page.
    pub folder_id: &'static str,
    /// Stable default object id.
    pub object_id: &'static str,
    /// Decrypted Page path.
    pub path: &'static str,
    /// Decrypted Page markdown.
    pub markdown: &'static str,
}

const DEFAULT_AGENTS_MARKDOWN: &str = r#"# AGENTS.md

This is a FiniteBrain vault. Start with [[Getting Started]], then use
[[How FiniteBrain Works]] and [[Access And Folders]] to understand the
product model. Treat every readable Folder as its own encrypted, syncable
LLM wiki scope.

## Operating Model

FiniteBrain stores encrypted Vault state on the server. Trusted clients and
agent runtimes open Folder Key Grants locally, decrypt accessible Pages and
Assets, edit ordinary files, then sync encrypted changes back. See
[[How FiniteBrain Works]] for the technical spine and [[Access And Folders]] for
the privacy boundary.

Agents act as the user. They do not have independent Vault membership, Folder access, or attribution unless explicitly modeled as a separate user.

A Vault is not one giant wiki with folders. It is a namespace of many
Folder-scoped LLM wikis. Folder access determines which wiki scopes can be read
or written. The local scope contract lives in [[Getting Started Config]],
with navigation in [[Getting Started Index]] and maintenance history in
[[Getting Started Log]].

## Use `fbrain`

Use `fbrain` for identity, sync, access, and daemon state.

Start here:

```sh
fbrain doctor --server "$SERVER"
fbrain auth status --json
fbrain open "$VAULT" "$TREE" --server "$SERVER"
cd "$TREE"
fbrain sync now --summary
fbrain unlock --all
fbrain sync now --summary
fbrain conflicts --json
```

Use an explicit config dir in agent runtimes:

```sh
fbrain --config-dir "$HOME/.config/finitebrain" auth status --json
```

Never print or expose Nostr secrets, Folder Keys, grant plaintext, auth files, decrypted sync internals, or rotation bodies.

## Editing Rules

Before editing:

1. Sync.
2. Unlock readable folders.
3. Read this file.
4. Read [[HUMANS.md]].
5. Read [[Getting Started Index]], [[Getting Started Config]],
   [[Getting Started Log]], `index.md`, or `SCHEMA.md` when present.
6. Search before creating new pages.

Only edit readable content. Do not edit `.finitebrain/`, encrypted sync evidence, locked metadata-only folders, generated state files, auth files, or key material.

After editing:

```sh
fbrain sync now --summary
fbrain conflicts --json
```

Resolve conflicts before reporting done.

## LLM Wiki Rules

Use each readable Folder as a durable LLM wiki scope.

- The default `getting-started` Folder is the shared orientation scope for users and agents. Its starter map is [[Getting Started]].
- The default `restricted` Folder is the starter tighter-boundary scope for sensitive work. If readable, its starter note is [[Restricted Folder Example]].
- Keep raw sources immutable under that Folder's `raw/`.
- Store non-Markdown source files under that Folder's `raw/assets/`.
- Pair every Asset with a Markdown Source Note that records provenance, content type, hash or extraction status when known.
- Cite Source Notes from synthesized wiki pages; do not make the blob itself the knowledge surface.
- Put synthesized durable knowledge in that Folder's `wiki/`.
- Prefer updating existing pages over creating duplicates.
- Use wikilinks for internal relationships.
- Keep the Folder-local `_index.md` current; this starter scope uses [[Getting Started Index]].
- Append only to the Folder-local `log.md` after meaningful writes in that Folder; this starter scope uses [[Getting Started Log]].
- Use `inventory/` for source candidates, open questions, watch items, and next actions.
- Use `datasets/` for manifests, schemas, samples, and query recipes.
- Use `output/` for reports, plans, summaries, and deliverables.
- Archive superseded material instead of deleting it.
- Answer from curated wiki pages first; say what is missing when evidence is thin.
- Never summarize restricted Folder contents into a less-restricted Folder,
  index, log, or output.

## Suggested Layout

```text
config.md
_index.md
log.md
inbox/
raw/
  assets/
wiki/
inventory/
datasets/
output/
archive/
```

Local folder instructions may override this layout. Human-facing context is in
[[HUMANS.md]], and the seeded graph hub is [[Getting Started]].

## Final Report

When finished, report:

- working tree path
- acting npub, if relevant
- folders readable or locked
- pages or sources created/updated/moved/deleted
- index/log updates
- sync summary
- latest sequence, if available
- whether conflicts are empty
"#;

const DEFAULT_HUMANS_MARKDOWN: &str = r#"# HUMANS.md

This vault is your private, encrypted knowledge workspace.

FiniteBrain keeps the server blind to page and asset contents. Your client or agent opens the vault locally, decrypts what you can access, edits ordinary files, then syncs encrypted changes back. [[How FiniteBrain Works]] explains that flow.

A FiniteBrain vault is a namespace of wiki scopes. Each top-level Folder is its
own LLM wiki with its own `_index.md`, `config.md`, and `log.md`. The
starter orientation scope is mapped in [[Getting Started Index]], configured
by [[Getting Started Config]], and recorded in [[Getting Started Log]].

Inside a Folder:

- `raw/` is source material.
- `raw/assets/` is non-Markdown source files such as PDFs, images, audio, video, and datasets.
- Source Notes are Markdown pages that explain those files and make them usable by agents.
- `wiki/` is durable notes and synthesized understanding.
- `inventory/` tracks things to revisit.
- `datasets/` indexes structured references.
- `output/` holds reports, plans, and finished work.
- `log.md` records meaningful changes for that Folder only.

The default `getting-started` Folder is for orientation and shared operating
rules. The default `restricted` Folder demonstrates a tighter access boundary
for private work.

Read [[Getting Started]] for the first-page map, [[Access And Folders]] for
sharing rules, and [[AGENTS.md]] for agent operating instructions. Agents
should sync before editing, avoid duplicates, preserve sources, create Source
Notes for assets, and keep the wiki useful for future work.
"#;

const DEFAULT_GETTING_STARTED_SCOPE_CONFIG_MARKDOWN: &str = r#"# Getting Started Config

This Folder is an independent FiniteBrain LLM wiki scope.

Use [[Getting Started Index]] as the local navigation hub and append meaningful
maintenance to [[Getting Started Log]]. Shared product orientation starts at
[[Getting Started]], with related model notes in [[How FiniteBrain Works]]
and [[Access And Folders]].

Use this Folder's `raw/`, `raw/assets/`, `wiki/`, `inventory/`, `datasets/`, and `output/`
directories for knowledge that belongs inside this access boundary. Keep this
Folder's `_index.md` and `log.md` scoped only to pages in this Folder.

Store non-Markdown source files in `raw/assets/` and pair each one with a
Markdown Source Note in this Folder.

Do not summarize restricted sibling Folder contents here unless the user
explicitly chooses this Folder as an equal-or-more-restricted destination.

Related default scope: `restricted`. Keep cross-Folder synthesis access-safe.
"#;

const DEFAULT_GETTING_STARTED_SCOPE_INDEX_MARKDOWN: &str = r#"# Getting Started Index

This index maps the shared orientation wiki scope.

## Local Pages

- [[Getting Started]] is the first-page map for a new Vault.
- [[How FiniteBrain Works]] explains the trusted-client and encrypted-server model.
- [[Access And Folders]] explains Folder-scoped access boundaries.
- [[AGENTS.md]] gives agent operating rules.
- [[HUMANS.md]] gives human-facing orientation.
- [[Getting Started Config]] defines this scope's wiki conventions.
- [[Getting Started Log]] records meaningful writes in this Folder only.

## Boundaries

Do not list private titles, summaries, source hints, assets, or activity from
sibling Folders here. Link out only to product-safe default orientation.
"#;

const DEFAULT_GETTING_STARTED_SCOPE_LOG_MARKDOWN: &str = r#"# Getting Started Log

Append meaningful changes in this Folder only. Keep [[Getting Started Index]] in
sync with durable pages and follow [[Getting Started Config]] for scope rules.

Do not record activity from sibling Folders here.
"#;

const DEFAULT_RESTRICTED_SCOPE_CONFIG_MARKDOWN: &str = r#"# Restricted Config

This Folder is an independent FiniteBrain LLM wiki scope.

Use [[Restricted Index]] as the local navigation hub and append meaningful
maintenance to [[Restricted Log]]. Shared product orientation starts at
[[Getting Started]], with related model notes in [[How FiniteBrain Works]]
and [[Access And Folders]].

Use this Folder's `raw/`, `raw/assets/`, `wiki/`, `inventory/`, `datasets/`, and `output/`
directories for knowledge that belongs inside this access boundary. Keep this
Folder's `_index.md` and `log.md` scoped only to pages in this Folder.

Store non-Markdown source files in `raw/assets/` and pair each one with a
Markdown Source Note in this Folder.

Do not summarize restricted sibling Folder contents here unless the user
explicitly chooses this Folder as an equal-or-more-restricted destination.

Related default scope: `getting-started`. Keep cross-Folder synthesis access-safe.
"#;

const DEFAULT_RESTRICTED_SCOPE_INDEX_MARKDOWN: &str = r#"# Restricted Index

This index maps the restricted starter wiki scope. It should describe only
content that belongs inside this Folder's access boundary.

## Local Pages

- [[Restricted Folder Example]] explains this default tighter-boundary Folder.
- [[Restricted Config]] defines the local wiki conventions.
- [[Restricted Log]] records meaningful writes in this Folder only.

## Related Orientation

- [[Getting Started]] is the shared starter map.
- [[How FiniteBrain Works]] explains trusted-client encryption and sync.
- [[Access And Folders]] explains why restricted content must stay inside an equal-or-more-restricted destination.
- [[AGENTS.md]] gives agent operating rules.
"#;

const DEFAULT_RESTRICTED_SCOPE_LOG_MARKDOWN: &str = r#"# Restricted Log

Append meaningful changes in this Folder only. Keep [[Restricted Index]] in
sync with durable pages and follow [[Restricted Config]] for scope rules.

Do not record activity from sibling Folders here.
"#;

const DEFAULT_GETTING_STARTED_README_MARKDOWN: &str = r#"# Getting Started

This Folder explains the default FiniteBrain vault layout.

For humans, read [[HUMANS.md]]. For agents, read [[AGENTS.md]]. For the local
scope map, use [[Getting Started Index]], [[Getting Started Config]], and
[[Getting Started Log]].

Default Folders:

- `getting-started` is the shared orientation scope for users and agents. Keep
  operating rules, onboarding notes, and vault-level guidance here.
- `restricted` is the starter tighter-boundary scope for sensitive work. Do not
  copy restricted titles, summaries, source notes, assets, or logs back here
  unless the intended audience is allowed to read them.

Core starter pages:

- [[How FiniteBrain Works]] explains encrypted server state, local Folder Keys, Pages, Assets, and sync.
- [[Access And Folders]] explains why every Folder is its own wiki boundary.
- [[Restricted Folder Example]] is readable only when that Folder's key is open.

Inside any Folder, keep non-Markdown source files as encrypted Assets under
`raw/assets/`. Pair each Asset with a Markdown Source Note in the same Folder.
Agents and synthesized wiki pages cite the Source Note; the Asset preserves the
original bytes.

Keep durable knowledge inside Folder-scoped `wiki/` pages, and keep private or
sensitive work inside a Folder with an equal or tighter access boundary.

Backlinks to keep this starter graph connected: [[HUMANS.md]], [[AGENTS.md]],
[[Getting Started Index]], [[How FiniteBrain Works]], and [[Access And Folders]].
"#;

const DEFAULT_HOW_FINITEBRAIN_WORKS_MARKDOWN: &str = r#"# How FiniteBrain Works

FiniteBrain stores encrypted Vault data on the server. The client or agent
opens Folder Keys locally, decrypts the Pages and Assets it can access, edits
ordinary files, and syncs encrypted updates back.

This is the technical companion to [[Getting Started]] and should stay
consistent with [[Access And Folders]], [[Getting Started Config]], and
[[AGENTS.md]].

Non-Markdown source files are encrypted as Assets and kept under `raw/assets/`.
Agents use Markdown Source Notes to describe those Assets before synthesizing
durable wiki pages from them.

Each top-level Folder is an LLM wiki scope. A Folder has its own `config.md`,
`_index.md`, and `log.md`, so activity and summaries stay inside the same
access boundary as the content they describe.

Graph View and backlinks are client-side projections over decrypted Pages. The
server stores encrypted objects and sync records; it does not need plaintext
page titles, links, backlinks, or wiki indexes.

Related pages: [[Getting Started]], [[Access And Folders]], [[Getting Started Index]], [[HUMANS.md]], and [[AGENTS.md]].
"#;

const DEFAULT_ACCESS_AND_FOLDERS_MARKDOWN: &str = r#"# Access And Folders

Access is Folder-scoped.

Read this with [[How FiniteBrain Works]]: Folder Keys are why the wiki graph
is built from readable local Pages instead of server-side plaintext indexing.

- `getting-started` is the default shared orientation Folder.
- `restricted` is the default example of a tighter access boundary.
- Open Folders are intended for everyone who belongs in that Vault.
- Restricted Folders are for material that should only be visible to approved
  people.
- Do not copy restricted titles, summaries, Source Notes, Assets, or log entries
  into a less-restricted Folder.

Use [[Getting Started]] and [[Getting Started Index]] for shared orientation.
Use [[Restricted Folder Example]] only when the restricted Folder is readable.
Agent rules live in [[AGENTS.md]], and human-facing orientation lives in
[[HUMANS.md]].
"#;

const DEFAULT_RESTRICTED_EXAMPLE_MARKDOWN: &str = r#"# Restricted Folder Example

This Folder demonstrates a tighter access boundary.

It is the restricted counterpart to [[Getting Started]]. Keep local navigation
in [[Restricted Index]], local rules in [[Restricted Config]], and local history
in [[Restricted Log]].

In an organization Vault, this Folder starts with access for admins only. Add
specific members later when the work in this Folder should be shared with them.

Keep this Folder's `_index.md` and `log.md` local to this Folder. Do not
summarize this Folder into `getting-started` unless the user explicitly chooses
that destination and the audience is allowed to see the summary.

Related shared pages: [[Access And Folders]], [[How FiniteBrain Works]],
[[AGENTS.md]], and [[HUMANS.md]].
"#;

const DEFAULT_PRIMARY_SCOPE_PAGES: [DefaultVaultPage; 5] = [
    DefaultVaultPage {
        folder_id: "getting-started",
        object_id: "obj_default_agents",
        path: "AGENTS.md",
        markdown: DEFAULT_AGENTS_MARKDOWN,
    },
    DefaultVaultPage {
        folder_id: "getting-started",
        object_id: "obj_default_humans",
        path: "HUMANS.md",
        markdown: DEFAULT_HUMANS_MARKDOWN,
    },
    DefaultVaultPage {
        folder_id: "getting-started",
        object_id: "obj_default_getting-started_scope_config",
        path: "config.md",
        markdown: DEFAULT_GETTING_STARTED_SCOPE_CONFIG_MARKDOWN,
    },
    DefaultVaultPage {
        folder_id: "getting-started",
        object_id: "obj_default_getting-started_scope_index",
        path: "_index.md",
        markdown: DEFAULT_GETTING_STARTED_SCOPE_INDEX_MARKDOWN,
    },
    DefaultVaultPage {
        folder_id: "getting-started",
        object_id: "obj_default_getting-started_scope_log",
        path: "log.md",
        markdown: DEFAULT_GETTING_STARTED_SCOPE_LOG_MARKDOWN,
    },
];

const RESTRICTED_SCOPE_PAGES: [DefaultVaultPage; 3] = [
    DefaultVaultPage {
        folder_id: "restricted",
        object_id: "obj_default_restricted_scope_config",
        path: "config.md",
        markdown: DEFAULT_RESTRICTED_SCOPE_CONFIG_MARKDOWN,
    },
    DefaultVaultPage {
        folder_id: "restricted",
        object_id: "obj_default_restricted_scope_index",
        path: "_index.md",
        markdown: DEFAULT_RESTRICTED_SCOPE_INDEX_MARKDOWN,
    },
    DefaultVaultPage {
        folder_id: "restricted",
        object_id: "obj_default_restricted_scope_log",
        path: "log.md",
        markdown: DEFAULT_RESTRICTED_SCOPE_LOG_MARKDOWN,
    },
];

const GETTING_STARTED_GUIDE_PAGES: [DefaultVaultPage; 3] = [
    DefaultVaultPage {
        folder_id: "getting-started",
        object_id: "obj_default_getting-started_readme",
        path: "README.md",
        markdown: DEFAULT_GETTING_STARTED_README_MARKDOWN,
    },
    DefaultVaultPage {
        folder_id: "getting-started",
        object_id: "obj_default_getting-started_how_finitebrain_works",
        path: "wiki/how-finitebrain-works.md",
        markdown: DEFAULT_HOW_FINITEBRAIN_WORKS_MARKDOWN,
    },
    DefaultVaultPage {
        folder_id: "getting-started",
        object_id: "obj_default_getting-started_access_and_folders",
        path: "wiki/access-and-folders.md",
        markdown: DEFAULT_ACCESS_AND_FOLDERS_MARKDOWN,
    },
];

const RESTRICTED_GUIDE_PAGE: DefaultVaultPage = DefaultVaultPage {
    folder_id: "restricted",
    object_id: "obj_default_restricted_example",
    path: "wiki/restricted-folder-example.md",
    markdown: DEFAULT_RESTRICTED_EXAMPLE_MARKDOWN,
};

const PERSONAL_DEFAULT_VAULT_PAGES: [DefaultVaultPage; 12] = [
    DEFAULT_PRIMARY_SCOPE_PAGES[0],
    DEFAULT_PRIMARY_SCOPE_PAGES[1],
    DEFAULT_PRIMARY_SCOPE_PAGES[2],
    DEFAULT_PRIMARY_SCOPE_PAGES[3],
    DEFAULT_PRIMARY_SCOPE_PAGES[4],
    GETTING_STARTED_GUIDE_PAGES[0],
    GETTING_STARTED_GUIDE_PAGES[1],
    GETTING_STARTED_GUIDE_PAGES[2],
    RESTRICTED_SCOPE_PAGES[0],
    RESTRICTED_SCOPE_PAGES[1],
    RESTRICTED_SCOPE_PAGES[2],
    RESTRICTED_GUIDE_PAGE,
];

const ORGANIZATION_DEFAULT_VAULT_PAGES: [DefaultVaultPage; 12] = PERSONAL_DEFAULT_VAULT_PAGES;

/// Returns the crate name used in workspace status surfaces.
pub fn crate_name() -> &'static str {
    "finite-brain-core"
}

/// Default encrypted Pages clients should write after new Vault bootstrap.
pub fn default_vault_pages(kind: VaultKind) -> &'static [DefaultVaultPage] {
    match kind {
        VaultKind::Personal => &PERSONAL_DEFAULT_VAULT_PAGES,
        VaultKind::Organization => &ORGANIZATION_DEFAULT_VAULT_PAGES,
    }
}

/// Primary Folder that receives starter orientation Pages for a new Vault.
pub fn default_vault_pages_folder_id(kind: VaultKind) -> &'static str {
    match kind {
        VaultKind::Personal | VaultKind::Organization => "getting-started",
    }
}

/// Core domain validation errors.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CoreError {
    /// A stable id is empty or not path-safe.
    InvalidId { field: &'static str, value: String },
    /// A display name is empty or contains forbidden characters.
    InvalidName { field: &'static str, value: String },
    /// A path is not a safe relative path.
    InvalidPath { field: &'static str, value: String },
    /// A case-sensitive product identity collision occurred.
    Collision { field: &'static str, value: String },
    /// A folder hierarchy operation is invalid.
    InvalidHierarchy { reason: String },
    /// Bootstrap input is incomplete or violates the Vault kind rules.
    InvalidBootstrapInput { reason: String },
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId { field, value } => {
                write!(f, "invalid id for {field}: {value}")
            }
            Self::InvalidName { field, value } => {
                write!(f, "invalid name for {field}: {value}")
            }
            Self::InvalidPath { field, value } => {
                write!(f, "invalid path for {field}: {value}")
            }
            Self::Collision { field, value } => {
                write!(f, "collision for {field}: {value}")
            }
            Self::InvalidHierarchy { reason } => write!(f, "invalid hierarchy: {reason}"),
            Self::InvalidBootstrapInput { reason } => {
                write!(f, "invalid bootstrap input: {reason}")
            }
        }
    }
}

impl Error for CoreError {}

/// Stable Vault id.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct VaultId(String);

impl VaultId {
    /// Validate and create a Vault id.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        validate_stable_id("vault_id", value.into(), 1, 128).map(Self)
    }

    /// Borrow the normalized id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VaultId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable Folder id.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct FolderId(String);

impl FolderId {
    /// Validate and create a Folder id.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        validate_stable_id("folder_id", value.into(), 1, 128).map(Self)
    }

    /// Borrow the normalized id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable Folder Object id.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct ObjectId(String);

impl ObjectId {
    /// Validate and create a Folder Object id.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let normalized = validate_stable_id("object_id", value.into(), 16, 128)?;
        if normalized.contains('.') {
            return Err(CoreError::InvalidId {
                field: "object_id",
                value: normalized,
            });
        }
        Ok(Self(normalized))
    }

    /// Borrow the normalized id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Nostr user id as stored by FiniteBrain.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct UserId(String);

impl UserId {
    /// Validate and create a user id.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        let normalized = normalize_nfc(&value);
        if normalized.is_empty()
            || normalized.len() > MAX_USER_ID_LEN
            || contains_nul_or_control(&normalized)
        {
            return Err(CoreError::InvalidId {
                field: "user_id",
                value,
            });
        }
        Ok(Self(normalized))
    }

    /// Borrow the normalized id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// User-facing Folder or Vault display name.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct DisplayName(String);

impl DisplayName {
    /// Validate and normalize a display name.
    pub fn new(field: &'static str, value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        let normalized = normalize_nfc(&value);
        if normalized.is_empty()
            || normalized.len() > MAX_DISPLAY_NAME_LEN
            || normalized.contains('/')
            || contains_nul_or_control(&normalized)
            || normalized == "."
            || normalized == ".."
        {
            return Err(CoreError::InvalidName { field, value });
        }
        Ok(Self(normalized))
    }

    /// Borrow the normalized display name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Safe relative path normalized to Unicode NFC.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct SafeRelativePath(String);

impl SafeRelativePath {
    /// Validate a Folder path or decrypted object path.
    pub fn new(field: &'static str, value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        let normalized = normalize_nfc(&value);

        if normalized.is_empty()
            || normalized.len() > MAX_SAFE_RELATIVE_PATH_LEN
            || normalized.starts_with('/')
            || normalized.contains('\\')
            || contains_nul_or_control(&normalized)
        {
            return Err(CoreError::InvalidPath { field, value });
        }

        let segments = normalized.split('/').collect::<Vec<_>>();
        if segments
            .iter()
            .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
        {
            return Err(CoreError::InvalidPath { field, value });
        }

        if RESERVED_TOP_LEVEL_NAMES.contains(&segments[0]) {
            return Err(CoreError::InvalidPath { field, value });
        }

        Ok(Self(normalized))
    }

    /// Borrow the normalized path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SafeRelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Vault kind.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultKind {
    /// Personal Vault owned by one user.
    Personal,
    /// Organization Vault with members and admins.
    Organization,
}

/// Folder role.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderRole {
    /// Personal home folder.
    PersonalHome,
    /// Organization operations/admin folder.
    VaultOps,
    /// Organization general folder.
    General,
    /// Ordinary folder.
    Folder,
}

/// Binary access mode for a Folder.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderAccessMode {
    /// Personal Vault owner only.
    Owner,
    /// Organization Vault admins only.
    AdminOnly,
    /// Organization members and admins.
    AllMembers,
    /// Vault admins, the personal owner for personal Vaults, plus explicitly listed members.
    Restricted,
}

/// Vault member metadata.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct VaultMember {
    /// Member user id.
    pub user_id: UserId,
    /// Explicit restricted Folder Access entries.
    pub folder_access: BTreeSet<FolderId>,
}

/// Folder metadata.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    /// Stable folder id.
    pub id: FolderId,
    /// User-facing display name.
    pub name: DisplayName,
    /// Folder role.
    pub role: FolderRole,
    /// Binary access mode.
    pub access: FolderAccessMode,
    /// Optional parent Folder id.
    pub parent_folder_id: Option<FolderId>,
    /// Decorated Folder hierarchy path.
    pub path: SafeRelativePath,
    /// Current Folder Key version.
    pub current_key_version: u32,
    /// Whether this Folder is a shared-folder source.
    pub shared_folder_source: bool,
}

/// Folder Object metadata without encrypted bytes.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct FolderObject {
    /// Stable object id.
    pub object_id: ObjectId,
    /// Containing Folder id.
    pub folder_id: FolderId,
    /// Encrypted plaintext path.
    pub plaintext_path: SafeRelativePath,
}

/// Vault metadata.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Vault {
    /// Stable Vault id.
    pub id: VaultId,
    /// Vault kind.
    pub kind: VaultKind,
    /// User-facing Vault name.
    pub name: DisplayName,
    /// Personal Vault owner, if this is a personal Vault.
    pub owner_user_id: Option<UserId>,
    /// Folders in this Vault.
    pub folders: Vec<Folder>,
    /// Organization members.
    pub members: Vec<VaultMember>,
    /// Organization admins.
    pub admins: Vec<UserId>,
}

/// Required current Folder Key Grant recipient produced by bootstrap.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RequiredFolderKeyGrant {
    /// Folder receiving a grant.
    pub folder_id: FolderId,
    /// Recipient user id.
    pub recipient_user_id: UserId,
    /// Folder Key version.
    pub key_version: u32,
}

/// Bootstrap output for an initial Vault.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BootstrapOutput {
    /// Created Vault metadata.
    pub vault: Vault,
    /// Required current key grants.
    pub required_key_grants: Vec<RequiredFolderKeyGrant>,
}

/// Mutable pure-domain collection used to enforce hierarchy collisions.
#[derive(Debug, Clone, Default)]
pub struct VaultDraft {
    folders_by_id: BTreeMap<FolderId, Folder>,
    sibling_names: BTreeSet<(Option<FolderId>, DisplayName)>,
    object_paths: BTreeSet<(FolderId, SafeRelativePath)>,
    object_ids: BTreeSet<(FolderId, ObjectId)>,
}

impl VaultDraft {
    /// Add a Folder while enforcing id, parent, and sibling-name uniqueness.
    pub fn add_folder(&mut self, folder: Folder) -> Result<(), CoreError> {
        if self.folders_by_id.contains_key(&folder.id) {
            return Err(CoreError::Collision {
                field: "folder_id",
                value: folder.id.to_string(),
            });
        }

        if let Some(parent_id) = &folder.parent_folder_id
            && !self.folders_by_id.contains_key(parent_id)
        {
            return Err(CoreError::InvalidHierarchy {
                reason: format!("missing parent folder: {parent_id}"),
            });
        }

        let sibling_key = (folder.parent_folder_id.clone(), folder.name.clone());
        if !self.sibling_names.insert(sibling_key) {
            return Err(CoreError::Collision {
                field: "sibling_folder_name",
                value: folder.name.to_string(),
            });
        }

        self.folders_by_id.insert(folder.id.clone(), folder);
        Ok(())
    }

    /// Add a Folder Object while enforcing object id and page-path uniqueness per Folder.
    pub fn add_object(&mut self, object: FolderObject) -> Result<(), CoreError> {
        if !self.folders_by_id.contains_key(&object.folder_id) {
            return Err(CoreError::InvalidHierarchy {
                reason: format!("missing object folder: {}", object.folder_id),
            });
        }

        if !self
            .object_ids
            .insert((object.folder_id.clone(), object.object_id.clone()))
        {
            return Err(CoreError::Collision {
                field: "object_id",
                value: object.object_id.as_str().to_owned(),
            });
        }

        if !self
            .object_paths
            .insert((object.folder_id.clone(), object.plaintext_path.clone()))
        {
            return Err(CoreError::Collision {
                field: "object_path",
                value: object.plaintext_path.to_string(),
            });
        }

        Ok(())
    }

    /// Return folders in id order for deterministic tests/smoke output.
    pub fn folders(&self) -> Vec<Folder> {
        self.folders_by_id.values().cloned().collect()
    }
}

/// Build the initial personal Vault shape.
pub fn bootstrap_personal_vault(
    vault_id: impl Into<String>,
    name: impl Into<String>,
    owner_user_id: impl Into<String>,
) -> Result<BootstrapOutput, CoreError> {
    let vault_id = VaultId::new(vault_id)?;
    let name = DisplayName::new("vault_name", name)?;
    let owner_user_id = UserId::new(owner_user_id)?;

    let folders = vec![
        root_folder(
            "getting-started",
            "getting-started",
            FolderRole::PersonalHome,
            FolderAccessMode::Owner,
        )?,
        root_folder(
            "restricted",
            "restricted",
            FolderRole::Folder,
            FolderAccessMode::Restricted,
        )?,
    ];

    let required_key_grants = folders
        .iter()
        .map(|folder| RequiredFolderKeyGrant {
            folder_id: folder.id.clone(),
            recipient_user_id: owner_user_id.clone(),
            key_version: 1,
        })
        .collect();

    let vault = Vault {
        id: vault_id,
        kind: VaultKind::Personal,
        name,
        owner_user_id: Some(owner_user_id),
        folders,
        members: Vec::new(),
        admins: Vec::new(),
    };

    Ok(BootstrapOutput {
        vault,
        required_key_grants,
    })
}

/// Build the initial organization Vault shape.
pub fn bootstrap_organization_vault(
    vault_id: impl Into<String>,
    name: impl Into<String>,
    admin_user_id: impl Into<String>,
) -> Result<BootstrapOutput, CoreError> {
    let vault_id = VaultId::new(vault_id)?;
    let name = DisplayName::new("vault_name", name)?;
    let admin_user_id = UserId::new(admin_user_id)?;

    let folders = vec![
        root_folder(
            "getting-started",
            "getting-started",
            FolderRole::General,
            FolderAccessMode::AllMembers,
        )?,
        root_folder(
            "restricted",
            "restricted",
            FolderRole::Folder,
            FolderAccessMode::Restricted,
        )?,
    ];

    let required_key_grants = folders
        .iter()
        .map(|folder| RequiredFolderKeyGrant {
            folder_id: folder.id.clone(),
            recipient_user_id: admin_user_id.clone(),
            key_version: 1,
        })
        .collect();

    let vault = Vault {
        id: vault_id,
        kind: VaultKind::Organization,
        name,
        owner_user_id: None,
        folders,
        members: vec![VaultMember {
            user_id: admin_user_id.clone(),
            folder_access: BTreeSet::new(),
        }],
        admins: vec![admin_user_id],
    };

    Ok(BootstrapOutput {
        vault,
        required_key_grants,
    })
}

/// Development-only deterministic bootstrap summary used by the smoke server.
pub fn smoke_bootstrap_summary() -> Result<BootstrapSmokeSummary, CoreError> {
    let personal =
        bootstrap_personal_vault("personal-smoke", "Personal Smoke", "npub-smoke-owner")?;
    let organization =
        bootstrap_organization_vault("org-smoke", "Organization Smoke", "npub-smoke-admin")?;

    Ok(BootstrapSmokeSummary {
        personal: BootstrapVaultSummary::from_output(&personal),
        organization: BootstrapVaultSummary::from_output(&organization),
    })
}

/// Development smoke summary for both bootstrap shapes.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BootstrapSmokeSummary {
    /// Personal Vault summary.
    pub personal: BootstrapVaultSummary,
    /// Organization Vault summary.
    pub organization: BootstrapVaultSummary,
}

/// Compact bootstrap summary safe to return from smoke endpoints.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BootstrapVaultSummary {
    /// Vault kind.
    pub kind: VaultKind,
    /// Folder ids created by bootstrap.
    pub folder_ids: Vec<String>,
    /// Number of current required Folder Key Grants.
    pub required_grants: usize,
    /// Admin count.
    pub admin_count: usize,
    /// Member count.
    pub member_count: usize,
}

impl BootstrapVaultSummary {
    fn from_output(output: &BootstrapOutput) -> Self {
        Self {
            kind: output.vault.kind,
            folder_ids: output
                .vault
                .folders
                .iter()
                .map(|folder| folder.id.to_string())
                .collect(),
            required_grants: output.required_key_grants.len(),
            admin_count: output.vault.admins.len(),
            member_count: output.vault.members.len(),
        }
    }
}

/// Folder Object crypto and signed-record validation errors.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CryptoRecordError {
    /// Encrypted envelope is malformed or unsupported.
    InvalidEnvelope { reason: String },
    /// AES-GCM authentication failed for the expected AAD.
    AadMismatch,
    /// Envelope key version does not match the expected context.
    KeyVersionMismatch { expected: u32, actual: u32 },
    /// Signed payload ciphertext hash does not match the submitted envelope.
    CiphertextHashMismatch { expected: String, actual: String },
    /// Nostr event kind, id, signature, content, or tags did not match.
    EventMismatch { reason: String },
    /// Event signer did not match the payload actor.
    SignerMismatch { expected: String, actual: String },
    /// Operation/action is not allowed for this payload type.
    BadOperation { operation: String },
}

impl fmt::Display for CryptoRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnvelope { reason } => write!(f, "invalid envelope: {reason}"),
            Self::AadMismatch => f.write_str("folder object AAD mismatch"),
            Self::KeyVersionMismatch { expected, actual } => {
                write!(
                    f,
                    "folder key version mismatch: expected {expected}, got {actual}"
                )
            }
            Self::CiphertextHashMismatch { expected, actual } => write!(
                f,
                "ciphertext hash mismatch: expected {expected}, got {actual}"
            ),
            Self::EventMismatch { reason } => write!(f, "signed event mismatch: {reason}"),
            Self::SignerMismatch { expected, actual } => {
                write!(f, "signer mismatch: expected {expected}, got {actual}")
            }
            Self::BadOperation { operation } => write!(f, "bad operation: {operation}"),
        }
    }
}

impl Error for CryptoRecordError {}

/// AES-256-GCM Folder Key.
#[derive(Clone, Eq, PartialEq)]
pub struct FolderKey([u8; 32]);

impl fmt::Debug for FolderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FolderKey([redacted; 32])")
    }
}

impl FolderKey {
    /// Generate a random Folder Key.
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Import raw key bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Import a base64 raw AES-256 key.
    pub fn from_base64(value: &str) -> Result<Self, CryptoRecordError> {
        let bytes =
            BASE64_STANDARD
                .decode(value)
                .map_err(|_| CryptoRecordError::InvalidEnvelope {
                    reason: "folder key is not base64".to_owned(),
                })?;
        let bytes = bytes
            .try_into()
            .map_err(|_| CryptoRecordError::InvalidEnvelope {
                reason: "folder key must be 32 bytes".to_owned(),
            })?;
        Ok(Self(bytes))
    }

    /// Export raw key bytes as base64.
    pub fn to_base64(&self) -> String {
        BASE64_STANDARD.encode(self.0)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(&self.0).expect("FolderKey is exactly 32 bytes")
    }
}

/// Folder Object encryption context used as AES-GCM AAD.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FolderObjectAad {
    /// Vault id.
    pub vault_id: VaultId,
    /// Folder id.
    pub folder_id: FolderId,
    /// Object id.
    pub object_id: ObjectId,
    /// Folder Key version.
    pub key_version: u32,
}

impl FolderObjectAad {
    /// Build the canonical AAD JSON string.
    pub fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"vaultId\":{},\"folderId\":{},\"objectId\":{},\"keyVersion\":{}}}",
            json_string(FOLDER_OBJECT_VERSION),
            json_string(self.vault_id.as_str()),
            json_string(self.folder_id.as_str()),
            json_string(self.object_id.as_str()),
            self.key_version
        )
    }
}

/// `finite-folder-object-v1` encrypted envelope.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct EncryptedFolderObjectEnvelope {
    /// Envelope version.
    pub version: String,
    /// Cipher name.
    pub cipher: String,
    /// Folder Key version.
    #[serde(rename = "keyVersion")]
    pub key_version: u32,
    /// Base64 12-byte AES-GCM nonce.
    pub nonce: String,
    /// Base64 AES-GCM ciphertext plus tag.
    pub ciphertext: String,
}

impl EncryptedFolderObjectEnvelope {
    /// Build the exact canonical envelope string used for hashing.
    pub fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"cipher\":{},\"keyVersion\":{},\"nonce\":{},\"ciphertext\":{}}}",
            json_string(&self.version),
            json_string(&self.cipher),
            self.key_version,
            json_string(&self.nonce),
            json_string(&self.ciphertext)
        )
    }

    /// Parse a canonical or ordinary JSON envelope.
    pub fn from_json(value: &str) -> Result<Self, CryptoRecordError> {
        serde_json::from_str(value).map_err(|_| CryptoRecordError::InvalidEnvelope {
            reason: "envelope JSON did not parse".to_owned(),
        })
    }
}

/// Encrypt plaintext bytes into a canonical Folder Object envelope with a fresh nonce.
pub fn encrypt_folder_object(
    key: &FolderKey,
    aad: &FolderObjectAad,
    plaintext: impl AsRef<[u8]>,
) -> Result<EncryptedFolderObjectEnvelope, CryptoRecordError> {
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce);
    encrypt_folder_object_with_nonce(key, aad, nonce, plaintext)
}

/// Encrypt plaintext bytes with a caller-provided nonce for deterministic vectors/tests.
pub fn encrypt_folder_object_with_nonce(
    key: &FolderKey,
    aad: &FolderObjectAad,
    nonce: [u8; 12],
    plaintext: impl AsRef<[u8]>,
) -> Result<EncryptedFolderObjectEnvelope, CryptoRecordError> {
    let aad_json = aad.canonical_json();
    let ciphertext = key
        .cipher()
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_ref(),
                aad: aad_json.as_bytes(),
            },
        )
        .map_err(|_| CryptoRecordError::InvalidEnvelope {
            reason: "encryption failed".to_owned(),
        })?;

    Ok(EncryptedFolderObjectEnvelope {
        version: FOLDER_OBJECT_VERSION.to_owned(),
        cipher: CIPHER_AES_256_GCM.to_owned(),
        key_version: aad.key_version,
        nonce: BASE64_STANDARD.encode(nonce),
        ciphertext: BASE64_STANDARD.encode(ciphertext),
    })
}

/// Open a `finite-folder-object-v1` envelope using expected AAD.
pub fn open_folder_object(
    key: &FolderKey,
    aad: &FolderObjectAad,
    envelope: &EncryptedFolderObjectEnvelope,
) -> Result<Vec<u8>, CryptoRecordError> {
    validate_envelope_header(aad, envelope)?;

    let nonce = BASE64_STANDARD.decode(&envelope.nonce).map_err(|_| {
        CryptoRecordError::InvalidEnvelope {
            reason: "nonce is not base64".to_owned(),
        }
    })?;
    let nonce: [u8; 12] = nonce
        .try_into()
        .map_err(|_| CryptoRecordError::InvalidEnvelope {
            reason: "nonce must be 12 bytes".to_owned(),
        })?;
    let ciphertext = BASE64_STANDARD.decode(&envelope.ciphertext).map_err(|_| {
        CryptoRecordError::InvalidEnvelope {
            reason: "ciphertext is not base64".to_owned(),
        }
    })?;
    let aad_json = aad.canonical_json();

    key.cipher()
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: aad_json.as_bytes(),
            },
        )
        .map_err(|_| CryptoRecordError::AadMismatch)
}

/// SHA-256 hex digest of an exact string.
pub fn sha256_hex(input: impl AsRef<[u8]>) -> String {
    let digest = Sha256::digest(input.as_ref());
    hex_encode(&digest)
}

/// Hash the exact serialized encrypted envelope string.
pub fn ciphertext_hash(envelope_json: &str) -> String {
    sha256_hex(envelope_json.as_bytes())
}

/// Folder Object revision operation.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FolderObjectOperation {
    /// Create a new object.
    Create,
    /// Update an existing object.
    Update,
    /// Move an existing object to a new plaintext path.
    Move,
}

impl FolderObjectOperation {
    /// String representation used in signed payloads and tags.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Move => "move",
        }
    }
}

impl TryFrom<&str> for FolderObjectOperation {
    type Error = CryptoRecordError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            "move" => Ok(Self::Move),
            _ => Err(CryptoRecordError::BadOperation {
                operation: value.to_owned(),
            }),
        }
    }
}

/// Signed Folder Object revision payload.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub struct FolderObjectRevisionPayload {
    /// Payload version.
    pub version: String,
    /// Vault id.
    #[serde(rename = "vaultId")]
    pub vault_id: String,
    /// Folder id.
    #[serde(rename = "folderId")]
    pub folder_id: String,
    /// Object id.
    #[serde(rename = "objectId")]
    pub object_id: String,
    /// Operation.
    pub operation: String,
    /// New revision number.
    pub revision: u64,
    /// Base revision.
    #[serde(rename = "baseRevision")]
    pub base_revision: Option<u64>,
    /// Folder Key version.
    #[serde(rename = "keyVersion")]
    pub key_version: u32,
    /// Cipher.
    pub cipher: String,
    /// Ciphertext hash.
    #[serde(rename = "ciphertextHash")]
    pub ciphertext_hash: String,
    /// Author npub.
    #[serde(rename = "authorNpub")]
    pub author_npub: String,
    /// Creation timestamp.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl FolderObjectRevisionPayload {
    /// Create a signed revision payload.
    pub fn new(input: &RevisionValidation) -> Self {
        Self {
            version: "finite-folder-object-revision-v1".to_owned(),
            vault_id: input.vault_id.to_string(),
            folder_id: input.folder_id.to_string(),
            object_id: input.object_id.as_str().to_owned(),
            operation: input.operation.as_str().to_owned(),
            revision: input.revision,
            base_revision: input.base_revision,
            key_version: input.key_version,
            cipher: CIPHER_AES_256_GCM.to_owned(),
            ciphertext_hash: ciphertext_hash(&input.envelope_json),
            author_npub: input.author_npub.clone(),
            created_at: input.created_at.clone(),
        }
    }

    /// Canonical JSON in spec field order.
    pub fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"vaultId\":{},\"folderId\":{},\"objectId\":{},\"operation\":{},\"revision\":{},\"baseRevision\":{},\"keyVersion\":{},\"cipher\":{},\"ciphertextHash\":{},\"authorNpub\":{},\"createdAt\":{}}}",
            json_string(&self.version),
            json_string(&self.vault_id),
            json_string(&self.folder_id),
            json_string(&self.object_id),
            json_string(&self.operation),
            self.revision,
            json_optional_u64(self.base_revision),
            self.key_version,
            json_string(&self.cipher),
            json_string(&self.ciphertext_hash),
            json_string(&self.author_npub),
            json_string(&self.created_at)
        )
    }
}

/// Expected values for validating a signed revision event.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RevisionValidation {
    /// Vault id.
    pub vault_id: VaultId,
    /// Folder id.
    pub folder_id: FolderId,
    /// Object id.
    pub object_id: ObjectId,
    /// Operation.
    pub operation: FolderObjectOperation,
    /// New revision.
    pub revision: u64,
    /// Expected base revision.
    pub base_revision: Option<u64>,
    /// Folder Key version.
    pub key_version: u32,
    /// Exact serialized encrypted envelope string submitted in the request.
    pub envelope_json: String,
    /// Expected actor/author npub.
    pub author_npub: String,
    /// Expected payload timestamp.
    pub created_at: String,
}

/// Validate a signed create/update/move Folder Object revision event.
pub fn validate_revision_event(
    event: &Event,
    expected: &RevisionValidation,
) -> Result<FolderObjectRevisionPayload, CryptoRecordError> {
    validate_event_integrity(event)?;
    validate_revision_envelope(expected)?;
    let payload: FolderObjectRevisionPayload = parse_event_content(event)?;
    FolderObjectOperation::try_from(payload.operation.as_str())?;

    if payload.canonical_json() != event.content {
        return Err(CryptoRecordError::EventMismatch {
            reason: "revision payload is not canonical".to_owned(),
        });
    }

    let expected_payload = FolderObjectRevisionPayload::new(expected);
    if payload != expected_payload {
        if payload.ciphertext_hash != expected_payload.ciphertext_hash {
            return Err(CryptoRecordError::CiphertextHashMismatch {
                expected: expected_payload.ciphertext_hash,
                actual: payload.ciphertext_hash,
            });
        }
        return Err(CryptoRecordError::EventMismatch {
            reason: "revision payload fields differ from expected request".to_owned(),
        });
    }

    validate_signer(event, &payload.author_npub)?;
    require_exact_tags(event, revision_tags(expected))?;
    Ok(payload)
}

fn validate_revision_envelope(expected: &RevisionValidation) -> Result<(), CryptoRecordError> {
    let envelope = EncryptedFolderObjectEnvelope::from_json(&expected.envelope_json)?;
    let aad = FolderObjectAad {
        vault_id: expected.vault_id.clone(),
        folder_id: expected.folder_id.clone(),
        object_id: expected.object_id.clone(),
        key_version: expected.key_version,
    };
    validate_envelope_header(&aad, &envelope)
}

/// Signed Folder Object tombstone payload.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub struct FolderObjectTombstonePayload {
    /// Payload version.
    pub version: String,
    /// Vault id.
    #[serde(rename = "vaultId")]
    pub vault_id: String,
    /// Folder id.
    #[serde(rename = "folderId")]
    pub folder_id: String,
    /// Object id.
    #[serde(rename = "objectId")]
    pub object_id: String,
    /// Operation, always delete.
    pub operation: String,
    /// New revision number.
    pub revision: u64,
    /// Base revision.
    #[serde(rename = "baseRevision")]
    pub base_revision: u64,
    /// Author npub.
    #[serde(rename = "authorNpub")]
    pub author_npub: String,
    /// Deletion timestamp.
    #[serde(rename = "deletedAt")]
    pub deleted_at: String,
}

impl FolderObjectTombstonePayload {
    /// Create a signed tombstone payload.
    pub fn new(input: &TombstoneValidation) -> Self {
        Self {
            version: "finite-folder-object-tombstone-v1".to_owned(),
            vault_id: input.vault_id.to_string(),
            folder_id: input.folder_id.to_string(),
            object_id: input.object_id.as_str().to_owned(),
            operation: "delete".to_owned(),
            revision: input.revision,
            base_revision: input.base_revision,
            author_npub: input.author_npub.clone(),
            deleted_at: input.deleted_at.clone(),
        }
    }

    /// Canonical JSON in spec field order.
    pub fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"vaultId\":{},\"folderId\":{},\"objectId\":{},\"operation\":{},\"revision\":{},\"baseRevision\":{},\"authorNpub\":{},\"deletedAt\":{}}}",
            json_string(&self.version),
            json_string(&self.vault_id),
            json_string(&self.folder_id),
            json_string(&self.object_id),
            json_string(&self.operation),
            self.revision,
            self.base_revision,
            json_string(&self.author_npub),
            json_string(&self.deleted_at)
        )
    }
}

/// Expected values for validating a signed tombstone event.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TombstoneValidation {
    /// Vault id.
    pub vault_id: VaultId,
    /// Folder id.
    pub folder_id: FolderId,
    /// Object id.
    pub object_id: ObjectId,
    /// New tombstone revision.
    pub revision: u64,
    /// Previous revision.
    pub base_revision: u64,
    /// Expected actor/author npub.
    pub author_npub: String,
    /// Expected deletion timestamp.
    pub deleted_at: String,
}

/// Validate a signed delete/tombstone event.
pub fn validate_tombstone_event(
    event: &Event,
    expected: &TombstoneValidation,
) -> Result<FolderObjectTombstonePayload, CryptoRecordError> {
    validate_event_integrity(event)?;
    let payload: FolderObjectTombstonePayload = parse_event_content(event)?;

    if payload.operation != "delete" {
        return Err(CryptoRecordError::BadOperation {
            operation: payload.operation,
        });
    }

    if payload.canonical_json() != event.content {
        return Err(CryptoRecordError::EventMismatch {
            reason: "tombstone payload is not canonical".to_owned(),
        });
    }

    let expected_payload = FolderObjectTombstonePayload::new(expected);
    if payload != expected_payload {
        return Err(CryptoRecordError::EventMismatch {
            reason: "tombstone payload fields differ from expected request".to_owned(),
        });
    }

    validate_signer(event, &payload.author_npub)?;
    require_exact_tags(event, tombstone_tags(expected))?;
    Ok(payload)
}

/// Admin access-change action.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AdminAccessAction {
    /// Add member.
    AddMember,
    /// Remove member.
    RemoveMember,
    /// Add admin.
    AddAdmin,
    /// Remove admin.
    RemoveAdmin,
    /// Grant restricted folder access.
    GrantFolderAccess,
    /// Remove restricted folder access.
    RemoveFolderAccess,
    /// Rotate a Folder Key.
    RotateFolderKey,
    /// Change Folder Access mode.
    SetFolderAccessMode,
}

impl AdminAccessAction {
    /// String representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AddMember => "add-member",
            Self::RemoveMember => "remove-member",
            Self::AddAdmin => "add-admin",
            Self::RemoveAdmin => "remove-admin",
            Self::GrantFolderAccess => "grant-folder-access",
            Self::RemoveFolderAccess => "remove-folder-access",
            Self::RotateFolderKey => "rotate-folder-key",
            Self::SetFolderAccessMode => "set-folder-access-mode",
        }
    }
}

impl TryFrom<&str> for AdminAccessAction {
    type Error = CryptoRecordError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "add-member" => Ok(Self::AddMember),
            "remove-member" => Ok(Self::RemoveMember),
            "add-admin" => Ok(Self::AddAdmin),
            "remove-admin" => Ok(Self::RemoveAdmin),
            "grant-folder-access" => Ok(Self::GrantFolderAccess),
            "remove-folder-access" => Ok(Self::RemoveFolderAccess),
            "rotate-folder-key" => Ok(Self::RotateFolderKey),
            "set-folder-access-mode" => Ok(Self::SetFolderAccessMode),
            _ => Err(CryptoRecordError::BadOperation {
                operation: value.to_owned(),
            }),
        }
    }
}

/// Signed Vault admin access-change payload.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub struct AdminAccessChangePayload {
    /// Payload version.
    pub version: String,
    /// Vault id.
    #[serde(rename = "vaultId")]
    pub vault_id: String,
    /// Change id.
    #[serde(rename = "changeId")]
    pub change_id: String,
    /// Action.
    pub action: String,
    /// Admin npub.
    #[serde(rename = "adminNpub")]
    pub admin_npub: String,
    /// Optional folder id.
    #[serde(rename = "folderId")]
    pub folder_id: Option<String>,
    /// Optional target npub.
    #[serde(rename = "targetNpub")]
    pub target_npub: Option<String>,
    /// Optional key version.
    #[serde(rename = "keyVersion")]
    pub key_version: Option<u32>,
    /// Optional note.
    pub note: Option<String>,
    /// Creation timestamp.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl AdminAccessChangePayload {
    /// Create an access-change payload.
    pub fn new(input: &AdminAccessChangeValidation) -> Self {
        Self {
            version: "finite-vault-admin-access-change-v1".to_owned(),
            vault_id: input.vault_id.to_string(),
            change_id: input.change_id.clone(),
            action: input.action.as_str().to_owned(),
            admin_npub: input.admin_npub.clone(),
            folder_id: input.folder_id.as_ref().map(ToString::to_string),
            target_npub: input.target_npub.clone(),
            key_version: input.key_version,
            note: input.note.clone(),
            created_at: input.created_at.clone(),
        }
    }

    /// Canonical JSON in spec field order, omitting absent optional fields.
    pub fn canonical_json(&self) -> String {
        let mut fields = vec![
            format!("\"version\":{}", json_string(&self.version)),
            format!("\"vaultId\":{}", json_string(&self.vault_id)),
            format!("\"changeId\":{}", json_string(&self.change_id)),
            format!("\"action\":{}", json_string(&self.action)),
            format!("\"adminNpub\":{}", json_string(&self.admin_npub)),
        ];

        if let Some(folder_id) = &self.folder_id {
            fields.push(format!("\"folderId\":{}", json_string(folder_id)));
        }
        if let Some(target_npub) = &self.target_npub {
            fields.push(format!("\"targetNpub\":{}", json_string(target_npub)));
        }
        if let Some(key_version) = self.key_version {
            fields.push(format!("\"keyVersion\":{key_version}"));
        }
        if let Some(note) = &self.note {
            fields.push(format!("\"note\":{}", json_string(note)));
        }
        fields.push(format!("\"createdAt\":{}", json_string(&self.created_at)));

        format!("{{{}}}", fields.join(","))
    }
}

/// Expected values for validating an admin access-change event.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AdminAccessChangeValidation {
    /// Vault id.
    pub vault_id: VaultId,
    /// Change id.
    pub change_id: String,
    /// Action.
    pub action: AdminAccessAction,
    /// Admin npub.
    pub admin_npub: String,
    /// Optional folder id.
    pub folder_id: Option<FolderId>,
    /// Optional target npub.
    pub target_npub: Option<String>,
    /// Optional key version.
    pub key_version: Option<u32>,
    /// Optional note.
    pub note: Option<String>,
    /// Expected timestamp.
    pub created_at: String,
}

/// Validate a signed Vault admin access-change event.
pub fn validate_admin_access_change_event(
    event: &Event,
    expected: &AdminAccessChangeValidation,
) -> Result<AdminAccessChangePayload, CryptoRecordError> {
    validate_event_integrity(event)?;
    let payload: AdminAccessChangePayload = parse_event_content(event)?;
    AdminAccessAction::try_from(payload.action.as_str())?;

    if payload.canonical_json() != event.content {
        return Err(CryptoRecordError::EventMismatch {
            reason: "access-change payload is not canonical".to_owned(),
        });
    }

    let expected_payload = AdminAccessChangePayload::new(expected);
    if payload != expected_payload {
        return Err(CryptoRecordError::EventMismatch {
            reason: "access-change payload fields differ from expected request".to_owned(),
        });
    }

    validate_signer(event, &payload.admin_npub)?;
    require_exact_tags(event, admin_access_change_tags(expected)?)?;
    Ok(payload)
}

fn validate_envelope_header(
    aad: &FolderObjectAad,
    envelope: &EncryptedFolderObjectEnvelope,
) -> Result<(), CryptoRecordError> {
    if envelope.version != FOLDER_OBJECT_VERSION {
        return Err(CryptoRecordError::InvalidEnvelope {
            reason: "unsupported version".to_owned(),
        });
    }
    if envelope.cipher != CIPHER_AES_256_GCM {
        return Err(CryptoRecordError::InvalidEnvelope {
            reason: "unsupported cipher".to_owned(),
        });
    }
    if envelope.key_version != aad.key_version {
        return Err(CryptoRecordError::KeyVersionMismatch {
            expected: aad.key_version,
            actual: envelope.key_version,
        });
    }
    Ok(())
}

fn validate_event_integrity(event: &Event) -> Result<(), CryptoRecordError> {
    if event.kind != Kind::ApplicationSpecificData {
        return Err(CryptoRecordError::EventMismatch {
            reason: format!(
                "expected kind {APP_SPECIFIC_KIND}, got {}",
                event.kind.as_u16()
            ),
        });
    }
    verify_event_integrity(event).map_err(|error| CryptoRecordError::EventMismatch {
        reason: error.to_string(),
    })
}

fn validate_signer(event: &Event, expected_npub: &str) -> Result<(), CryptoRecordError> {
    let actual = NostrPublicKey::from_protocol(event.pubkey)
        .to_npub()
        .map_err(|error| CryptoRecordError::EventMismatch {
            reason: error.to_string(),
        })?;

    if actual != expected_npub {
        return Err(CryptoRecordError::SignerMismatch {
            expected: expected_npub.to_owned(),
            actual,
        });
    }

    Ok(())
}

fn parse_event_content<T>(event: &Event) -> Result<T, CryptoRecordError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(&event.content).map_err(|_| CryptoRecordError::EventMismatch {
        reason: "payload JSON did not parse".to_owned(),
    })
}

fn require_exact_tags(event: &Event, expected: Vec<Vec<String>>) -> Result<(), CryptoRecordError> {
    let actual = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();

    if actual != expected {
        return Err(CryptoRecordError::EventMismatch {
            reason: "event tags differ from payload".to_owned(),
        });
    }

    Ok(())
}

fn revision_tags(input: &RevisionValidation) -> Vec<Vec<String>> {
    vec![
        vec![
            "d".to_owned(),
            format!(
                "finite-folder-object-revision:{}:{}:{}:{}",
                input.vault_id,
                input.folder_id,
                input.object_id.as_str(),
                input.revision
            ),
        ],
        vec!["vault".to_owned(), input.vault_id.to_string()],
        vec!["folder".to_owned(), input.folder_id.to_string()],
        vec!["object".to_owned(), input.object_id.as_str().to_owned()],
        vec!["operation".to_owned(), input.operation.as_str().to_owned()],
        vec!["keyVersion".to_owned(), input.key_version.to_string()],
    ]
}

fn tombstone_tags(input: &TombstoneValidation) -> Vec<Vec<String>> {
    vec![
        vec![
            "d".to_owned(),
            format!(
                "finite-folder-object-tombstone:{}:{}:{}:{}",
                input.vault_id,
                input.folder_id,
                input.object_id.as_str(),
                input.revision
            ),
        ],
        vec!["vault".to_owned(), input.vault_id.to_string()],
        vec!["folder".to_owned(), input.folder_id.to_string()],
        vec!["object".to_owned(), input.object_id.as_str().to_owned()],
        vec!["operation".to_owned(), "delete".to_owned()],
    ]
}

fn admin_access_change_tags(
    input: &AdminAccessChangeValidation,
) -> Result<Vec<Vec<String>>, CryptoRecordError> {
    let mut tags = vec![
        vec![
            "d".to_owned(),
            format!(
                "finite-vault-admin-access-change:{}:{}",
                input.vault_id, input.change_id
            ),
        ],
        vec!["vault".to_owned(), input.vault_id.to_string()],
        vec!["action".to_owned(), input.action.as_str().to_owned()],
    ];

    if let Some(folder_id) = &input.folder_id {
        tags.push(vec!["folder".to_owned(), folder_id.to_string()]);
    }
    if let Some(target_npub) = &input.target_npub {
        let target_hex = NostrPublicKey::parse(target_npub)
            .map_err(|error| CryptoRecordError::EventMismatch {
                reason: error.to_string(),
            })?
            .to_hex();
        tags.push(vec!["p".to_owned(), target_hex]);
    }
    if let Some(key_version) = input.key_version {
        tags.push(vec!["keyVersion".to_owned(), key_version.to_string()]);
    }

    Ok(tags)
}

fn json_optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing string cannot fail")
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn root_folder(
    id: &str,
    name: &str,
    role: FolderRole,
    access: FolderAccessMode,
) -> Result<Folder, CoreError> {
    if RESERVED_TOP_LEVEL_NAMES.contains(&name) {
        return Err(CoreError::InvalidName {
            field: "folder_name",
            value: name.to_owned(),
        });
    }

    Ok(Folder {
        id: FolderId::new(id)?,
        name: DisplayName::new("folder_name", name)?,
        role,
        access,
        parent_folder_id: None,
        path: SafeRelativePath::new("folder_path", name)?,
        current_key_version: 1,
        shared_folder_source: false,
    })
}

fn validate_stable_id(
    field: &'static str,
    value: String,
    min_len: usize,
    max_len: usize,
) -> Result<String, CoreError> {
    let normalized = normalize_nfc(&value);
    let valid_len = (min_len..=max_len).contains(&normalized.len());
    let valid_chars = normalized
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');

    if !valid_len || !valid_chars {
        return Err(CoreError::InvalidId { field, value });
    }

    Ok(normalized)
}

fn normalize_nfc(value: &str) -> String {
    value.nfc().collect::<String>()
}

fn contains_nul_or_control(value: &str) -> bool {
    value.chars().any(|c| c == '\0' || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::event::FinalizeEvent;
    use nostr::{EventBuilder, Keys, Tag, Timestamp};

    #[test]
    fn exposes_core_crate_name() {
        assert_eq!(crate_name(), "finite-brain-core");
    }

    #[test]
    fn exposes_default_vault_pages() {
        let pages = default_vault_pages(VaultKind::Personal);

        assert_eq!(
            pages
                .iter()
                .take(5)
                .map(|page| (page.folder_id, page.object_id, page.path))
                .collect::<Vec<_>>(),
            vec![
                ("getting-started", "obj_default_agents", "AGENTS.md"),
                ("getting-started", "obj_default_humans", "HUMANS.md"),
                (
                    "getting-started",
                    "obj_default_getting-started_scope_config",
                    "config.md"
                ),
                (
                    "getting-started",
                    "obj_default_getting-started_scope_index",
                    "_index.md"
                ),
                (
                    "getting-started",
                    "obj_default_getting-started_scope_log",
                    "log.md"
                )
            ]
        );
        assert_eq!(pages.len(), 12);
        assert_eq!(
            pages
                .iter()
                .map(|page| page.object_id)
                .collect::<BTreeSet<_>>()
                .len(),
            pages.len()
        );
        assert!(pages.iter().any(|page| page.path == "README.md"));
        let getting_started_readme = pages
            .iter()
            .find(|page| page.path == "README.md")
            .expect("getting-started README exists");
        assert!(getting_started_readme.markdown.contains("Default Folders"));
        assert!(
            getting_started_readme
                .markdown
                .contains("encrypted Assets under")
        );
        assert!(getting_started_readme.markdown.contains("Source Note"));
        assert!(
            pages
                .iter()
                .any(|page| page.path == "wiki/access-and-folders.md")
        );
        assert!(pages.iter().any(|page| page.folder_id == "restricted"));
        assert!(pages[0].markdown.contains("Use `fbrain`"));
        assert!(pages[0].markdown.contains("LLM Wiki Rules"));
        assert!(pages[0].markdown.contains("raw/assets/"));
        assert!(pages[0].markdown.contains("Source Note"));
        assert!(
            pages[1]
                .markdown
                .contains("private, encrypted knowledge workspace")
        );
        assert!(pages[1].markdown.contains("Source Notes"));
        assert!(pages[2].markdown.contains("raw/assets/"));
        assert!(pages[2].markdown.contains("Source Note"));
        assert!(pages[2].markdown.contains("# Getting Started Config"));
        assert!(pages[3].markdown.contains("# Getting Started Index"));
        assert!(pages[4].markdown.contains("# Getting Started Log"));
        assert_eq!(
            pages
                .iter()
                .filter(|page| page.markdown.contains("[["))
                .count(),
            pages.len()
        );
        assert!(
            !pages
                .iter()
                .any(|page| page.markdown.contains("[[wikilinks]]"))
        );
        let organization_pages = default_vault_pages(VaultKind::Organization);
        assert_eq!(organization_pages.len(), 12);
        assert_eq!(
            organization_pages
                .iter()
                .map(|page| page.object_id)
                .collect::<BTreeSet<_>>()
                .len(),
            organization_pages.len()
        );
        assert!(
            organization_pages
                .iter()
                .all(|page| page.folder_id != "vault-ops")
        );
        assert!(
            organization_pages
                .iter()
                .all(|page| page.folder_id != "product")
        );
        assert!(
            organization_pages
                .iter()
                .any(|page| page.folder_id == "restricted"
                    && page.path == "wiki/restricted-folder-example.md")
        );
        assert!(
            organization_pages[9]
                .markdown
                .contains("# Restricted Index")
        );
        assert_eq!(
            default_vault_pages_folder_id(VaultKind::Personal),
            "getting-started"
        );
        assert_eq!(
            default_vault_pages_folder_id(VaultKind::Organization),
            "getting-started"
        );
    }

    #[test]
    fn bootstraps_personal_vault() {
        let output = bootstrap_personal_vault("personal", "Austin", "npub-owner").unwrap();

        assert_eq!(output.vault.kind, VaultKind::Personal);
        assert_eq!(
            output.vault.owner_user_id,
            Some(UserId::new("npub-owner").unwrap())
        );
        assert!(output.vault.members.is_empty());
        assert!(output.vault.admins.is_empty());
        assert_eq!(output.vault.folders.len(), 2);

        let getting_started = &output.vault.folders[0];
        assert_eq!(
            getting_started.id,
            FolderId::new("getting-started").unwrap()
        );
        assert_eq!(getting_started.role, FolderRole::PersonalHome);
        assert_eq!(getting_started.access, FolderAccessMode::Owner);
        assert_eq!(getting_started.current_key_version, 1);
        assert_eq!(
            getting_started.path,
            SafeRelativePath::new("folder_path", "getting-started").unwrap()
        );

        let restricted = &output.vault.folders[1];
        assert_eq!(restricted.id, FolderId::new("restricted").unwrap());
        assert_eq!(restricted.role, FolderRole::Folder);
        assert_eq!(restricted.access, FolderAccessMode::Restricted);
        assert_eq!(
            restricted.path,
            SafeRelativePath::new("folder_path", "restricted").unwrap()
        );
        assert_eq!(
            output
                .required_key_grants
                .iter()
                .map(|grant| grant.folder_id.to_string())
                .collect::<Vec<_>>(),
            vec!["getting-started", "restricted"]
        );
        assert!(
            output
                .required_key_grants
                .iter()
                .all(
                    |grant| grant.recipient_user_id == UserId::new("npub-owner").unwrap()
                        && grant.key_version == 1
                )
        );
    }

    #[test]
    fn bootstraps_organization_vault() {
        let output = bootstrap_organization_vault("org", "Finite", "npub-admin").unwrap();

        assert_eq!(output.vault.kind, VaultKind::Organization);
        assert_eq!(output.vault.owner_user_id, None);
        assert_eq!(
            output.vault.admins,
            vec![UserId::new("npub-admin").unwrap()]
        );
        assert_eq!(output.vault.members.len(), 1);
        assert_eq!(
            output.vault.members[0].user_id,
            UserId::new("npub-admin").unwrap()
        );
        assert_eq!(output.vault.folders.len(), 2);
        assert_eq!(output.required_key_grants.len(), 2);
        assert_eq!(
            output
                .required_key_grants
                .iter()
                .map(|grant| grant.folder_id.to_string())
                .collect::<Vec<_>>(),
            vec!["getting-started", "restricted"]
        );

        let getting_started = &output.vault.folders[0];
        assert_eq!(
            getting_started.id,
            FolderId::new("getting-started").unwrap()
        );
        assert_eq!(getting_started.role, FolderRole::General);
        assert_eq!(getting_started.access, FolderAccessMode::AllMembers);

        let restricted = &output.vault.folders[1];
        assert_eq!(restricted.id, FolderId::new("restricted").unwrap());
        assert_eq!(restricted.role, FolderRole::Folder);
        assert_eq!(restricted.access, FolderAccessMode::Restricted);
    }

    #[test]
    fn validates_paths_and_names() {
        let decomposed = "Cafe\u{301}/notes.md";
        let path = SafeRelativePath::new("page_path", decomposed).unwrap();
        assert_eq!(path.as_str(), "Café/notes.md");

        assert_eq!(
            SafeRelativePath::new("page_path", "/absolute").unwrap_err(),
            CoreError::InvalidPath {
                field: "page_path",
                value: "/absolute".to_owned()
            }
        );
        assert_eq!(
            SafeRelativePath::new("page_path", "a/../b").unwrap_err(),
            CoreError::InvalidPath {
                field: "page_path",
                value: "a/../b".to_owned()
            }
        );
        assert_eq!(
            SafeRelativePath::new("page_path", ".git/config").unwrap_err(),
            CoreError::InvalidPath {
                field: "page_path",
                value: ".git/config".to_owned()
            }
        );
        assert!(DisplayName::new("folder_name", "bad/name").is_err());
        assert!(DisplayName::new("folder_name", "bad\u{0}name").is_err());
        assert!(DisplayName::new("folder_name", "x".repeat(MAX_DISPLAY_NAME_LEN + 1)).is_err());
        assert!(SafeRelativePath::new("page_path", format!("{}.md", "x".repeat(1025))).is_err());
        assert!(UserId::new("x".repeat(MAX_USER_ID_LEN + 1)).is_err());
        assert!(ObjectId::new("too-short").is_err());
        assert!(ObjectId::new("object_id_with_extension.md").is_err());
    }

    #[test]
    fn folder_and_page_collisions_are_case_sensitive() {
        let mut draft = VaultDraft::default();
        let root = root_folder(
            "root",
            "Root",
            FolderRole::Folder,
            FolderAccessMode::Restricted,
        )
        .unwrap();
        draft.add_folder(root.clone()).unwrap();

        let duplicate = Folder {
            id: FolderId::new("other").unwrap(),
            name: root.name.clone(),
            role: FolderRole::Folder,
            access: FolderAccessMode::Restricted,
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "Root 2").unwrap(),
            current_key_version: 1,
            shared_folder_source: false,
        };
        assert_eq!(
            draft.add_folder(duplicate).unwrap_err(),
            CoreError::Collision {
                field: "sibling_folder_name",
                value: "Root".to_owned()
            }
        );

        draft
            .add_folder(Folder {
                id: FolderId::new("lower").unwrap(),
                name: DisplayName::new("folder_name", "root").unwrap(),
                role: FolderRole::Folder,
                access: FolderAccessMode::Restricted,
                parent_folder_id: None,
                path: SafeRelativePath::new("folder_path", "root").unwrap(),
                current_key_version: 1,
                shared_folder_source: false,
            })
            .unwrap();

        let object = FolderObject {
            object_id: ObjectId::new("object_0000000001").unwrap(),
            folder_id: root.id.clone(),
            plaintext_path: SafeRelativePath::new("page_path", "wiki/Intro.md").unwrap(),
        };
        draft.add_object(object.clone()).unwrap();
        assert_eq!(
            draft
                .add_object(FolderObject {
                    object_id: ObjectId::new("object_0000000002").unwrap(),
                    ..object
                })
                .unwrap_err(),
            CoreError::Collision {
                field: "object_path",
                value: "wiki/Intro.md".to_owned()
            }
        );
    }

    #[test]
    fn child_access_is_independent_from_parent_access() {
        let mut draft = VaultDraft::default();
        let parent = root_folder(
            "parent",
            "Parent",
            FolderRole::Folder,
            FolderAccessMode::AllMembers,
        )
        .unwrap();
        draft.add_folder(parent.clone()).unwrap();

        let child = Folder {
            id: FolderId::new("child").unwrap(),
            name: DisplayName::new("folder_name", "Child").unwrap(),
            role: FolderRole::Folder,
            access: FolderAccessMode::Restricted,
            parent_folder_id: Some(parent.id.clone()),
            path: SafeRelativePath::new("folder_path", "Parent/Child").unwrap(),
            current_key_version: 1,
            shared_folder_source: false,
        };
        draft.add_folder(child.clone()).unwrap();

        let folders = draft.folders();
        let stored_parent = folders
            .iter()
            .find(|folder| folder.id == parent.id)
            .unwrap();
        let stored_child = folders.iter().find(|folder| folder.id == child.id).unwrap();

        assert_eq!(stored_parent.access, FolderAccessMode::AllMembers);
        assert_eq!(stored_child.access, FolderAccessMode::Restricted);
        assert_ne!(stored_parent.access, stored_child.access);
    }

    #[test]
    fn rejects_invalid_hierarchy() {
        let mut draft = VaultDraft::default();
        let orphan = Folder {
            id: FolderId::new("orphan").unwrap(),
            name: DisplayName::new("folder_name", "Orphan").unwrap(),
            role: FolderRole::Folder,
            access: FolderAccessMode::Restricted,
            parent_folder_id: Some(FolderId::new("missing").unwrap()),
            path: SafeRelativePath::new("folder_path", "Missing/Orphan").unwrap(),
            current_key_version: 1,
            shared_folder_source: false,
        };

        assert_eq!(
            draft.add_folder(orphan).unwrap_err(),
            CoreError::InvalidHierarchy {
                reason: "missing parent folder: missing".to_owned()
            }
        );
    }

    #[test]
    fn smoke_bootstrap_summary_is_stable() {
        let summary = smoke_bootstrap_summary().unwrap();

        assert_eq!(summary.personal.kind, VaultKind::Personal);
        assert_eq!(
            summary.personal.folder_ids,
            vec!["getting-started", "restricted"]
        );
        assert_eq!(summary.personal.required_grants, 2);

        assert_eq!(summary.organization.kind, VaultKind::Organization);
        assert_eq!(
            summary.organization.folder_ids,
            vec!["getting-started".to_owned(), "restricted".to_owned()]
        );
        assert_eq!(summary.organization.required_grants, 2);
        assert_eq!(summary.organization.admin_count, 1);
        assert_eq!(summary.organization.member_count, 1);
    }

    #[test]
    fn hashes_canonical_spec_vectors() {
        let request_body = r#"{"recordType":"folder_object_revision","folderId":"strategy","objectId":"obj_0123456789abcdef"}"#;
        assert_eq!(
            sha256_hex(request_body),
            "beb370cd8804a3a4e7b4764f1f7fdf4bac95895004513a19abee515a2b9c55e4"
        );

        let envelope = r#"{"version":"finite-folder-object-v1","cipher":"AES-256-GCM","keyVersion":1,"nonce":"AAAAAAAAAAAAAAAA","ciphertext":"AQIDBAUGBwgJCgsMDQ4PEA=="}"#;
        assert_eq!(
            ciphertext_hash(envelope),
            "9083fa9666f921de7da1d0b435903e98045b27a1065030dc6d4c841d2374b5bb"
        );
    }

    #[test]
    fn encrypts_and_opens_folder_object_with_aad() {
        let key = FolderKey::from_bytes([7; 32]);
        let aad = folder_object_aad(1);
        let plaintext = br#"{"path":"wiki/concepts/example.md","body":"hello"}"#;

        let envelope = encrypt_folder_object_with_nonce(&key, &aad, [0; 12], plaintext).unwrap();

        assert_eq!(envelope.version, FOLDER_OBJECT_VERSION);
        assert_eq!(envelope.cipher, CIPHER_AES_256_GCM);
        assert_eq!(envelope.key_version, 1);
        assert_eq!(envelope.nonce, "AAAAAAAAAAAAAAAA");
        assert_eq!(
            open_folder_object(&key, &aad, &envelope).unwrap(),
            plaintext
        );
        assert_eq!(
            EncryptedFolderObjectEnvelope::from_json(&envelope.canonical_json()).unwrap(),
            envelope
        );
    }

    #[test]
    fn rejects_wrong_folder_object_aad() {
        let key = FolderKey::from_bytes([7; 32]);
        let aad = folder_object_aad(1);
        let envelope = encrypt_folder_object_with_nonce(&key, &aad, [1; 12], b"hello").unwrap();
        let wrong_aad = FolderObjectAad {
            object_id: ObjectId::new("obj_aaaaaaaaaaaaaaaa").unwrap(),
            ..aad
        };

        assert_eq!(
            open_folder_object(&key, &wrong_aad, &envelope).unwrap_err(),
            CryptoRecordError::AadMismatch
        );
    }

    #[test]
    fn rejects_wrong_folder_key_version() {
        let key = FolderKey::from_bytes([7; 32]);
        let aad = folder_object_aad(1);
        let envelope = encrypt_folder_object_with_nonce(&key, &aad, [2; 12], b"hello").unwrap();
        let wrong_version = folder_object_aad(2);

        assert_eq!(
            open_folder_object(&key, &wrong_version, &envelope).unwrap_err(),
            CryptoRecordError::KeyVersionMismatch {
                expected: 2,
                actual: 1
            }
        );
    }

    #[test]
    fn validates_signed_create_update_and_move_revisions() {
        let keys = Keys::generate();
        let author_npub = npub(&keys);
        let envelope_json = sample_envelope_json();

        for (operation, revision, base_revision) in [
            (FolderObjectOperation::Create, 1, None),
            (FolderObjectOperation::Update, 2, Some(1)),
            (FolderObjectOperation::Move, 3, Some(2)),
        ] {
            let expected = revision_validation(
                operation,
                revision,
                base_revision,
                author_npub.clone(),
                envelope_json.clone(),
            );
            let payload = FolderObjectRevisionPayload::new(&expected);
            let event = sign_app_event(&keys, payload.canonical_json(), revision_tags(&expected));

            assert_eq!(validate_revision_event(&event, &expected).unwrap(), payload);
        }
    }

    #[test]
    fn rejects_revision_ciphertext_hash_mismatch() {
        let keys = Keys::generate();
        let expected = revision_validation(
            FolderObjectOperation::Create,
            1,
            None,
            npub(&keys),
            sample_envelope_json(),
        );
        let payload = FolderObjectRevisionPayload {
            ciphertext_hash: sha256_hex("different envelope"),
            ..FolderObjectRevisionPayload::new(&expected)
        };
        let event = sign_app_event(&keys, payload.canonical_json(), revision_tags(&expected));

        assert!(matches!(
            validate_revision_event(&event, &expected).unwrap_err(),
            CryptoRecordError::CiphertextHashMismatch { .. }
        ));
    }

    #[test]
    fn rejects_malformed_revision_payloads_and_bad_operations() {
        let keys = Keys::generate();
        let expected = revision_validation(
            FolderObjectOperation::Create,
            1,
            None,
            npub(&keys),
            sample_envelope_json(),
        );
        let malformed = sign_app_event(&keys, "{}".to_owned(), revision_tags(&expected));
        assert!(matches!(
            validate_revision_event(&malformed, &expected).unwrap_err(),
            CryptoRecordError::EventMismatch { .. }
        ));

        let bad_operation = FolderObjectRevisionPayload {
            operation: "delete".to_owned(),
            ..FolderObjectRevisionPayload::new(&expected)
        };
        let event = sign_app_event(
            &keys,
            bad_operation.canonical_json(),
            revision_tags(&expected),
        );

        assert_eq!(
            validate_revision_event(&event, &expected).unwrap_err(),
            CryptoRecordError::BadOperation {
                operation: "delete".to_owned()
            }
        );
    }

    #[test]
    fn rejects_revision_signer_mismatch_and_invalid_envelopes() {
        let author_keys = Keys::generate();
        let signer_keys = Keys::generate();
        let expected = revision_validation(
            FolderObjectOperation::Create,
            1,
            None,
            npub(&author_keys),
            sample_envelope_json(),
        );
        let payload = FolderObjectRevisionPayload::new(&expected);
        let event = sign_app_event(
            &signer_keys,
            payload.canonical_json(),
            revision_tags(&expected),
        );

        assert!(matches!(
            validate_revision_event(&event, &expected).unwrap_err(),
            CryptoRecordError::SignerMismatch { .. }
        ));

        let invalid_envelope = RevisionValidation {
            envelope_json: r#"{"bad":true}"#.to_owned(),
            ..expected
        };
        assert!(matches!(
            validate_revision_event(&event, &invalid_envelope).unwrap_err(),
            CryptoRecordError::InvalidEnvelope { .. }
        ));

        let key_version_mismatch = RevisionValidation {
            key_version: 2,
            ..revision_validation(
                FolderObjectOperation::Create,
                1,
                None,
                npub(&author_keys),
                sample_envelope_json(),
            )
        };
        assert_eq!(
            validate_revision_event(&event, &key_version_mismatch).unwrap_err(),
            CryptoRecordError::KeyVersionMismatch {
                expected: 2,
                actual: 1
            }
        );
    }

    #[test]
    fn validates_signed_tombstone() {
        let keys = Keys::generate();
        let expected = TombstoneValidation {
            vault_id: VaultId::new("acme").unwrap(),
            folder_id: FolderId::new("strategy").unwrap(),
            object_id: ObjectId::new("obj_0123456789abcdef").unwrap(),
            revision: 4,
            base_revision: 3,
            author_npub: npub(&keys),
            deleted_at: "2026-06-23T00:01:00.000Z".to_owned(),
        };
        let payload = FolderObjectTombstonePayload::new(&expected);
        let event = sign_app_event(&keys, payload.canonical_json(), tombstone_tags(&expected));

        assert_eq!(
            validate_tombstone_event(&event, &expected).unwrap(),
            payload
        );
    }

    #[test]
    fn validates_signed_admin_access_change() {
        let admin_keys = Keys::generate();
        let target_keys = Keys::generate();
        let expected = AdminAccessChangeValidation {
            vault_id: VaultId::new("acme").unwrap(),
            change_id: "change_0123456789abcdef".to_owned(),
            action: AdminAccessAction::GrantFolderAccess,
            admin_npub: npub(&admin_keys),
            folder_id: Some(FolderId::new("strategy").unwrap()),
            target_npub: Some(npub(&target_keys)),
            key_version: Some(2),
            note: Some("initial restricted access".to_owned()),
            created_at: "2026-06-23T00:02:00.000Z".to_owned(),
        };
        let payload = AdminAccessChangePayload::new(&expected);
        let event = sign_app_event(
            &admin_keys,
            payload.canonical_json(),
            admin_access_change_tags(&expected).unwrap(),
        );

        assert_eq!(
            validate_admin_access_change_event(&event, &expected).unwrap(),
            payload
        );
    }

    fn folder_object_aad(key_version: u32) -> FolderObjectAad {
        FolderObjectAad {
            vault_id: VaultId::new("acme").unwrap(),
            folder_id: FolderId::new("strategy").unwrap(),
            object_id: ObjectId::new("obj_0123456789abcdef").unwrap(),
            key_version,
        }
    }

    fn sample_envelope_json() -> String {
        let key = FolderKey::from_bytes([9; 32]);
        let aad = folder_object_aad(1);
        encrypt_folder_object_with_nonce(&key, &aad, [3; 12], b"encrypted page")
            .unwrap()
            .canonical_json()
    }

    fn revision_validation(
        operation: FolderObjectOperation,
        revision: u64,
        base_revision: Option<u64>,
        author_npub: String,
        envelope_json: String,
    ) -> RevisionValidation {
        RevisionValidation {
            vault_id: VaultId::new("acme").unwrap(),
            folder_id: FolderId::new("strategy").unwrap(),
            object_id: ObjectId::new("obj_0123456789abcdef").unwrap(),
            operation,
            revision,
            base_revision,
            key_version: 1,
            envelope_json,
            author_npub,
            created_at: "2026-06-23T00:00:00.000Z".to_owned(),
        }
    }

    fn sign_app_event(keys: &Keys, content: String, tags: Vec<Vec<String>>) -> Event {
        let tags = tags
            .into_iter()
            .map(|tag| Tag::parse(tag).unwrap())
            .collect::<Vec<_>>();

        EventBuilder::new(Kind::ApplicationSpecificData, content)
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(1_780_000_000))
            .finalize(keys)
            .unwrap()
    }

    fn npub(keys: &Keys) -> String {
        NostrPublicKey::from_protocol(keys.public_key())
            .to_npub()
            .unwrap()
    }
}
