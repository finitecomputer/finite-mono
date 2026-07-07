# Issue 26 Session: Server Route Split

## Issue

- Issue: finitecomputer/finite-brain#26
- Fixed point before session: finite-brain `01bba95` (`Pin split finite-nostr primitives`)
- Worker session: current Codex thread
- Commit: this commit (`Split Rust modules and add lifecycle test`)
- Status: complete

## Inputs

- PRD issue: finitecomputer/finite-brain#23
- Relevant glossary terms: Product Client, Secure Encrypted Object Routes, Vault, Folder, Share Link, Shared Folder Mount
- Relevant ADRs:
  - `docs/adr/0001-adopt-rust-workspace-and-finite-nostr.md`
  - `docs/adr/0002-use-sqlite-from-day-one.md`

## Implementation

- Public interface used: existing `finite-brain-server` routes and `ServerState`.
- Behaviors covered:
  - Static public routes moved to `routes/public.rs`.
  - Vault bootstrap, creation, metadata, and invitation routes moved to `routes/vaults.rs`.
  - Folder creation, setup repair, and access rotation routes moved to `routes/folders.rs`.
  - Share link and shared folder connection routes moved to `routes/sharing.rs`.
  - Secure object and sync routes moved to `routes/objects_sync.rs`.
  - `lib.rs` keeps the server facade, shared auth/rate-limit helpers, test fixtures, and route wiring.
- `tdd` used: refactor-only; existing server and workspace tests guard behavior.
- Commands run:
  - `cargo test -p finite-brain-server --no-run`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo build --workspace`

## Review

- Standards findings: pass; route behavior is grouped by product domain without widening public API.
- Spec findings: pass; issue #26 route-domain split is covered.
- Worthy fixes applied:
  - Corrected static asset include paths after extraction.
- Findings ignored with reasons: none.

## Risks

- Server unit tests still live in `lib.rs`; moving tests can be a later cleanup, but production route code is now split by domain.
