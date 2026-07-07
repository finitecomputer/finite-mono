# FiniteBrain Context

## Glossary

### FiniteBrain Portable v1

The hard-cut implementation target for the Rust rebuild. It is defined by
`docs/specs/finitebrain-portability-spec.md` and covers Vaults, Folders, Folder
Objects, Folder Key Grants, sync, sharing, OKF import/export, and compatibility.

### FiniteBrain Policy

Application-specific behavior for Vaults, Folders, access, sync, storage,
sharing, OKF, hardening rules, the Product Client, and the Smoke UI.
FiniteBrain Policy belongs in the `finite-brain` workspace, not in
`finite-nostr`.

### Reusable Nostr Primitive

A generic Nostr operation that can be reused across Finite repos without
knowing about FiniteBrain Vaults or Folders. Examples include NIP-19 identity
encoding, event serialization and verification, NIP-44 encryption adapters,
NIP-59 gift-wrap helpers, and NIP-98-style HTTP authorization helpers.

### Smoke UI

A development-only HTML/CSS interface served by the Rust app for local
end-to-end verification. It is not the product client. It exists to inspect
Vaults, Folders, encrypted objects, sync state, grants, invitations, shares,
and mounts while the Rust core and server mature.

### Product Client

The trusted browser experience a User actually uses to open a Vault, connect a
NIP-07 signer, open Folder Key Grants, decrypt accessible Folder Objects,
materialize Pages, edit content, sync changes, run local search/graph indexes,
and perform OKF import/export. Unlike the Smoke UI, the Product Client owns the
normal user workflow.

### Product Client Spine

The minimum trusted-client workflow that later client features build on:
connect the User's NIP-07 signer, load Vault state, open current Folder Key
Grants, decrypt readable Pages, edit one Page, encrypt and write the Page back
as a signed revision, and pull/apply sync records without losing unresolved
local edits.

### Folder-scoped LLM Wiki

The FiniteBrain knowledge model. A Vault is a namespace of many LLM wikis, and
each Folder is the enforceable wiki scope because Folder Keys and Folder Access
define who can read it. Folder-local `_index.md`, `config.md`, and `log.md`
describe only that Folder. Root/global indexes must not leak private Folder
titles, summaries, sources, or activity.

### Asset

An encrypted non-Markdown source file stored inside a Folder, such as a PDF,
image, audio file, or other blob. An Asset is evidence or source material; it
is not the primary LLM Wiki knowledge surface.

### Source Note

A Markdown Page that describes one captured source with provenance, extraction
status, and human or agent-readable notes. Source Notes are the readable handles
that LLM Wiki pages cite when synthesizing knowledge from raw material.

### Asset Source Note Pair

The expected pairing for non-Markdown source material: one Asset under
`raw/assets/` plus one Source Note that explains and cites that Asset. The
Asset preserves the original evidence, while the Source Note lets humans,
agents, search, and graph flows reason over it.

### Graph View

A Product Client view over the active User's decrypted accessible Pages. It
renders Page nodes and Page relationships only after Folder Keys are open and
visibility filtering has been applied.

### Graph Replay

A Product Client playback of graph/index changes derived from the client's
applied sync history and decrypted Page index. It is not a server-side graph
event log.

### OKF Import Execution

A Product Client workflow that parses readable OKF, plans import conflicts,
opens destination Folder Keys, encrypts imported Pages client-side, signs
Folder Object revisions, and uploads those revisions through normal secure
object routes. The Rust server does not parse readable OKF or receive
plaintext Page content during import.

### Vault Working Tree

A local agent-facing file projection built from already-decrypted accessible
Pages. It materializes readable Folders as Folder-scoped LLM wiki roots with
local `AGENTS.md` or `HUMANS.md` when present, `_index.md`, `config.md`,
`log.md`, `raw/`, `wiki/`, `inventory/`, `datasets/`, and `output/`
conventions. It stores only safe locked metadata for inaccessible Folders, and
maps file changes back into Product Client encrypted-object write, move, and
delete intents.

### Agent CLI

The terminal control surface for a trusted Agent Runtime working inside a Vault
Working Tree. It explains and controls identity, local daemon state, Folder Key
opening, automatic sync health, blocked edits, activity, and access reasons
while the agent reads and writes ordinary files.

### Agent Sync Daemon

The resident trusted-client process that watches a Vault Working Tree, opens
available Folder Keys for the acting User, detects file changes, syncs with the
server, and records blocked states that require agent or human resolution.

### Local Agent Signer

A trusted signer available to the Agent Runtime when browser NIP-07 is not
available. It exposes the same conceptual abilities the Product Client needs:
identify the acting npub, sign FiniteBrain events, and perform NIP-44
encryption and decryption for Folder Key Grant handling.

### Blocked Sync State

A local condition where automatic sync cannot safely complete without
resolution. Examples include missing auth, missing Folder Key Grant, locked
Folder, stale base revision conflict, revoked access, unavailable server, or a
working-tree change that cannot be mapped to a secure object intent.

### Hard Cut

A compatibility boundary where FiniteBrain does not carry legacy route,
storage, client, or migration behavior forward. Hard-cut work may import data
through explicit new-format flows such as OKF, but it does not preserve old v1
runtime compatibility as a feature requirement.
