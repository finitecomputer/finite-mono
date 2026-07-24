# Issue #44 Session: Brain Working Tree Sync

## Issue

- Issue: `finitecomputer/finite-brain#44`
- Fixed point before session: `df69b01521d9e126f430d926a7730f4f4c641d05`
- Worker session: current thread
- Commit: `8bf422f969aa689c7c0214d70f98df85f1eca7b7`
- Status: implemented, verified locally

## Inputs

- PRD issue: `finitecomputer/finite-brain#43`
- Slice issue: `finitecomputer/finite-brain#44`
- Relevant glossary terms: Brain Working Tree, Agent CLI, Agent Sync Daemon, Local Agent Signer, Blocked Sync State
- Relevant ADRs/specs: `CONTEXT.md`, `docs/specs/finitebrain-portability-spec.md`, `docs/adr/0002-use-sqlite-from-day-one.md`, `docs/adr/0003-keep-folder-object-crypto-in-finite-brain-core.md`
- Prototype answer, if any: none

## Implementation

- Public interface used: `fbrain sync now`, `fbrain open`, secure object routes, encrypted export, and sync bootstrap.
- Behaviors covered:
  - Sync fetches encrypted export and sync bootstrap, then persists evidence under `.finitebrain/encrypted-sync`.
  - Sync opens NIP-59 Folder Key Grants addressed to the local signer and records local Folder Keys in agent state.
  - Accessible objects are decrypted and materialized through `materialize_brain_working_tree`.
  - Empty readable folders remain materialized with agent/wiki convention files.
  - Local markdown creates, updates, and deletes are planned through `plan_working_tree_change_intents`, encrypted with Folder Keys, signed as Nostr `30078` records, and submitted through secure object routes.
  - Server `409` write conflicts become open `ConflictEntry` records instead of silent overwrites.
  - `brain create` now sends client-generated bootstrap Folder Key Grants so newly created personal brains open as readable for the creating signer.
- `tdd` used: focused sync-engine tests for scan/materialization/revision validation plus a public command test that drives `fbrain sync now --server ...` against a local fake server and verifies conflict recording.
- Commands run during implementation:
  - `cargo test -p finite-brain-cli`
  - `cargo test -p finite-brain-server finish_setup_route_repairs_empty_setup_incomplete_folder -- --nocapture`
  - `cargo test -p finite-brain-server secure_object_routes_create_update_delete_and_pull_sync -- --nocapture`
  - `cargo build -p finite-brain-app -p finite-brain-cli`
  - live smoke against `http://127.0.0.1:4016`
  - `cargo fmt --check && cargo check --workspace && cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo build && git diff --check`
- Full suite command: `cargo test --workspace`

## Review

- Review fixed point: `df69b01521d9e126f430d926a7730f4f4c641d05`
- Standards findings: none from direct two-axis review.
- Spec findings: none from direct two-axis review.
- Worthy fixes applied:
  - Added the public-seam conflict test after review found unit-only conflict coverage too weak.
  - Added route-level bootstrap grant set validation before server conversion; store validation still enforces the authoritative invariant transactionally.
  - Addressed local CodeRabbit findings for partial-success rematerialization, current-key readability, stale moved-file cleanup, and bootstrap grant validation.
- Findings ignored with reasons: none.

## Risks

- Cross-Folder moves and a resident file watcher remain outside this slice. The verified path is command-driven create/update/delete sync for markdown files inside readable materialized folders.
