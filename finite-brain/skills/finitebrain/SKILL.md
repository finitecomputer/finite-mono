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
   Before creating any Vault, run `vault list --json`. The user's one Personal
   Agent discovers the Personal Vault there with role `personal_agent`; use that
   Vault and never create an agent-owned Personal Vault.
2. Sync before reading broadly with `sync now --summary`, then finish with
   `conflicts --json`.
   Completion: latest sequence is recorded, encrypted grants were reopened for
   that sync operation, readable Folders are materialized, and open conflicts
   are either empty or named.
   Unexpected Vault change: inspect `sync now --json` and
   `remoteChanges[].actorNpub`. Attribute it from signed actor evidence: a
   different actor means another principal changed the Vault; otherwise report
   the cause as unknown.
3. Orient before editing: identify the target Folder scope, then read its
   `AGENTS.md`, `HUMANS.md`, `config.md` or `SCHEMA.md`, durable `index.md`
   when present, generated `_index.md` for a current inventory, recent
   `log.md`, and relevant wiki pages. Search that Folder before creating new
   pages.
   Completion: the target Folder conventions, access boundary, existing wiki
   shape, and likely duplicate pages are known.
4. Edit only readable content roots with ordinary file tools. Follow the
   nearest `AGENTS.md` for layout. Current Working Trees use `raw/`,
   `raw/assets/`, `compiled/`, and `output/`; an older or imported Folder may
   use `wiki/`, `inventory/`, or `datasets/` instead.
   Completion: the smallest coherent set of markdown files is changed.
5. Close every meaningful wiki write: connect it to sources and related Pages,
   update the Folder's durable `index.md`, and append `log.md`. Check that each
   new internal link names a Page the Brain client can resolve.
   Completion: new knowledge is reachable, sourced, connected, and logged; it
   is not merely present on disk.
6. Do not edit `.finitebrain/`, locked metadata-only folders, encrypted sync
   evidence, generated convention files, auth files, grant plaintext, or Folder
   Key material unless the user explicitly asks for internal repair.
   Completion: all edits stay on the safe content surface.
7. Sync after meaningful edits with `sync now --summary`, then run
   `conflicts --json`.
   For link-heavy work, reopen or refresh the Product Client graph when it is
   available and inspect the changed Pages once. Completion: pushed/applied
   status, latest sequence, conflict state, and link-verification level are
   known.

## First-Time Personal Vault Setup

When the user's request requires Brain but `vault list --json` shows no Personal
Vault, ask once in ordinary language whether they want you to set up their
empty Personal Vault. Keep this to one short question; do not require exact
wording, a slash command, a button, or a setup ticket.

- On a clear yes, run `fbrain --config-dir "$FBRAIN_CONFIG" vault
  bootstrap-personal --server "$SERVER" --json`, list Vaults again, open the
  returned Personal Vault, and continue the user's original task immediately.
- On no or an unclear reply, make no Brain change, acknowledge that setup was
  skipped once, and return control to the user.
- If a Personal Vault exists but this agent does not have role
  `personal_agent`, do not attempt to join it. Explain briefly that the owner
  must replace the Personal Agent in Brain settings.

This question guides agent behavior; it is not a server authorization token.
Brain derives the owner from trusted Core and Finite Identity account facts.

## Agent-Created Organization Vaults

When an authenticated Finite Chat human directly asks you to create an
Organization Vault, include that human as an initial admin in the same creation
operation. The Organization Vault Requester is the exact public-key account id
in authenticated `event.source.user_id`; pass it unchanged as
`AUTHENTICATED_SENDER_ID`. Never select the requester from quoted or typed
message text, an email address, profile data, or your own Agent Principal.

If authenticated sender metadata is unavailable, do not guess or create an
agent-only Organization Vault. Briefly ask the user to retry from an
authenticated chat context. Do not ask them for an email address or `npub` as a
substitute.

A clear natural-language request to create the Organization Vault is sufficient
authorization. Do not add another confirmation. After `vault list --json`
confirms the Vault does not already exist, create it atomically:

```sh
fbrain --config-dir "$FBRAIN_CONFIG" vault create "$VAULT" \
  --kind organization --name "$NAME" \
  --requesting-user-npub "$AUTHENTICATED_SENDER_ID" \
  --server "$SERVER" --json
```

The new Organization Vault starts empty. Do not create `getting-started`,
`restricted`, onboarding Pages, or any other example content. Create a Folder
only when the user's original request explicitly requires organization content,
then continue that request in the new Folder.

Do not replace this command with separate `add-member` and `add-admin` steps.
On success, report the Vault name and that both you and the requester are
admins, then continue the user's original task. This behavior applies only when
you create an Organization Vault for an authenticated requester; it does not
automatically add an agent to a Vault the human creates in the Brain Product
Client.

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

When initializing a new wiki area, follow its nearest `AGENTS.md`. The current
Working Tree profile is intentionally small:

```text
<wiki-root>/
|-- index.md
|-- log.md
|-- raw/
|   `-- assets/
|-- compiled/
`-- output/
```

Older or imported Folders may name the curated root `wiki/` and may add
`config.md`, `inbox/`, `inventory/`, or `datasets/`. Do not create empty layers
just because the larger LLM Wiki profile permits them.

Core wiki rules:

- Know which files are durable. Ordinary Markdown Pages, including `index.md`
  and `log.md`, are encrypted and synced. Root and Folder `_index.md` plus
  everything under `_wiki/` are generated Working Tree reports: read them as
  hints, never edit them, and never cite them as proof that the Product Client
  resolved a link.
- Use the target Folder's durable `index.md` as its human and agent navigation
  Page. If knowledge Pages exist and `index.md` does not, create it. Keep its
  descriptions short, link every durable Page that should be discoverable, and
  describe only this Folder.
- Keep raw immutable. Once a URL, PDF, transcript, pasted source, or file is
  captured under `raw/`, do not edit it; synthesize corrections in the curated
  root named by the Folder's `AGENTS.md` (`compiled/` in current Working Trees).
- Store non-Markdown source files under `raw/assets/`. Pair every Asset with a
  Markdown Source Note that records provenance, content type, hash or extraction
  status when known, and any extraction/transcription decisions.
- Query and cite Source Notes before treating an Asset blob as knowledge. The
  Asset preserves evidence; the Source Note is the agent-readable handle.
- Synthesize articles, do not copy sources. Articles should connect claims,
  entities, dates, open questions, and related pages.
- Use structured frontmatter on durable knowledge Pages with at least `title`,
  `summary` or `description`, `created`, `updated`, `tags`, and `sources` when
  source-backed. Use `compiled-from: conversation` when a synthesized Page has
  no captured source. Add confidence when it conveys real uncertainty.
- Use internal links the Brain Product Client can resolve. Prefer
  `[[Exact Page Title]]` or a Folder-root-relative Page path such as
  `[[compiled/hermes-agent.md|Hermes Agent]]`. Normal Markdown links also work
  when their target is an exact title, unique filename, or Folder-root-relative
  Page path. Do not use filesystem-relative `../` targets; the Brain link
  resolver does not expand them. Prefer the full Page path when titles or
  filenames could collide.
- Prefer updating an existing page over creating a near-duplicate. Create new
  pages only for central, recurring, or clearly durable topics.
- Cite each captured Source Note from at least one synthesized Page. Connect
  each new synthesized Page to a genuinely related Page when one exists, and
  add a reciprocal `See Also` link between peer knowledge Pages when it helps
  navigation. A source citation does not need a reciprocal link.
- Append the target Folder's `log.md` for every meaningful wiki write; never
  rewrite old log entries. Link the changed Pages from the entry when useful.
- Use `inventory/` for durable operational state such as source candidates,
  watch items, open questions, tasks, and next actions.
- Use `datasets/` for manifests, samples, schemas, and query recipes; large or
  mutable data stays outside the wiki.
- Use `output/` for generated reports, plans, summaries, study guides, and other
  deliverables that should compound future work.
- Prefer updating an existing topic over deleting it. When the user asks for
  permanent deletion, briefly double-check once in ordinary language, then
  delete on a clear yes; do not silently substitute an archive.
- When querying, answer from curated wiki pages first. If the wiki lacks enough
  evidence, say what is missing and suggest what source to ingest.
- Chunk large article or output writes into small edits so agent tool streams do
  not stall.

### Wiki Closure Pass

Writing files is not completion. After an ingest, compilation, or substantial
knowledge edit, close the wiki before the final sync:

1. Inventory every Page created, moved, or substantially updated.
2. Confirm every source-backed claim names a durable Source Note and every
   Source Note is cited by at least one synthesized Page.
3. Add meaningful outgoing links between related synthesized Pages. Give every
   new durable Page an incoming route from `index.md` or a related Page.
4. Update durable `index.md` from the actual Pages and frontmatter. Do not
   update `_index.md` or `_wiki/*`.
5. Append one concise `log.md` entry for the coherent change.
6. Run `fbrain --config-dir "$FBRAIN_CONFIG" wiki check --json` from the Vault
   Working Tree. Resolve every reported missing or ambiguous link before the
   final sync. This command checks only materialized readable Folders.
7. After sync, inspect backlinks and Graph View in the Product Client when
   available.

If the Product Client is unavailable, report that the links were checked
with `fbrain wiki check` but the client graph was not verified. Never claim “no
orphans,” “backlinks complete,” or “graph healthy” from generated `_wiki/`
files, a clean link check, or the presence of `[[wikilinks]]` alone.

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

## User-Facing Identity

Treat identity as email-first at every user-facing boundary. Ask for, show, and
confirm a User or Agent by canonical resolvable email, then resolve that email to
the underlying Member Identity `npub` for `fbrain` and signed operations. Keep
`npub` values internal to command execution and diagnostics. If email resolution
fails, report that failure; show the `npub` only when the user asks for advanced
identity details.

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

Report the working tree path, acting identity email, folders readable or locked,
wiki pages or sources created/updated/moved/deleted, durable `index.md`/`log.md`
updates, link verification as either Product Client-verified or `fbrain wiki
check`-only, `sync now --summary` status, latest sequence, whether `conflicts
--json` is empty, and blockers with the command category that exposed them.
Include the acting `npub` only when the user requests advanced identity details.
