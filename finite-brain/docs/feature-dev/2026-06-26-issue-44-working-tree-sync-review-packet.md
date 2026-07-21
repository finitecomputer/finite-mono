# Issue #44 Review Packet: Brain Working Tree Sync

## Issue

- Issue: `finitecomputer/finite-brain#44`
- Slice type: AFK
- Acceptance criteria:
  - Sync pulls metadata/export/bootstrap or records and persists encrypted sync evidence.
  - Sync opens real NIP-59 Folder Key Grants addressed to the local signer.
  - Accessible objects decrypt into the existing `materialize_brain_working_tree` projection.
  - Local markdown creates/updates/deletes inside readable folders are encrypted, signed, and submitted through secure routes.
  - Unsafe or unmappable changes are recorded as open conflicts.
  - Focused tests prove materialization, writeback submission shape, and conflict recording through the CLI public seam.
- Baseline: `df69b01521d9e126f430d926a7730f4f4c641d05`
- Current diff: `git diff df69b01521d9e126f430d926a7730f4f4c641d05...HEAD`

## Implementation Summary

`fbrain sync now` now performs the trusted Agent Runtime loop: fetch encrypted server state, open local grants, materialize readable folders as ordinary files, push local markdown create/update/delete edits as signed encrypted object changes, and record server write conflicts locally.

## Implementation Evidence

- `implement` session: current thread
- `tdd` used: yes, focused sync-engine and CLI public-seam tests.
- Red test, if applicable: not preserved as a separate commit.
- Green implementation, if applicable:
  - `cargo test -p finite-brain-cli`
  - `cargo test -p finite-brain-server secure_object_routes_create_update_delete_and_pull_sync -- --nocapture`
  - `cargo test --workspace`
  - live smoke against `http://127.0.0.1:4016`
- Refactor, if applicable: sync logic moved out of `http.rs` into `sync_engine.rs`.
- Commands run:
  - `cargo fmt --check`
  - `cargo check --workspace`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo build`
  - `git diff --check`

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- None. The implementation keeps server state encrypted, uses existing core/store validation, and preserves the AGENTS.md guidance for explicit sync and crypto-adjacent control flow.

SPEC_STATUS: pass
SPEC_FINDINGS:
- None. The implementation satisfies issue #44 and PRD #43 command-driven Brain Working Tree sync criteria. Resident file watching and OS daemon supervision are correctly left out of scope.
```
