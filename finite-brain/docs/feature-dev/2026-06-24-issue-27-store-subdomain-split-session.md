# Issue 27 Session: Store Subdomain Split

## Issue

- Issue: finitecomputer/finite-brain#27
- Fixed point before session: finite-brain `01bba95` (`Pin split finite-nostr primitives`)
- Worker session: current Codex thread
- Commit: this commit (`Split Rust modules and add lifecycle test`)
- Status: complete

## Inputs

- PRD issue: finitecomputer/finite-brain#23
- Relevant glossary terms: BrainStore, Brain, Folder Access, Folder Key Grant, Brain Invitation, Share Link, Shared Folder Mount, Current-State Projection
- Relevant ADRs:
  - `docs/adr/0002-use-sqlite-from-day-one.md`
  - `docs/adr/0005-keep-folder-access-binary.md`
  - `docs/adr/0006-use-source-backed-shared-folder-mounts.md`

## Implementation

- Public interface used: existing `BrainStore` facade.
- Behaviors covered:
  - Schema and migrations moved to `schema.rs`.
  - Brain loading and projection helpers moved to `loading.rs`.
  - Brain bootstrap/member/admin mutation moved to `brains.rs`.
  - Folder creation, grants, setup repair, and rotation moved to `folder_access.rs`.
  - Brain invitations and share links moved to `links.rs`.
  - Shared folder invitations, mounts, delegated members, and revocation moved to `shared_folders.rs`.
  - Sync records remained in `sync_records.rs`.
  - `lib.rs` keeps shared types, conversion helpers, validation helpers, sync facade methods, export facade methods, and tests.
- `tdd` used: refactor-only; existing store tests plus the new lifecycle test guard behavior.
- Commands run:
  - `cargo test -p finite-brain-store --no-run`
  - `cargo test -p finite-brain-store`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo build --workspace`

## Review

- Standards findings: pass; `BrainStore` remains the public transaction boundary.
- Spec findings: pass; issue #27 store subdomain split is covered.
- Worthy fixes applied:
  - Removed stale copied invitation methods from `lib.rs`.
  - Made internal loading/migration helpers `pub(crate)` so sibling modules can call them cleanly.
- Findings ignored with reasons: none.

## Risks

- Store tests still live in `lib.rs`; moving tests into domain test modules can happen later without changing behavior.
