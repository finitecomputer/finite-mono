---
name: finitebrain
description: FiniteBrain personal or Organization Brain/wiki knowledge-base work through fbrain. Use for search, sync, conflicts, wiki writes, Folder access, sharing and invitations, members and who has access, Brain setup, or daemon operation. A repository .wiki/, ~/wiki/, or configured wiki hub uses llm-wiki-finite.
---

# FiniteBrain

`fbrain` is the control plane and the Brain Working Tree is the content
surface. Each readable Folder in the tree is its own access-scoped LLM wiki:
edit it with ordinary file tools, then sync. Commands that need keys reopen
Folder Key grants in memory for that one operation; there is no unlock step.

## Quick Start

The ordinary commands work with the built-in hosted defaults and need no
server, config, Brain ID or path variables. The signing identity is the
current Finite Home's Local Identity Key.

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

The configured server is authoritative. A hosted Agent Runtime supplies
durable `FBRAIN_CONFIG_DIR` and `FBRAIN_WORKING_TREE_ROOT` values below
`/data`. Use `--server`, `--config-dir`, an explicit Brain ID or an explicit
path only for development, recovery or resolving ambiguity.

A Working Tree remembers the server it was opened against. Check
`status --json` before reusing a tree. A tree that names
`brain.smoke.finite.computer` stays smoke-pinned; the servers are not
replicas. Keep that tree until its Brain is deliberately reconciled, then
reopen the intended Brain with an explicit production `--server`.

## Operating Loop

1. **Verify.** Run `doctor`, `auth status --json` and `status --json`.
   Before creating any Brain, run `brain list --json`: the user's one
   Personal Agent finds their Personal Brain there with role `personal_agent`
   and works in it; never create an agent-owned Personal Brain. Done when the
   acting identity, tree path, server and its source, daemon state, sync state
   and blockers are known.
2. **Sync.** Run `sync now --summary`, then `conflicts --json`. Done when the
   latest sequence is recorded, readable Folders are materialized, and
   conflicts are empty or named. To explain an unexpected Brain change, read
   `remoteChanges[].actorNpub` from `sync now --json` and attribute it from
   signed actor evidence only.
   A different actor means another principal changed the Brain; otherwise report
   the cause as unknown.
3. **Orient.** In the target Folder, read `AGENTS.md`, `HUMANS.md`,
   `config.md` or `SCHEMA.md`, durable `index.md`, generated `_index.md`,
   recent `log.md` and the relevant Pages, then search the Folder for the
   topic. Done when the Folder's conventions, access boundary, wiki shape and
   likely duplicate Pages are known.
4. **Edit** the smallest coherent set of Markdown files in readable Folders,
   following the nearest `AGENTS.md` and the LLM Wiki Rules below.
5. **Close** every meaningful wiki write with the Wiki Closure Pass.
6. **Sync** with `sync now --summary`, then `conflicts --json`. Done when
   pushed/applied status, latest sequence and conflict state are known.
7. **Report** with the Final Report.

## Branches

Read the matching reference before acting:

| When | Read |
| --- | --- |
| the user asks to create or bootstrap a Brain | [brain-creation.md](references/brain-creation.md) |
| the user asks to invite someone, find or accept an invitation, share an Org Brain with another Agent, or unlock a member's Folders | [sharing.md](references/sharing.md) |
| the user asks who has access, who a key belongs to, what the dashboard roster shows, or to remove or demote someone | [who-has-access.md](references/who-has-access.md) |
| the user asks to delete a Folder | [destructive-operations.md](references/destructive-operations.md) |
| the user asks about backup, restore, migration or whether Brain data is durable | [recovery.md](references/recovery.md) |
| you run brain, daemon, search-index, folder, admin or mount commands, run `fbrain` from the Rust repo, or a command fails | [fbrain-cli.md](references/fbrain-cli.md) |
| the installed CLI, a server response or a user-owned skill disagrees with this skill | [skill-freshness.md](references/skill-freshness.md) |

The installed CLI's `--help` and errors outrank this skill for syntax, and
current server responses outrank it for behavior.

## LLM Wiki Rules

A Brain is a namespace of Folder-scoped LLM wikis, not one wiki: treat each
readable Folder as an independent wiki root unless its local instructions say
otherwise. Folder Keys and Folder Access decide which wikis the acting
identity can read, and each
Folder's `index.md` and `log.md` describe only that Folder. Sources become
immutable `raw/` notes, synthesis becomes cross-linked Pages, and later work
builds on the curated wiki instead of re-deriving it.

Current Working Trees use this Folder layout:

```text
<folder>/
|-- index.md     durable navigation Page
|-- log.md       append-only change log
|-- raw/         immutable Source Notes
|-- wiki/        synthesized Pages
|-- inventory/   source candidates, watch items, open questions, tasks
|-- datasets/    manifests, samples, schemas, query recipes
`-- output/      reports, plans, summaries and other deliverables
```

Older or imported Folders may use `compiled/`, `config.md` or `inbox/`. Keep
their existing durable paths, and put new synthesized Pages under `wiki/`
unless the nearest `AGENTS.md` says otherwise. Large or mutable data stays
outside the wiki; `datasets/` describes it.

### Files

- Ordinary Markdown Pages, including `index.md` and `log.md`, are durable:
  encrypted and synced. `_index.md` files and everything under `_wiki/` are
  generated reports: read them as hints and never edit them. Cite and link
  durable Pages instead.
- Edit Folder content only. `.finitebrain/`, locked metadata-only Folders,
  encrypted sync evidence, generated convention files, auth files, grant
  plaintext and Folder Key material belong to `fbrain`; change them only when
  the user asks for internal repair.
- Write large Pages in several small edits so tool streams do not stall.

### Sourcing

- Capture each URL, PDF, transcript, pasted source or file once under `raw/`
  and leave it unchanged. Corrections belong in synthesized Pages.
- Keep non-Markdown bytes outside the Brain. Represent each Asset with one
  Source Note under `raw/` whose frontmatter has `type`, `title` and the
  canonical `resource` URI, plus `description` and any known `finite_asset`
  content type, size, hash or provider revision. Cite the Source Note before
  treating its resource as knowledge; the resource's availability still
  depends on where it lives.
- When a task expects a domain skill,
  inspect the skills that are actually installed: in a managed Runtime, the
  Runtime catalog and `/data/agent/managed-skills/finite/current`. A skill
  named by the user or a Page may not exist. If it is missing, find
  authoritative primary documentation for the subject and capture it under
  `raw/`. Write only sourced synthesis under `wiki/` for that subject, cite the
  captured Source Notes, and call the result authoritative only as far as
  they support. If no authoritative source exists, stop and explain the
  blocker. Write no Page, Source Note, Asset, inventory placeholder, or
  model-memory draft for the requested content.
  Do not silently substitute model knowledge.

### Pages

- Update an existing Page before creating a near-duplicate. Create Pages only
  for central, recurring or clearly durable topics.
- Synthesize rather than copy: connect claims, entities, dates, open questions
  and related Pages.
- Give durable knowledge Pages frontmatter with at least `title`, `summary` or
  `description`, `created`, `updated`, `tags`, and `sources` when
  source-backed. Use `compiled-from: conversation` for synthesis with no
  captured source. Add `confidence` when it conveys real uncertainty.
- Write internal links the Brain Product Client resolves: `[[Exact Page Title]]`
  or a Folder-root-relative path such as `[[wiki/hermes-agent.md|Hermes Agent]]`.
  Markdown links work when the target is an exact title, unique filename or
  Folder-root-relative path. Use the full path when titles or filenames could
  collide; the resolver does not expand `../`.
- Keep each Folder's durable `index.md` as its navigation Page: create it when
  knowledge Pages exist without one, link every Page that should be
  discoverable with a short description, and describe only that Folder.
- Append one `log.md` entry per meaningful write, linking the changed Pages
  when useful. Earlier entries stay as written.
- Prefer updating a topic to deleting it. Before deleting a Page,
  double-check once in ordinary language, then delete it on a clear yes rather
  than archiving it. Folder deletion removes a whole
  subtree: read [destructive-operations.md](references/destructive-operations.md).

### Querying

Run `fbrain search "<query>" --json` for ranked evidence across every readable
Folder. Treat the strongest results as entry points: open their full Pages and
follow the internal links that bear on the question. When the answer depends
on relationships, exact-search each central Page's title, filename and
Folder-root-relative path to find incoming links. Narrow with repeatable
`--folder` only when the user does. Done when the answer rests on opened Pages
and their directly relevant linked context, or names the missing evidence and
a source to ingest.

### Wiki Closure Pass

Writing files is not completion. Close every meaningful write before the
final sync:

1. Inventory every Page created, moved or substantially updated.
2. Confirm every source-backed claim names a durable Source Note, and every
   Source Note is cited by at least one synthesized Page.
3. Link related synthesized Pages to each other, adding a reciprocal
   `See Also` between peers when it helps navigation; a source citation needs
   no reciprocal link. Give every new durable Page an incoming route from
   `index.md` or a related Page.
4. Update durable `index.md` from the actual Pages and frontmatter, never
   `_index.md` or `_wiki/*`.
5. Append one concise `log.md` entry for the coherent change.
6. Run `fbrain wiki check --json` from the Working Tree and resolve every
   missing or ambiguous link. It checks only materialized readable Folders.
7. After sync, inspect backlinks and Graph View in the Product Client when it
   is available.

Done when every new Page is reachable, sourced, connected and logged, and link
verification is named as Product Client-verified or `wiki check`-only. Claims
such as "no orphans", "backlinks complete" or "graph healthy" need the Product
Client; generated `_wiki/` files, a clean link check or the presence of
`[[wikilinks]]` do not show them.

### Access Boundaries

Knowledge flows only toward an audience at least as restricted as its source.

- Query, compile, index and answer from readable Folders only. Locked
  metadata-only Folders are not source material.
- Keep restricted Folder activity in that Folder's own `log.md`; a root-level
  or Brain-wide log never records it.
- An index lists only what its readers may see: no private Folder titles,
  summaries, source hints or activity.
- Never synthesize content from a more-restricted Folder into a
  less-restricted Folder, index, log, output or public summary. Put a
  cross-Folder output in the most restrictive appropriate Folder for every
  source used; with no safe common audience, split it by Folder.
- When a readable Folder becomes locked or disappears, stop using its local
  Pages and earlier search results at once. Run sync and status again and let
  the client finish access-loss cleanup; never inspect, copy, rebuild or
  recover that Folder from its disposable search index.
- Directories inside a Folder are layout only and create no access boundary.
- Folder names and server-visible Folder IDs are metadata. Keep sensitive
  project, client, people or deal names inside encrypted Pages when the
  audience is narrow.

## Blocked Sync State

When sync, access or the daemon blocks, stop broad edits and inspect
`status --json`, `sync status --json`, `conflicts --json` and
`daemon status --json`. Report the block against the configured server; never
substitute production, smoke or another FiniteBrain server.

For a missing, stale or repeatedly failing daemon:

```sh
fbrain daemon status --json
fbrain daemon start
fbrain daemon logs --json
fbrain daemon tick --json
```

Run `daemon watch` only in the foreground under a supervisor such as tmux,
systemd or the agent runtime; leave no unsupervised watch running when the
task ends.

## Identity And Security

- Name people the way the user knows them: the resolved name or email from
  `fbrain access list`, or a NIP-05 name. Use `npub` values for commands and
  diagnostics, and show one when the user asks for identity details or when a
  key has no resolved name, since the exact key is then its only identity.
- `fbrain` turns an email into a key only through public NIP-05; it never
  looks up a Finite account or login email. When resolution fails, report the
  actual reason: a transport or document error means the key could not be
  verified, not that none exists. For a confirmed
  missing record, explain the email capability invitation in
  [sharing.md](references/sharing.md) and get the user's choice before
  changing the intended identity binding.
- The runtime or a human runbook provisions identity in the current Finite
  Home (`$FINITE_HOME/identity/identity.json`, else
  `~/.finite/identity/identity.json`), shared by the Finite tools in that home.
  Hosted users and agents have distinct Member Identities; an Agent Runtime's
  Finite Home holds only its Agent Principal. Act as that provisioned
  identity. Run `fbrain auth import`, or create, replace or request a keypair,
  only when the user or a runbook asks.
- Never print or expose private Nostr secrets, Folder Keys, grant plaintext,
  decrypted sync payload internals, local auth files or rotation bodies. Use
  `--json` for inspection and summarize sensitive results instead of pasting
  raw payloads.
- Brain sync, a server export, a Provider Durable Volume and a TEE are not
  backups. Call hosted Brain data durable only with the restore proof in
  [recovery.md](references/recovery.md); otherwise call recovery unproven.

## Final Report

Report:

- the Working Tree path and the acting identity's email (its `npub` only when
  asked);
- Folders readable or locked;
- Pages and Source Notes created, updated, moved or deleted, and the
  `index.md` and `log.md` updates;
- link verification: Product Client-verified, or `fbrain wiki check` only;
- the `sync now --summary` status, latest sequence, and whether
  `conflicts --json` is empty;
- blockers, with the command that exposed each.
