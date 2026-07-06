# Core Persistence Conventions

Status: enforced standard for `crates/finite-saas-core`. New store code that
violates these will be sent back in review. Born from the 2026-07-04 incident
where a global-lock, whole-DB-rewrite persistence model plus an `ON CONFLICT`
on a non-unique column deterministically broke paid signups — invisibly.

## The one rule that matters most

**Every write touches only the rows it changes, inside a scoped transaction.**
No operation may load the entire database, mutate it in memory, and write it
all back. No operation may take a process-wide lock to do its work.

## Anti-patterns (do NOT reintroduce)

1. **Global advisory lock.** `pg_advisory_xact_lock(<constant>)` serialized
   the ENTIRE system through one lock. Banned. Concurrency is handled by
   row-level locks (`SELECT ... FOR UPDATE`, `FOR UPDATE SKIP LOCKED`) and
   per-row unique constraints, scoped to the affected keys only.
2. **Load-all → mutate → persist-all.** The old `lock_state → load_state →
   mutate BridgeCoreState → persist_state` pattern rewrote every table on
   every call — including on *reads* (`billing_overview`). One unrelated
   org's bad row could fail an unrelated user's request. Banned. Each
   operation reads and writes only its own rows.
3. **A read that writes.** Reads run in read-only transactions and mutate
   nothing. If a "read" needs to persist, it's two operations, not one.
4. **`ON CONFLICT (cols)` without a matching unique constraint.** This fails
   at *runtime* with SQLSTATE 42P10, not at compile time. Every `ON CONFLICT`
   target MUST correspond to a real `UNIQUE`/PK/exclusion constraint or
   unique index in the migration. When you write an upsert, add the matching
   constraint in the same change, and add a test that exercises the conflict
   branch against real Postgres.
5. **Primary keys derived from PII/inputs.** IDs like `user_id = f(email)`
   couple identity to a value that outlives deletion — a wiped+recreated
   account rebuilt the same keys and collided with orphans. Banned. Use
   opaque surrogate IDs generated at insert time; look rows up by their
   natural key (e.g. `users.normalized_email UNIQUE`).

## Idiomatic patterns (DO these)

### Transaction shape
```rust
let mut client = self.client.lock().await;
let tx = client.transaction().await.map_err(store_error)?;
let result = do_the_thing(&tx, input).await?;  // touches only its rows
tx.commit().await.map_err(store_error)?;
Ok(result)
```
Reads that don't write: `tx.execute("SET TRANSACTION READ ONLY", &[])` or
just don't call any write.

### Upserts
`INSERT ... ON CONFLICT (<unique-key>) DO UPDATE SET ...` where `<unique-key>`
is provably a constraint in `migrations/`. Prefer `DO NOTHING` when you only
need existence. Never `ON CONFLICT` on a non-unique column.

### Idempotency
Enforce it with a `UNIQUE` constraint on the natural idempotency key (e.g.
`UNIQUE(owner_user_id, idempotency_key)`), then: look the row up by that key;
if present, return it (`reused: true`); else insert. Do NOT derive the row's
primary key from the idempotency inputs to fake dedupe.

### Surrogate IDs
Generate opaque ids at insert (`prefix_` + random). Resolve entities by
natural key via a unique index, never by recomputing an id from inputs.

### Concurrency on a queue
`SELECT ... FOR UPDATE SKIP LOCKED` scoped by the relevant partition (e.g.
per source-host), not a global claim across all tenants.

## Errors: never be blind again

- `store_error` MUST preserve `tokio_postgres::Error::as_db_error()`
  (code/constraint/table/column/detail) into `CoreError::Database`. Never
  collapse a DB error to `error.to_string()` (that renders literally
  `"db error"`).
- The HTTP layer returns a GENERIC message + a `correlation_id` to the user;
  the full detail is logged server-side (`tracing::error!`) with operation +
  user/org context and the same correlation id. Users quote the id; secrets
  and schema detail never leave the server.
- Every mutation path carries a `tracing` span with `operation` and the key
  ids.

## Testing contract (both, per the 2026-07-06 decision)

- **Every store operation** is tested against an **ephemeral real Postgres**
  (not the in-memory model — it cannot enforce UNIQUE/CHECK/FK and is what
  hid the incident). Cover the FAILURE paths too: constraint violations,
  wipe→re-signup, out-of-order webhooks, stuck leases.
- **One golden-path E2E per PR**: signup → checkout (Stripe test clock) →
  create → lease → launch (fake runner) → invite-ready, against real
  Postgres. If any hop breaks, the PR is red.
- A new upsert without a test that hits its conflict branch is incomplete.

## When you add or change a table

1. Write the migration with every constraint the code relies on (unique keys
   behind every `ON CONFLICT`, FKs, CHECKs) using the idempotent `DO $$`
   block style already in `0001_core.sql` so it re-applies to prod safely.
2. Add the row-scoped read/write functions — no full-state helpers.
3. Add Postgres-backed tests including the conflict/failure branches.
4. If it has an idempotency key, add the `UNIQUE` and the lookup-first path.
