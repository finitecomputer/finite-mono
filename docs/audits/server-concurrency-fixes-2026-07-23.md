# Core and Finite Sites concurrency fixes

Date: 2026-07-23

Status: **IMPLEMENTED LOCALLY — NOT RELEASED OR DEPLOYED**

## Decision

The next performance work after the Finite Chat audit prioritizes:

1. Core's single Postgres connection mutex; and
2. Finite Sites' global Engine mutex on serving traffic.

Specialization-worker video extraction and Brain authorization-cache eviction
remain observations only and require a separate Austin sync before
implementation.

## Core Postgres

Before this change, all 64 Postgres checkout sites shared one
`Arc<Mutex<Client>>`. A slow independent query prevented heartbeat, runner
lease, billing, and dashboard operations from reaching Postgres.

Core now uses a bounded eight-connection `deadpool-postgres` pool. Every
existing transaction still checks out exactly one connection and retains its
existing SQL, row locks, commit, and rollback boundary. Fast recycling avoids
adding a validation query to every checkout; closed connections are rejected
when returned to the pool, and query failures surface through the existing
structured database error path.

Deterministic proof holds one pooled connection in `pg_sleep(1)` and requires an
independent store read to finish within 500 ms. The complete Postgres-backed
Core suite runs against an isolated-database harness and retains concurrent
redemption, revocation, leasing, idempotency, and rollback coverage.

Operational risk is **medium**:

- connection use rises from one to at most eight per Core process;
- query interleavings that Postgres was already designed to serialize can now
  occur in one process; and
- rollback is one component deploy, with no schema or durable-data change.

## Finite Sites

Before this change, site resolution, SQLite lookup, filesystem blob reads, and
Markdown rendering shared the control-plane Engine mutex. One cold asset or
large document could head-of-line block unrelated sites and control work while
also occupying a Tokio async worker.

Finite Sites now keeps the existing single writer Engine and adds eight
independent query-only SQLite WAL readers for serving traffic. Serving
registry work runs on Tokio's blocking pool. Verified blob reads and document
rendering occur after the reader is returned and never hold the writer mutex.

The publication boundary is unchanged: immutable content-addressed blobs are
stored and verified before the active version pointer changes. A request keeps
the resolved `active_version_id`, so it cannot combine a new manifest with old
bytes. Serving readers cannot write.

Deterministic proof blocks one serving reader and requires a second reader to
complete within 250 ms. Store coverage proves a read-only connection observes a
committed writer update and rejects SQL mutation. The full 23-test HTTP/Git
end-to-end suite covers static revalidation, document rendering, visibility,
viewer-session revocation, publishing, and restart reconciliation.

Operational risk is **medium**:

- the registry remains one SQLite WAL database and one writer;
- serving adds eight read-only file descriptors/connections;
- no schema, manifest, blob, cookie, or URL contract changes; and
- rollback is one component deploy with no durable-data migration.
