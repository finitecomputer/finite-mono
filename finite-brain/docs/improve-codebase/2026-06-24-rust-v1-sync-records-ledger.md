# Improve Codebase Ledger: Rust v1 Sync Record Storage

## Run

- Run ID: `2026-06-24-rust-v1-sync-records`
- Loop: Improve Codebase
- Target repo: `finitecomputer/finite-brain`
- Base branch: `staging`
- Improvement branch: `feature/rust-portable-v1-core`
- Human owner: delegated to Codex for this round
- Started: 2026-06-24
- Current status: selected structural slice verified locally

## Improvement Frame

- Starting intent: run another long Improve Context and Improve Codebase round.
- Specific area of concern, if any: none named.
- Out of scope: product behavior changes, production deployment, broad cleanup campaign.
- Known commands: `cargo fmt`, `cargo test -p finite-brain-store sync_ -- --nocapture`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build`, `git diff --check`.
- Repo context read: `AGENTS.md`, `CONTEXT.md`, `README.md`, `docs/adr/`, `docs/specs/finitebrain-portability-spec.md`, prior improve-codebase ledgers.
- Relevant ADRs: ADR 0001, ADR 0002, ADR 0003.

## Candidate Report

- Report path: `/tmp/architecture-review-finite-brain-20260624-sync.html`
- Generated at: 2026-06-24
- Top recommendation: Deepen Sync Record Storage
- Candidates shown:
  - Deepen Sync Record Storage
  - Deepen Store Sharing And Mounts
  - Split Portable Readable Surfaces
- ADR conflicts surfaced: none.

## Selection

- Selected candidate: Deepen Sync Record Storage
- Selected by: Codex under explicit human full-control delegation.
- Selected at: 2026-06-24
- Reason: best locality/leverage ratio after protected-route extraction; behavior-preserving and directly covered by existing sync, rotation, projection, and backup tests.
- Candidates parked or discarded:
  - Store Sharing And Mounts: parked because it touches broader access-control lifecycle.
  - Portable Readable Surfaces: parked until OKF/working-tree code grows further.

## Design Decisions

- Module being deepened: store-local sync append-log and current-projection mechanics.
- Interface: existing public `BrainStore` methods remain the external seam.
- Seam: `crates/finite-brain-store/src/sync_records.rs`, called only from `BrainStore` internals.
- Adapters: none; this is an implementation-local module, not a new public adapter seam.
- Test surface: existing `BrainStore` sync, rotation, projection, and backup tests.
- Scope boundaries: move sync validation, duplicate event lookup, sequence allocation, conflict checks, append-log insertion, projection updates, pull pagination, and record row decoding behind the store-local module.
- Non-goals: sharing/mount extraction, schema changes, behavior changes, public store interface changes.
- CONTEXT.md updates: none.
- ADRs created or updated: none.

## Slice Brief

- Brief path or issue: `docs/improve-codebase/2026-06-24-rust-v1-sync-records-slice-brief.md`
- Fixed point: `6cd959e`
- Files likely to change:
  - `crates/finite-brain-store/src/lib.rs`
  - `crates/finite-brain-store/src/sync_records.rs`
  - `docs/improve-codebase/2026-06-24-rust-v1-sync-records-*.md`
- Behavior changes approved: none
- Human gates: none; slice is behavior-preserving.

## Implementation Ledger

| Step | Command or source | Result | Notes |
| --- | --- | --- | --- |
| Focused sync tests | `cargo test -p finite-brain-store sync_ -- --nocapture` | pass | 8 tests passed. |
| Rotation/mount backup tests | `cargo test -p finite-brain-store rotation -- --nocapture && cargo test -p finite-brain-store shared_folder_connection -- --nocapture && cargo test -p finite-brain-store backup -- --nocapture` | pass | Covered re-encryption and projection rebuild paths using moved helpers. |
| Full workspace tests | `cargo test` | pass | 81 tests passed plus doctests. |
| Lints | `cargo clippy --all-targets -- -D warnings` | pass | Warnings denied. |
| Build | `cargo build` | pass | Workspace builds. |
| Product Client seams | `node --check crates/finite-brain-server/src/product-client.js && node crates/finite-brain-server/src/product-client.test.js` | pass | JS syntax and deterministic seams pass. |
| Diff hygiene | `git diff --check` | pass | No whitespace errors. |

## Review Ledger

| Review axis | Fixed point | Findings | Result |
| --- | --- | --- | --- |
| Standards | `6cd959e` | none | pass |
| Spec | `6cd959e` | none | pass |

## PR And Follow-Up

- PR URL: `https://github.com/finitecomputer/finite-brain/pull/15`
- Commit SHA: `4614860`
- Checks: targeted store tests, full Rust tests, clippy, build, JS checks, and diff hygiene passed.
- Review notes: no standards/spec findings.
- Follow-up issues: none created; parked candidates remain in this ledger.
- Handoffs: none.

## Open Gates

- None.
