# ADR 0002: Use SQLite From Day One

Status: accepted

Date: 2026-06-23

## Context

FiniteBrain Portable v1 depends on durable ordered state:

- Brain metadata.
- Folder hierarchy and access state.
- Folder Key Grant metadata.
- Signed encrypted object revisions and tombstones.
- Monotonic sync sequences.
- Current encrypted object projection.
- Invitations, Share Links, Shared Folder Connections, and Mounts.
- Retention and `rebootstrap_required` behavior.

These behaviors are transaction and recovery problems, not just in-memory
domain-model problems. The Finite engineering style also requires
authoritative server state to use schema, constraints, and transactions.

## Decision

The Rust implementation will use SQLite from day one for authoritative server
state.

`finite-brain-core` stays pure and testable without SQLite. `finite-brain-store`
owns SQLite schema, migrations, transaction boundaries, idempotency,
current-state projections, and restart tests.

`finite-brain-store` will use synchronous `rusqlite` first, behind a narrow
store interface. HTTP code may remain async, but storage calls should preserve
clear transaction boundaries and may run in bounded blocking tasks if needed.
The project will not use `sqlx` for the initial SQLite implementation.

In-memory stores may exist only as narrow unit-test adapters for pure core
behavior. They are not the reference implementation of sync, grants,
invitations, mounts, retention, or recovery.

## Consequences

- The first implementation slices include schema and migration work earlier.
- Storage tests can cover restart, replay, idempotency, and corruption
  boundaries from the beginning.
- The project avoids building product proof around a known-wrong pre-release
  persistence shape.
- Core logic remains separate from storage details, but server behavior is
  validated against real transactional state.
- Storage code stays direct and reviewable for the v1 SQLite target.
- A future Postgres or async store would require a separate ADR.
