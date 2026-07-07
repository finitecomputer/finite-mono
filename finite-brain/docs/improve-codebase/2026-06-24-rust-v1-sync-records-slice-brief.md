# Structural Slice Brief: Sync Record Storage

## Selected Candidate

- Candidate: Deepen Sync Record Storage
- Candidate report path: `/tmp/architecture-review-finite-brain-20260624-sync.html`
- Why this candidate: `finite-brain-store/src/lib.rs` was carrying sync append-log mechanics alongside membership, grants, sharing, mounts, schema, and tests. The sync behavior already has strong tests, making this a low-risk locality slice.
- Recommendation strength: Strong

## Design

- Module: store-local sync append-log/current-projection implementation.
- Interface: existing public `BrainStore` methods.
- Seam: private `sync_records` module inside `finite-brain-store`.
- Adapters: none; this is not a new public storage adapter.
- Leverage: duplicate event handling, baseRevision conflicts, pagination, retention, projection insert/update, and record row decoding now sit behind one local implementation module.
- Locality: future sync hardening can change one module without reopening membership, sharing, mount, or schema orchestration code.

## Scope

- In scope:
  - Add `crates/finite-brain-store/src/sync_records.rs`.
  - Move sync validation, append-log, projection, pagination, and row-decoding helpers out of `lib.rs`.
  - Keep public `BrainStore` behavior and method signatures stable.
- Out of scope:
  - Schema changes.
  - Behavior changes.
  - Sharing/mount lifecycle extraction.
  - New adapters or trait interfaces.
- Files likely to change:
  - `crates/finite-brain-store/src/lib.rs`
  - `crates/finite-brain-store/src/sync_records.rs`
- Behavior changes approved: none
- Parked follow-up candidates:
  - Deepen Store Sharing And Mounts.
  - Split Portable Readable Surfaces.

## Tests And Checks

- Interface-level tests: existing `BrainStore` sync, rotation, shared-folder, projection, and backup tests.
- Targeted checks:
  - `cargo test -p finite-brain-store sync_ -- --nocapture`
  - `cargo test -p finite-brain-store rotation -- --nocapture`
  - `cargo test -p finite-brain-store shared_folder_connection -- --nocapture`
  - `cargo test -p finite-brain-store backup -- --nocapture`
- Full relevant suite:
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo build`

## Implementation Contract

- Implement one bounded structural slice.
- Preserve behavior.
- Keep tests at the `BrainStore` public interface.
- Do not expand into sharing/mount or portability module candidates.
- Route feature, deployment, or context work to the right loop.

## Human Gates

- Interface or seam decision: resolved by keeping `BrainStore` as the public seam.
- Behavior change: none.
- ADR conflict: none.
- Ownership or risk concern: none.
- Review escalation: only if tests or review show behavior drift.
