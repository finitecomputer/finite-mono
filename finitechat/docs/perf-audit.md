# Performance & Simplification Audit

Date: 2026-06-10. Scope: `finitechat-client`, `finitechat-server`,
`finitechat-proto`, and the upstream `transport-http-server` core. Method:
full-file reads of the two big crates plus targeted verification of every
headline claim against source. Line numbers are as of commit `3be5b0d`.

Framing: nothing here is wrong *today* — the test suite proves correctness,
and current scale (single server, small groups) tolerates all of it. The audit
ranks by (a) structural debt that gets more expensive to fix after shipping,
(b) cheap wins, and (c) things that look suspicious but should be left alone.
Simplifications are ranked above raw perf where they reduce maintenance
burden, per our priorities.

---

## Tier 1 — structural, fix before shipping to users

### 1. Client: full-state re-encrypt + new SQLite connection per applied entry

The single biggest cost driver in the codebase, verified:

- `SqliteClientStore::apply_log_entry_and_save` calls `save_device_state`
  **per entry** inside the sync loop (`client/src/lib.rs:3570` in
  `sync_room_pages`, `:3513` in fanout completion), plus
  `advance_room_cursor_and_save` saves **again per page** (`:3580`, `:3523`).
- Every `save_device_state` (`:2436`) runs `export_state` → clones *all*
  OpenMLS storage records for *all* rooms, re-sorts them, serializes the
  entire device snapshot, AES-256-GCM-encrypts the whole blob, and writes it
  in a transaction.
- Every save also calls `connect()` (`:2621`), which `Connection::open`s a
  brand-new SQLite handle and re-runs the `PRAGMA` batch (WAL, synchronous,
  busy_timeout) — per save.

Cost shape: applying a 100-entry page costs ~101 full-snapshot encryptions of
a blob that grows with rooms × OpenMLS history, plus 101 connection opens.
A device in 10 active rooms catching up after a day offline will do thousands
of full-state encryptions to apply a few thousand small entries.

Why it's Tier 1: the *durability semantics* are the contract ("a crash between
entries never replays or skips"), and the contract is what the tests pin. The
fix must preserve exactly that, which is easy now and annoying after more
call sites accrue.

Fix sketch (medium effort, low risk under existing tests):
1. Hold one `Connection` in `SqliteClientStore` (open in `open()`), reuse it;
   `rusqlite::Connection` is `Send`, the store is already `&mut self`
   everywhere. Removes per-save open + PRAGMA cost outright.
2. Batch the save boundary: apply a full page in memory, save once per page
   (the existing `advance_room_cursor_and_save` at page end already provides
   the natural boundary — make the per-entry variant non-saving and let the
   page loop own persistence). Crash mid-page then replays at most one page,
   which the seq-dedup guard (`:2602`, `entry.seq <= last_applied` → skip)
   already makes idempotent. The daemon-survival contract is unchanged: durable
   state still never runs ahead of acks.
3. Longer term (only if profiling demands): split the snapshot so OpenMLS
   storage records persist as individual rows keyed by (room, key) instead of
   inside one monolithic encrypted blob. Bigger change; do not do this until a
   real device shows the page-batched version is still too slow.

### 2. Server: clone-the-world candidate per mutation

`apply_mutation` (`server/src/lib.rs:~155`) clones the **entire**
`HttpDeliveryService` — every group's full message log, every inbox, every
dedup index — as the rollback candidate for each mutating request. The same
candidate pattern repeats for `key_package_inventory` (6 sites),
`room_memberships`, `account_rooms`, and peaks in `submit_commit`
(`:~1737`), which locks five maps and clones all five before validating.

Cost shape: O(total server state) per write. At one user it's megabytes per
publish at worst; at 100 rooms × 4k entries it's untenable.

Why it's Tier 1: the candidate/persist/swap pattern is *load-bearing for
crash atomicity* (mutate copy → append op to SQLite → swap), and the crash
matrix tests pin that behavior. Replacing it wholesale is risky; bounding it
is not. Two options in order of preference:

1. **Cheap structural fix:** make the clones shallow. Wrap each
   `GroupQueue.entries` / `InboxQueue.entries` in `Arc<…>` (or store
   `Arc<TransportMessage>` per entry) so cloning the service shares the logs
   and only copies map skeletons. Append then uses `Arc::make_mut` /
   copy-on-write at the single queue being touched. This keeps the
   candidate pattern and its test-pinned semantics intact while making the
   per-request cost O(touched queue), not O(world). Note `entries` lives in
   the upstream crate — this is a legitimate upstream improvement
   (`HttpQueuedDelivery { message: Arc<TransportMessage> }`), or finitechat
   can stop cloning at the wrapper level by scoping candidates per group.
2. **Bigger rewrite (only at multi-tenant scale):** delta-based mutations with
   explicit rollback. Not worth it until clone profiling says so.

The five-lock clone in `submit_commit` also deserves a `fn commit_candidates`
helper regardless — the lock-acquire/clone/swap choreography is repeated
boilerplate and the place a future lock-ordering bug would sneak in (ordering
is currently consistent everywhere; keep it that way by centralizing it).

### 3. Server: unbounded growth — op log replay, idempotency maps

Three related unbounded structures, all fine today, all needing one shared
answer (and the answer interacts with the retention decision in the feature
audit §1.6):

- Startup replays the **entire** operation log into the in-memory core
  (`from_sqlite_path`, `:100–123`): O(history) startup, forever.
- `publish_idempotency` and `key_package_claim_idempotency` HashMaps grow
  per unique key with no TTL; the capacity cap is per room/sender, not global,
  and the records are also all loaded at startup.
- The upstream in-memory queues cap at 4,096 entries per route
  (`MAX_HTTP_QUEUE_ENTRIES_PER_ROUTE`) — an active room will simply *fill up*
  and start rejecting publishes. This is the nearest actual cliff: a busy
  room hits 4,096 durable entries long before any perf limit matters.

Fix sketch: one design, "log horizon + snapshot": periodically persist a
snapshot of the in-memory core (it's all serde-serializable), record the op
seq it covers, replay only the tail at startup, and let the same horizon
drive idempotency-record expiry and (with the retention decision) entry
pruning with tombstoned cursors. Raising the upstream queue cap (or making it
configurable) is a one-line upstream PR that should happen first.

---

## Tier 2 — cheap wins, do opportunistically

1. **Upstream `sync_page` linear scan** (`transport-http-server`, `sync_page`):
   skips entries one-by-one from seq 0 on every sync — O(total log) per page
   request. `entries` is sorted by seq, so
   `entries.partition_point(|e| e.seq <= after_seq)` makes it O(log n) in a
   3-line change with no behavior difference. Nice little upstream PR that
   also polishes the crate you're presenting.
2. **`export_state` re-sorts what BTreeMaps already keep sorted**
   (`client/src/lib.rs:~602, ~615`): records pulled from BTreeMaps are sorted
   again before encoding. Iteration order of a BTreeMap is already the sort
   order — delete the sorts (keep a debug_assert if nervous).
3. **`claim_key_package_for_device` double-decode** (`client/src/lib.rs:
   ~2956`): the claimed package payload is `serde_json::from_slice`-decoded
   into a full `UploadKeyPackageRequest` just to re-extract fields the route
   could return directly. Either extend the claim response DTO (small wire
   change, do before v1 freeze) or accept one decode and stop re-cloning the
   four strings.
4. **Fanout room lookup is O(n) three times per prepare**
   (`link_fanout_room`, `client/src/lib.rs:~1872`; called at `:1695`, `:1712`,
   `:1744`): `rooms: Vec<LinkFanoutRoomState>` scanned by room id. n ≤ 32 so
   it's microseconds — take it only as part of touching that code; switching
   to a map changes the persisted state shape, so it is *not* free.
5. **Lock boilerplate** (`server`, 48+ `lock().expect("…")` sites): a
   `fn locked<T>(&self, m: &Mutex<T>) -> MutexGuard<T>` or per-map accessor
   trims noise and centralizes the panic message. Pure readability.
6. **`publish_message` / `publish_typed_event_message` near-duplication**
   (`server/src/lib.rs:166–279`): same idempotency/candidate choreography
   twice. One `publish_with_idempotency(request, scope_rule)` helper removes
   ~60 lines. Do it together with the `submit_commit` candidate helper from
   Tier 1 §2 so the pattern lives in exactly one place.

---

## Tier 3 — looked suspicious, leave alone (with reasons)

- **`Arc<Mutex<…>>` per state map on `HttpServerState`** — the prompt's
  instinct was right to question it, but the audit verdict is: keep. The maps
  have consistent lock ordering, no I/O inside critical sections, fine-grained
  scopes, and Axum's cloneable-state model genuinely wants shared ownership.
  The *clone inside* the lock is the problem (Tier 1 §2), not the lock.
  One real caveat: these are sync mutexes inside async handlers — fine while
  critical sections stay micro-scale; re-evaluate if any section ever does I/O.
- **Blocking `reqwest` client in `HttpRuntimeDelivery`** — constructed once,
  cloned cheaply (internal Arc), used from the sync worker. Correct as long as
  the runtime worker is a thread, which it is.
- **`RuntimeDelivery` trait with one production impl** — keep. It is the test
  seam for failure injection (`InProcessHttpTransport`) and the boundary the
  whole client-state suite is written against. Deleting it saves ~50 lines
  and costs the test architecture.
- **BTreeMaps where HashMaps would do** (client device state) — deterministic
  iteration order is what makes snapshot encoding deterministic, which the
  encrypted-snapshot tests rely on. Keep.
- **Per-entry `validate_limits` on server-returned pages** — defense in depth
  on the decrypt path, microseconds per call. Keep.
- **JSON + SHA-256 per publish** (digest, op-log row, idempotency
  fingerprint) — linear in payload size, dominated by network and the Tier 1
  items. Not worth touching until both Tier 1 items are done; any payload-
  format change (e.g. not JSON-encoding payload bytes into the op log) should
  ride along with the snapshot/compaction work, not happen alone.
- **`ClientError`'s very large variant count** — it is verbose, but every
  variant is load-bearing in test assertions, and collapsing them would churn
  the suite for compile-time gains only. Revisit if compile times actually
  hurt.

## Suggested sequencing

1. Client save batching + persistent connection (Tier 1 §1) — biggest user-
   visible win (sync latency on real devices), smallest blast radius, fully
   covered by existing tests.
2. Upstream pair: `partition_point` sync fix + configurable queue cap
   (Tier 2 §1, Tier 1 §3 cliff) — small, strengthens the upstream PR.
3. Server candidate-clone bounding via `Arc` log sharing (Tier 1 §2),
   together with the candidate/idempotency helpers (Tier 2 §6).
4. Snapshot + horizon design (Tier 1 §3), co-designed with the retention
   decision from the feature audit.

Every item above is protected by the existing suite: the crash matrices pin
the durability semantics the fixes must preserve, which is exactly what the
suite was built for.
