---
name: finitebrain
description: Personal Brain/wiki knowledge-base operations in FiniteBrain through ordinary file edits plus the fbrain CLI control plane. Use for FiniteBrain product knowledge requests; setting up or acting as an agent participant; opening, syncing, or editing Vault Working Trees; maintaining content inside readable Folders; inspecting sync/conflict state; checking Folder access; using fbrain daemon/watch; or performing Vault, Folder, permission, invitation, and share-link admin flows. A repository .wiki/, ~/wiki/, or configured wiki hub uses llm-wiki-finite.
---

# FiniteBrain

Use `fbrain` as the control plane and the Vault Working Tree as the content
surface. The repeatable loop is: verify identity, open or enter the tree, sync,
edit wiki content and source assets in readable Folders with ordinary file
tools, sync, and prove conflicts are empty. Key-using operations reopen grants
into memory for that operation; the CLI has no durable unlock state.

## Quick Start

Prefer explicit `--config-dir` in agent runtimes. The CLI default is
`$FBRAIN_CONFIG_DIR`, then `$HOME/.finitebrain/fbrain`, but explicit state avoids
surprises when shell environment resets between calls. The signing identity is
not stored there: it is the current Finite Home's Local Identity Key, resolved from
`$FINITE_HOME/identity/identity.json` (else `~/.finite/identity/identity.json`)
regardless of `--config-dir`.

```sh
SERVER="${FINITE_BRAIN_SERVER_URL:?FiniteBrain server is not configured}"
FBRAIN_CONFIG="${FBRAIN_CONFIG_DIR:-${FINITE_HOME:-$HOME/.finite}/fbrain}"
TREE_ROOT="${FBRAIN_WORKING_TREE_ROOT:-${FINITECHAT_WORKSPACE:-$HOME/finitebrain}}"
VAULT="replace-with-vault-id"
TREE="$TREE_ROOT/$VAULT"

fbrain --config-dir "$FBRAIN_CONFIG" doctor --server "$SERVER"
fbrain --config-dir "$FBRAIN_CONFIG" auth status --json
fbrain --config-dir "$FBRAIN_CONFIG" vault list --server "$SERVER" --json
fbrain --config-dir "$FBRAIN_CONFIG" open "$VAULT" "$TREE" --server "$SERVER"
cd "$TREE"
fbrain --config-dir "$FBRAIN_CONFIG" sync now --summary
fbrain --config-dir "$FBRAIN_CONFIG" conflicts --json
```

A Working Tree remembers the server it was opened against. Before reusing an
existing tree, inspect `status --json`; a tree that names
`brain.smoke.finite.computer` remains smoke-pinned even after this skill's
default changes. Do not treat the servers as replicas or silently move that
tree. Preserve it until its Vault is deliberately reconciled, then reopen the
intended Vault with an explicit production `--server`.

The configured server is authoritative. If `doctor` or sync cannot reach it,
stop in Blocked Sync State; never substitute production, smoke, or another
FiniteBrain server. In a hosted Agent Runtime, the image supplies durable
`FBRAIN_CONFIG_DIR` and `FBRAIN_WORKING_TREE_ROOT` values below `/data`.

Read [fbrain-cli.md](references/fbrain-cli.md) when a command fails, when using
daemon/watch, access, vault, folder, permission, invite, or share commands, or
when working from the Rust repo where `cargo run -p finite-brain-cli --bin
fbrain -- <args>` may be the available entrypoint.

## Operating Loop

1. Verify runtime state with `doctor`, `auth status --json`, and `status --json`.
   Completion: acting identity, working tree path, server source, daemon state,
   sync state, and blockers are known.
   Before creating any Vault, run `vault list --json`. An Agent Principal paired
   by a user discovers that user's Personal Vault there with role `member`; use
   that Vault and never create a second agent-owned Personal Vault.
2. Sync before reading broadly with `sync now --summary`, then finish with
   `conflicts --json`.
   Completion: latest sequence is recorded, encrypted grants were reopened for
   that sync operation, readable Folders are materialized, and open conflicts
   are either empty or named.
3. Orient before editing: identify the target Folder scope, then read its
   `AGENTS.md`, `HUMANS.md`, `config.md` or `SCHEMA.md`, `_index.md` or
   `index.md`, recent `log.md`, and relevant wiki pages. Search that Folder
   before creating new pages.
   Completion: the target Folder conventions, access boundary, existing wiki
   shape, and likely duplicate pages are known.
4. Edit only readable content roots with ordinary file tools. Keep LLM wiki work
   under Folder-local conventions such as `raw/`, `raw/assets/`, `wiki/`,
   `inventory/`, `datasets/`, and `output/`.
   Completion: the smallest coherent set of markdown files is changed.
5. Do not edit `.finitebrain/`, locked metadata-only folders, encrypted sync
   evidence, generated convention files, auth files, grant plaintext, or Folder
   Key material unless the user explicitly asks for internal repair.
   Completion: all edits stay on the safe content surface.
6. Sync after meaningful edits with `sync now --summary`, then run
   `conflicts --json`.
   Completion: pushed/applied status, latest sequence, and conflict state are
   known.

## LLM Wiki Rules

A FiniteBrain Vault is not one wiki with folders. It is a namespace of many
Folder-scoped LLM wikis. Treat each readable FiniteBrain Folder as an
independent access-scoped LLM wiki root unless its local instructions say
otherwise. The wiki is Markdown-first: Markdown sources become immutable `raw/`
notes, non-Markdown source files become Assets under `raw/assets/`, synthesized
knowledge becomes cross-linked articles, and outputs build on the curated wiki
instead of re-deriving context from scratch.

The LLM Wiki topic model maps to a FiniteBrain Folder. Folder Keys and Folder
Access define which topic wikis the active user or agent can read. Indexes and
logs live at the same Folder scope as the knowledge they describe.

When initializing a new wiki area, prefer this shape unless an existing Folder
uses a different convention:

```text
<wiki-root>/
|-- config.md
|-- _index.md
|-- log.md
|-- inbox/
|-- raw/
|   `-- assets/
|-- wiki/
|   |-- concepts/
|   |-- topics/
|   `-- references/
|-- inventory/
|-- datasets/
`-- output/
```

Core wiki rules:

- Read the target Folder's local index first. `_index.md` and `index.md` are
  navigation caches for that Folder only; stale check them before trusting them,
  then update them after meaningful writes.
- Keep raw immutable. Once a URL, PDF, transcript, pasted source, or file is
  captured under `raw/`, do not edit it; synthesize corrections in `wiki/`.
- Store non-Markdown source files under `raw/assets/`. Pair every Asset with a
  Markdown Source Note that records provenance, content type, hash or extraction
  status when known, and any extraction/transcription decisions.
- Query and cite Source Notes before treating an Asset blob as knowledge. The
  Asset preserves evidence; the Source Note is the agent-readable handle.
- Synthesize articles, do not copy sources. Articles should connect claims,
  entities, dates, open questions, and related pages.
- Use structured frontmatter on wiki pages with at least title, summary or
  description, dates, tags, sources, and confidence when useful.
- Use `[[wikilinks]]` for Obsidian and add normal markdown links when agent path
  navigation would benefit from them.
- Prefer updating an existing page over creating a near-duplicate. Create new
  pages only for central, recurring, or clearly durable topics.
- Append the target Folder's `log.md` for every meaningful wiki write; never
  rewrite old log entries.
- Use `inventory/` for durable operational state such as source candidates,
  watch items, open questions, tasks, and next actions.
- Use `datasets/` for manifests, samples, schemas, and query recipes; large or
  mutable data stays outside the wiki.
- Use `output/` for generated reports, plans, summaries, study guides, and other
  deliverables that should compound future work.
- Archive quietly instead of deleting superseded topics. Prefer `.archive/` or
  the local Folder convention, update indexes, and log the archive.
- When querying, answer from curated wiki pages first. If the wiki lacks enough
  evidence, say what is missing and suggest what source to ingest.
- Chunk large article or output writes into small edits so agent tool streams do
  not stall.

Access-aware wiki rules:

- Never maintain a root-level or Vault-wide log that records restricted Folder
  activity.
- Do not list private Folder titles, summaries, source hints, or activity in an
  index visible to users who cannot access that Folder.
- Filter by readable Folder access before querying, compiling, indexing, or
  answering. Locked metadata-only Folders are not source material.
- Never synthesize content from a more-restricted Folder into a less-restricted
  Folder, index, log, output, or public summary.
- Put cross-Folder outputs in the most restrictive appropriate Folder for every
  source used. If there is no safe common audience, split the output by Folder.
- Treat local directories as layout inside one Folder. They do not create new
  access boundaries.
- Treat Folder names and server-visible Folder ids as metadata. Keep sensitive
  project, client, people, or deal names inside encrypted pages when the
  audience is narrow.

## Blocked State

If sync, access, or daemon work blocks, stop broad edits and inspect with
`status --json`, `sync status --json`, `conflicts --json`, `daemon status --json`,
and the relevant command in [fbrain-cli.md](references/fbrain-cli.md).

If daemon state is missing, stale, or repeatedly failing, use:

```sh
fbrain --config-dir "$FBRAIN_CONFIG" daemon status --json
fbrain --config-dir "$FBRAIN_CONFIG" daemon start
fbrain --config-dir "$FBRAIN_CONFIG" daemon logs --json
fbrain --config-dir "$FBRAIN_CONFIG" daemon tick --json
```

Use `daemon watch` only as a foreground process under a supervisor such as tmux,
systemd, or the agent runtime; do not leave an unmanaged watch running at the
end of a task.

Treat `access revoke` without `--rotation-body` as a safety checklist, not a
failed command: Folder access removal requires key rotation and re-encrypted live
Folder objects.

## Managed Skill Verification

In Finite runtimes, `~/.finite/managed-skills/finite/current` is a symlink to
the checked-out `finite-skills` repo. Use symlink-aware searches when verifying
the installed skill:

```sh
find -L ~/.finite/managed-skills/finite/current -path '*finitebrain/SKILL.md' -print
```

If a runtime lists `finitebrain-agent`, treat it as a stale skill name. The
current skill name is `finitebrain`.

## Security Rules

- Never print or expose private Nostr secrets, Folder Keys, grant plaintext,
  decrypted sync payload internals, local auth files, or rotation bodies.
- Assume identity is provisioned by the runtime or a human runbook via the
  current Finite Home identity file (`$FINITE_HOME/identity/identity.json`, else
  `~/.finite/identity/identity.json`), which Finite tools in that home share.
  Hosted users and agents have distinct provisioned Member Identities; an
  Agent Runtime's Finite Home contains only that Agent Principal. Follow the
  identity already provisioned for the current Finite Home. Do not run `fbrain auth import`,
  create, replace, or ask for keypairs unless the user or runbook explicitly
  asks.
- Use `--json` for machine inspection, but summarize sensitive results instead
  of pasting raw payloads.

## Final Report

Report the working tree path, safe acting npub when relevant, folders readable or
locked, wiki pages or sources created/updated/moved/deleted, index/log updates,
`sync now --summary` status, latest sequence, whether `conflicts --json` is
empty, and blockers with the command category that exposed them.
