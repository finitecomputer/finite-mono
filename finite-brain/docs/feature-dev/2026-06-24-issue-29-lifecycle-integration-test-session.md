# Issue 29 Session: Lifecycle Integration Test

## Issue

- Issue: finitecomputer/finite-brain#29
- Fixed point before session: finite-brain `01bba95` (`Pin split finite-nostr primitives`)
- Worker session: current Codex thread
- Commit: this commit (`Split Rust modules and add lifecycle test`)
- Status: complete

## Inputs

- PRD issue: finitecomputer/finite-brain#23
- Relevant glossary terms: SQLite Store, Vault Invitation, Shared Folder Mount, Sync Bootstrap, Filtered Export
- Relevant ADRs:
  - `docs/adr/0002-use-sqlite-from-day-one.md`
  - `docs/adr/0005-keep-folder-access-binary.md`
  - `docs/adr/0006-use-source-backed-shared-folder-mounts.md`

## Implementation

- Public interface used: `BrainStore` with a real temp SQLite database.
- Behaviors covered:
  - Create a source org vault and destination org vault.
  - Invite a destination member into the destination org.
  - Mark a restricted source Folder as shareable.
  - Write a secure object revision and verify sync projection.
  - Invite and accept a shared folder mount.
  - Add a destination org member to the shared folder connection.
  - Reopen the SQLite store and verify persisted mount availability.
  - Verify filtered encrypted export includes payload while access is present.
  - Remove the delegated member with key rotation and re-encryption.
  - Verify the member sees a locked mount and opaque filtered export.
  - Revoke the shared folder connection and verify revoked projection.
- `tdd` used: yes, the new story test was added and run directly before the full suite.
- Commands run:
  - `cargo test -p finite-brain-store sqlite_full_lifecycle_invite_share_sync_revoke_and_filter_visibility -- --nocapture`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo build --workspace`

## Review

- Standards findings: pass; the test uses the public store facade and a persisted SQLite file.
- Spec findings: pass; issue #29 lifecycle acceptance path is covered.
- Worthy fixes applied: none.
- Findings ignored with reasons: none.

## Risks

- This is a store-level integration test, not a browser E2E test. Browser smoke remains a separate concern.
