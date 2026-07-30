---
name: finitebrain
description: Operate personal or Organization Brain/wiki knowledge-base workflows in FiniteBrain through Brain Working Trees and the fbrain control plane. Use for search, sync, conflicts, Folder access, daemon operation, Brain setup, or collaboration administration. A repository .wiki/, ~/wiki/, or configured wiki hub uses llm-wiki-finite.
---

# FiniteBrain

Use `fbrain` as the control plane and the Brain Working Tree as the content
surface. The repeatable loop is: verify identity, open or enter the tree, sync,
edit wiki content and source assets in readable Folders with ordinary file
tools, sync, and prove conflicts are empty. Key-using operations reopen grants
into memory for that operation; the CLI has no durable unlock state.

## Quick Start

Use the built-in hosted defaults for the happy path. Do not invent server,
config, Brain ID, or Working Tree variables before trying the ordinary commands.
The signing identity is the current Finite Home's Local Identity Key.

```sh
fbrain doctor
fbrain auth status --json
fbrain brain list --json
open_result="$(fbrain open personal --json)"
brain_tree="$(printf '%s' "$open_result" | python3 -c 'import json,sys; print(json.load(sys.stdin)["nextCommandWorkingDirectory"])')"
cd "$brain_tree"
fbrain sync now --summary
fbrain conflicts --json
```

A Working Tree remembers the server it was opened against. Before reusing an
existing tree, inspect `status --json`; a tree that names
`brain.smoke.finite.computer` remains smoke-pinned even after this skill's
default changes. Do not treat the servers as replicas or silently move that
tree. Preserve it until its Brain is deliberately reconciled, then reopen the
intended Brain with an explicit production `--server`.

The configured server is authoritative. If `doctor` or sync cannot reach it,
stop in Blocked Sync State; never substitute production, smoke, or another
FiniteBrain server. In a hosted Agent Runtime, the image supplies durable
`FBRAIN_CONFIG_DIR` and `FBRAIN_WORKING_TREE_ROOT` values below `/data`.
Use `--server`, `--config-dir`, an explicit Brain ID, or an explicit path only
for development, recovery, ambiguity resolution, or another deliberate
override.

Read [fbrain-cli.md](references/fbrain-cli.md) when a command fails, when using
daemon/watch, access, brain, folder, admin, invite, collaborator, or mount commands, or
when working from the Rust repo where `cargo run -p finite-brain-cli --bin
fbrain -- <args>` may be the available entrypoint.

## Operating Loop

1. Verify runtime state with `doctor`, `auth status --json`, and `status --json`.
   Completion: acting identity, working tree path, server source, daemon state,
   sync state, and blockers are known.
   Before creating any Brain, run `brain list --json`. The user's one Personal
   Agent discovers the Personal Brain there with role `personal_agent`; use that
   Brain and never create an agent-owned Personal Brain.
2. Sync before reading broadly with `sync now --summary`, then finish with
   `conflicts --json`.
   Completion: latest sequence is recorded, encrypted grants were reopened for
   that sync operation, readable Folders are materialized, and open conflicts
   are either empty or named.
   Unexpected Brain change: inspect `sync now --json` and
   `remoteChanges[].actorNpub`. Attribute it from signed actor evidence: a
   different actor means another principal changed the Brain; otherwise report
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
   `raw/assets/`, `wiki/`, `inventory/`, `datasets/`, and `output/`. Preserve
   an older or imported Folder's existing noncanonical paths rather than moving
   durable Pages implicitly.
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

## Brain Creation

When the user asks to create or bootstrap a Brain, read
[brain-creation.md](references/brain-creation.md) before acting. Completion:
the Brain type, authenticated requester authority, duplicate check, initial
roles, returned Brain ID, and continuation of the user's original task all
follow that branch's contract.

## Missing Domain Skill

Before using an expected domain skill, inspect the skills that are actually installed;
do not assume a skill exists because the user or a previous Page named it. In a
managed Runtime, inspect both the Runtime catalog and
`~/.finite/managed-skills/finite/current`.

If the expected skill is missing:

1. Find authoritative primary documentation for the requested subject.
2. Capture Markdown Source Notes under `raw/`. Store non-Markdown evidence
   under `raw/assets/` and pair each Asset with a Source Note containing its
   provenance.
3. Write only sourced synthesis under `wiki/`, cite the captured Source Notes,
   update durable `index.md`, and append durable `log.md`.
4. Describe the result as authoritative only to the extent supported by those
   captured sources.

If no authoritative source can be found, stop and explain the blocker. Write
no Page, Source Note, Asset, inventory placeholder, or model-memory draft for
the requested content. Do not silently substitute model knowledge.

## LLM Wiki Rules

A Brain is not one wiki with folders. It is a namespace of many
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
|-- wiki/
|-- inventory/
|-- datasets/
`-- output/
```

Older or imported Folders may use a noncanonical curated root such as
`compiled/` or may add `config.md` and `inbox/`. Preserve existing durable
paths, but create new curated synthesized Pages under `wiki/` unless the
nearest `AGENTS.md` explicitly says otherwise.

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
  root named by the Folder's `AGENTS.md` (`wiki/` in current Working Trees).
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
  `[[wiki/hermes-agent.md|Hermes Agent]]`. Normal Markdown links also work
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
- Prefer updating an existing topic over deleting it. For a Page deletion,
  briefly double-check once in ordinary language, then delete on a clear yes;
  do not silently substitute an archive. For a Folder deletion, read
  [destructive-operations.md](references/destructive-operations.md) before
  confirmation or execution because the command deletes a complete subtree.
- When querying, use `fbrain search "<query>" --json` for ranked evidence
  across every readable Folder. Treat the strongest results as entry points:
  open their full Pages and follow internal links that bear on the question.
  When the answer depends on surrounding relationships, use exact file search
  for each central Page's title, filename, and Folder-root-relative path to find
  incoming links. Use repeatable `--folder` only when the user deliberately
  narrows the scope. Completion: the answer is grounded in opened Pages and
  includes the directly relevant linked context, or names the missing evidence
  and suggests what source to ingest.
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
6. Run `fbrain wiki check --json` from the Brain Working Tree. Resolve every
   reported missing or ambiguous link before the final sync. This command
   checks only materialized readable Folders.
7. After sync, inspect backlinks and Graph View in the Product Client when
   available.

If the Product Client is unavailable, report that the links were checked
with `fbrain wiki check` but the client graph was not verified. Never claim “no
orphans,” “backlinks complete,” or “graph healthy” from generated `_wiki/`
files, a clean link check, or the presence of `[[wikilinks]]` alone.

Access-aware wiki rules:

- Never maintain a root-level or Brain-wide log that records restricted Folder
  activity.
- Do not list private Folder titles, summaries, source hints, or activity in an
  index visible to users who cannot access that Folder.
- Filter by readable Folder access before querying, compiling, indexing, or
  answering. Locked metadata-only Folders are not source material.
- When a previously readable Folder becomes locked or disappears, stop using
  its local Pages and prior search results immediately. Run sync and status
  again and let the client finish access-loss cleanup. Never inspect, copy,
  rebuild, or recover that Folder from its disposable search index.
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
fbrain daemon status --json
fbrain daemon start
fbrain daemon logs --json
fbrain daemon tick --json
```

Use `daemon watch` only as a foreground process under a supervisor such as tmux,
systemd, or the agent runtime; do not leave an unmanaged watch running at the
end of a task.

Folder and Mount revocation commands prepare key rotation and re-encrypted live
Folder objects automatically. If preparation fails, stop on the reported
blocker; never create or edit a raw rotation body.

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

## Organization Brain Collaboration

For a normal request to share an Organization Brain with another managed
Agent, use the recipient's canonical Managed Agent Email and one convergent
operation:

```sh
fbrain collaborator ensure-admin \
  --target agent@example.finite.vip \
  --json
```

Do not resolve the email yourself and do not probe a public NIP-05 endpoint
with `curl`. `fbrain` performs native identity resolution once, prepares every
Folder grant whose current key this Finite Home can open, and returns a typed
receipt. Inspect `state` before reporting the result:

- `complete`: authoritative postcondition inspection proved the Admin Brain
  Role and a current Folder Key Grant for every Folder in this operation's
  snapshot. Report the ready Folder count. Do not promise automatic access to
  Folders created or rotated later.
- `partial`: useful role and grant progress was preserved, but collaboration
  is not complete. Name each safe Folder path and reason from `folders`; tell
  the user to retry the exact same command from a named current key holder's
  Finite Home when the receipt supplies a holder email. If it does not, ask
  another current Folder reader who can open the listed Folder to retry; never
  invent or expose a holder identity. Never describe Admin role alone as
  successful sharing.
- `indeterminate`: the mutation may have committed, but the client could not
  prove its postcondition. Do not claim success or clean failure. Retry the
  exact same idempotent command, then inspect the new typed receipt.

Reports may include the canonical Agent Email, Folder paths, readiness counts,
safe reason codes, and named holder emails supplied by the receipt. Never paste
raw response payloads, Member Identity keys, wrapped grant events, auth
material, Folder Keys, or grant plaintext.

Low-level `admin` commands are advanced primitives. `admin member add` and
`admin role grant admin` change Brain Role, while `admin folder-access grant`
grants one specific Folder version. Separately or together they do not prove
complete Organization Brain Collaboration; do not compose them for a normal
"share this Org Brain with Agent B" request.

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

## Recovery And Durability Claims

Brain sync, server export, a Provider Durable Volume, and a TEE are not each a
Recovery Set. Describe hosted Brain data as durable only after the same
five-part Recovery Set has restored onto an empty target and reopened Chat
history and attachments, the hosted identity, Brain knowledge, and a fresh
Agent turn. Disposable `.finitebrain/` search state is neither backup material
nor part of that set.

When the user asks about backup, restore, migration, disaster recovery, or a
durability claim, read [recovery.md](references/recovery.md) before answering or
acting. Completion: the five parts, empty-target proof, identity binding,
backup boundary, and rollback boundary are explicit; otherwise report recovery
as unproven.

## Final Report

Report the working tree path, acting identity email, folders readable or locked,
wiki pages or sources created/updated/moved/deleted, durable `index.md`/`log.md`
updates, link verification as either Product Client-verified or `fbrain wiki
check`-only, `sync now --summary` status, latest sequence, whether `conflicts
--json` is empty, and blockers with the command category that exposed them.
Include the acting `npub` only when the user requests advanced identity details.
