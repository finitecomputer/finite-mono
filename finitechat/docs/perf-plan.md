# Performance Plan: Hundreds of Users, Dozens of Long Chats

Date: 2026-06-11. Executes the Tier 1/Tier 2 findings from
`docs/perf-audit.md` and queues the protocol decisions from
`docs/feature-audit-marmot-pika.md`. The running measurement/observation
ledger lives in `docs/perf-log.md`.

## Scale target for this phase

- ~500 active users, 2 devices each (~1,000 devices)
- ~30 rooms per user, ~5 accounts per room → ~3,000 rooms server-wide
- "Long chats": hot rooms reach 10k+ durable entries; total server history
  on the order of 1–10M entries over time
- Peak sustained traffic guess: 50–200 publishes/sec server-wide

## Latency budgets (server-side, excluding network)

| Operation | p50 | p99 |
| --- | --- | --- |
| Typed/raw message publish (durable) | < 5 ms | < 25 ms |
| Group sync page (100 entries, any depth) | < 2 ms | < 10 ms |
| Commit submit (rare path) | < 50 ms | < 250 ms |
| Client: apply one 100-entry page incl. persistence | < 250 ms | < 1 s |

The publish budget is dominated by the SQLite fsync we *want* (durability is
the product); the plan removes the work that is not the fsync.

## Method

1. **Baseline first.** Ignored release-mode timing tests
   (`finitechat-server/tests/perf_baseline.rs`,
   `finitechat-client/tests/perf_baseline.rs`) capture numbers before any
   change. Same harness re-runs after each phase. Numbers go in
   `docs/perf-log.md`, not in test assertions (no flaky CI thresholds).
2. **Tests are the safety rail.** Every change must keep
   `cargo test --workspace`, clippy `-D warnings`, the Python suite, and the
   delivery conformance tests green. The crash matrices pin the
   durability semantics each optimization must preserve.
3. **Log as we go.** Every measurement, surprise, deferred idea, and
   redundant-validation observation goes in `docs/perf-log.md`.

## Phases

### Phase A — client persistence (perf-audit Tier 1 §1)

1. `SqliteClientStore` holds one open `Connection` (PRAGMAs run once at
   open). Today every save opens a fresh connection and re-runs PRAGMAs.
2. Page-batched saves: the sync and fanout loops apply entries in memory and
   persist once per page (the existing per-page cursor save becomes the only
   save). Crash mid-page replays at most one page; the existing
   `seq <= last_applied` guard makes replay idempotent, and durable state
   still never claims work that wasn't done (acks happen after their save).
   `apply_log_entry_and_save` keeps its semantics for external callers.
3. Verify-then-remove redundant sorting in `export_state` (only where the
   source container already guarantees order — OpenMLS records come from the
   provider and may NOT be ordered; check before touching).

### Phase B — delivery core

1. `sync_page` start via `partition_point` (entries are seq-sorted):
   O(log n) instead of O(n) skip scan.
2. `HttpDeliveryLimits` configuration on `HttpDeliveryService` with
   `Default` equal to today's constants. Raises the real cliff: 4,096
   entries/queue and 1,024 groups are too small for this phase's target.
   finitechat-server configures larger limits.
3. `check_publish` dry-run: validate a publish (including duplicate-replay
   detection and admission) without mutating, so a durable wrapper can
   persist-first and then apply infallibly. This is what removes
   clone-the-world from finitechat's hot path (Phase C) and is a reasonable
   upstream story ("validate before you spend durability").

### Phase C — server hot path (perf-audit Tier 1 §2)

1. Raw `/messages` and typed `/events`//application-events` publishes move to
   **validate → persist op → apply to live state**, eliminating the
   whole-service candidate clone on the paths that carry actual chat traffic.
   The upstream publish is checked to be all-or-nothing (all validation
   precedes any mutation), so post-persist apply cannot fail.
2. Commit paths (`/commits`, raw commit import) KEEP the candidate pattern:
   commits are rare (membership changes), and their multi-map atomicity is
   exactly what the candidate pattern is good at. Bounding their cost is
   queued for Phase E if measurements demand it.
3. Centralize the candidate/lock choreography in helpers; merge the
   near-duplicate publish handler bodies while in there.

### Phase D — validation

Re-run the harness; compare against baseline; record in `docs/perf-log.md`.
Full workspace verification. Ship.

### Phase E — queued next (not this turn)

1. **Snapshot + horizon** for the server: periodic in-memory-state snapshot
   keyed to an op seq; startup = snapshot + tail replay; idempotency-record
   expiry tied to the same horizon. Co-design with the retention decision
   (feature audit §1.6) so compaction and disappearing messages share cursor
   semantics. This is also the answer to the in-memory full-history mirror's
   memory envelope (~1 KB/entry today).
2. **Protocol decisions** from the feature audit, in its §5 order:
   versioning/capability slots, admin authority, leave-group, retention
   field + below-horizon sync rule, push wake contract, stream-lane
   reservation, recovery doc. Items 1–3 change typed-route validation and
   should land before any external client exists.

## Out of scope, deliberately

- Multi-server / horizontal scale (the architecture is single-server by
  design at this phase).
- Replacing JSON in the op log or on the wire (rides with Phase E snapshot
  work if profiling demands it).
- Async SQLite or connection pooling (one writer is correct for the
  single-server model).
