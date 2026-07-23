# Performance & Simplification Log

Running ledger for the work tracked in `docs/perf-plan.md`. Every benchmark
run, surprise, deferred idea, and redundant-validation observation gets an
entry. Newest entries at the bottom of each section.

Harness: `cargo test --release -p finitechat-server --test perf_baseline -- --ignored --nocapture`
and the same for `-p finitechat-client`. Numbers are from a dev laptop
(Apple Silicon, local SQLite on internal SSD); treat them as relative, not
absolute.

## Benchmark results

(populated by runs below)

## Observations: potential performance improvements

(running list — candidates discovered during the work, not yet acted on)

## Observations: protocol simplification / redundant validation

(running list)

### 2026-06-11 — Baseline (before any optimization)

Server (`perf_baseline`, 20 rooms × 500 + hot room 2,500, 1 KB payloads):

- populate 12,000 publishes: 19.2 s total, 1.60 ms/publish average — average
  grows over the run because each publish clones all prior state
- publish at 12k-entry server state: p50 3.35 ms, p99 3.90 ms
- sync page (100 entries, depth 2.7k): p50 7.8 µs; from seq 0: p50 6.0 µs —
  the linear scan is irrelevant at this depth, will matter at 100k+
- startup replay of 12k ops: 143 ms (~12 µs/op → ~12 s per million ops)

Interpretation: publish latency scales with total server state
(~0.15 ms per MB of cloned state). At the phase target (1–10 GB total
in-memory history) the unmodified clone-the-world path would be 100 ms–1 s+
per publish. Tier 1 §2 confirmed as the server priority.

Client (`perf_baseline`, 1 room, 300-message catch-up):

- sync tick: 562 µs/entry applied
- save_device_state alone: p50 454 µs at minimal state (1 room)
- conclusion: the per-entry full-state save is ~81% of apply cost even at the
  smallest possible state; it grows with rooms × OpenMLS history while the
  actual decrypt stays constant. Tier 1 §1 confirmed as the client priority.

### 2026-06-11 — Phase A complete (client persistence)

Changes: `SqliteClientStore` holds one connection (PRAGMAs once at open, was
per-save); sync and fanout loops apply pages in memory via
`apply_log_entry_in_memory` and save once per dirty page; redundant `rooms`
sort removed from `export_state`.

Same client benchmark after:

- sync tick: 562 µs/entry → **62.7 µs/entry (9.0×)**
- save_device_state alone: 454 µs → **119 µs (3.8×)** — the per-save
  `Connection::open` + PRAGMA batch was ~335 µs of every save
- all 21 client tests green; crash semantics unchanged (replay of at most one
  page, idempotent via the seq guard)

Audit-claim verification worth recording: the OpenMLS storage-record sort in
`export_state` is REQUIRED (records come from a HashMap behind the provider's
RwLock — unsorted), contrary to the perf-audit hypothesis. Only the `rooms`
sort was redundant (BTreeMap source).

New observation for the improvements list: `export_state` still clones every
OpenMLS storage value on every save (~the dominant remaining save cost as
state grows). Fix would be content-addressed or per-record persistence —
Phase E material, recorded under improvements.

### 2026-06-11 — Phases B+C complete (upstream core + server hot path)

Upstream (`transport-http-server`, commit `4354cd4` on `http-delivery-upstream`):
`HttpDeliveryLimits` (configurable caps; defaults unchanged), `check_publish`
dry-run whose `Fresh` carries the exact predicted receipt, `partition_point`
page start. The queues share one `check_append` between the dry run and the
real append, so the two paths cannot drift.

Server changes: raw `/messages`, typed `/events`, `/application-events`, and
typed `/commits` all moved to **check (read-only) → persist (one SQLite tx) →
apply (infallible)**. `apply_mutation` and every whole-service clone are
deleted. `submit_commit` keeps candidate clones only for the small projection
maps. `from_sqlite_path` applies `finite_delivery_limits()` (65,536 rooms,
262,144 entries/room — replacing the 4,096-entry cliff) before op-log replay.
The durable store now holds one connection (was: `Connection::open` per
operation across 24 sites) and finally sets `journal_mode = WAL` — it had
been running on SQLite defaults the whole time.

Same server benchmark after (two runs, stable):

- publish at loaded server state: p50 3.35 ms → **46 µs (72×)**, p99 3.90 ms
  → ~100 µs, and **flat with state size** (the persist-first path does no
  O(state) work; the remaining cost is the WAL fsync + validation)
- populate 12,000 publishes: 19.2 s → **0.63 s (30×)**
- sync page: ~6 µs (unchanged; now O(log n) by construction at any depth)
- startup replay: 143 ms (unchanged — Phase E)
- client re-check: 60 µs/entry, save 116 µs (Phase A results hold)

Budgets vs. plan: publish p99 ~0.1 ms against a 25 ms budget; sync p99
~10 µs against 10 ms. Both met with two orders of magnitude of headroom.

## Observations: potential performance improvements (updated)

1. Client `export_state` still clones every OpenMLS storage value per save;
   per-record persistence is the Phase E shape if device state grows large.
2. Server startup replay is O(history) (~12 s per million ops): snapshot +
   horizon (Phase E), co-designed with retention.
3. Idempotency maps are unbounded in memory and fully loaded at startup —
   same horizon design.
4. The in-memory full-history mirror costs ~1 KB/entry of RAM; pruning or
   paging cold entries from SQLite is the Phase E memory answer.
5. `submit_commit` still clones the whole account-room directory and
   room-membership maps per commit (rare path, projections only); scoping the
   candidates to the touched room/accounts is a follow-up.
6. `PublishIdempotencyRecord` fingerprints store a full request clone where a
   digest would do — doubles idempotency memory and persisted row size.
7. The typed-commit replay path re-publishes Welcomes through
   `publish_message`, re-taking locks per Welcome (rare, correct, mildly
   wasteful).
8. If the per-publish WAL fsync ever becomes the bottleneck, `synchronous =
   NORMAL` is the knob — deliberately NOT taken now; durability-on-ack is the
   product.

## Observations: protocol simplification / redundant validation (updated)

1. Accepted publishes now run `validate_transport_message` twice (dry-run
   check, then the apply's own publish re-validates). Microseconds, but an
   upstream `apply_unchecked` entry point would make the check/apply contract
   explicit instead of re-validated — candidate for the next upstream PR.
2. Typed `/commits` validates membership-delta structure at the route, and
   the projection appliers re-validate overlapping invariants while building
   mutations. One authoritative validation pass feeding both would simplify;
   needs care because the appliers' checks also guard the replay path.
3. Client-side per-entry `validate_limits` on server-returned pages is
   deliberate defense-in-depth at the decrypt boundary — reviewed and kept.
4. `PublishMessageFingerprint` duplicating the full request is both the perf
   item (6) above and a wire-simplification: idempotency equality only needs
   a content digest.

### 2026-06-11 — Grill session outcomes (protocol simplification)

The six simplification observations were stress-tested with the user and all
six accepted (decisions + execution order in
`docs/adr/0004-protocol-surface-simplifications.md`). Two turned out to be
latent always-works defects, found during the grilling:

- the scoped idempotency capacity rule permanently blocked a sender after
  4,096 lifetime messages per room (records never expire) — exactly the
  long-chat scenario this phase targets;
- the production client sends through the no-effects `/events` route, so
  push/unread delivery effects are never recorded for real traffic.

Standing posture recorded in ADR 0004: Marmot interop is kept only when free
— never bend the product surface to preserve it.

### 2026-06-11 — ADR execution perf check

Re-ran both harnesses after the eight ADR 0003/0004 implementation steps
(admin authority, leave, versioning, and five surface deletions): publish
p50 49.8 µs, sync page unchanged, client apply 61.9 µs/entry, save 109 µs.
No regression from the added validation (admin/authority checks are map
lookups on the already-locked projection).

### 2026-06-11 — Snapshot startup + remaining ADR items

- Server now snapshots all op-derived state every 4,096 ops (and via
  `snapshot_now()`); startup = snapshot + tail replay. Measured at 12k ops:
  146 ms full replay → 114 ms from snapshot. The modest delta is honest: at
  this phase live state ≈ history size, so parsing one big blob ≈ parsing
  many small ones. The structural win is what the test proves — the snapshot
  is authoritative for its prefix (ops deleted under it, server still serves
  the full log), which is exactly the contract horizon compaction needs.
- New observation: snapshot/op-log JSON parsing dominates startup either
  way; a binary format (postcard) for the snapshot blob is the cheap next
  step if startup ever matters more.
- Push tokens and stream-lane kinds added with no hot-path impact.

### 2026-06-12 — Agent invite flow + the latency story (/sync/wait)

The invite/agent phase (ADR 0006) ended with the latency pass:

- **`/sync/wait` long-poll wake hints.** One server-held request replaces
  client-side sleep loops: it returns when a watched room log advances past
  the caller's cursor or a watched invite session changes (counts-keyed
  predicates so stale wakes cannot spin), capped at 25 s. Hints never
  advance state — pull-based sync is untouched. Publish-side cost is one
  `Notify::notify_waiters` (an atomic when nobody waits).
- **Measured:** the full two-home CLI pairing e2e (init ×2 → invite → join
  with PIN proof → admit → welcome → four message round trips) went
  **15.4 s → 0.49 s** once `hermes poll`/`hermes join` rode the wait route
  and poll returned on admitted joins. Message delivery latency is now
  ~1 RTT after send; the PIN→chatting handshake is a handful of RTTs.
- One hub `Notify` wakes all waiters who re-check their own predicates —
  right-sized for hundreds of users; per-key channels are the documented
  next step if waiter counts grow.
- Own application messages no longer round-trip through MLS decryption
  failure handling: the sync tick advances the cursor directly for
  sender==self application entries (also removed a latent ProcessMessage
  error for any client that sends and syncs).
- Observation (bridge): the platform adapter spawns a subprocess per bridge
  call, each reopening and decrypting the client store (~10 ms class). The
  long-poll amortizes this for poll; a resident daemon mode (or SSE lane,
  ADR 0003 §6) is the next step if send-side overhead ever matters.
- Observation (redundant validation): none added — invite-session
  verification is inviter-side only by design; the server stores opaque
  rendezvous material and never re-checks proofs.

### 2026-06-12 — Bridge-path benchmark: hot spots vs the theoretical floor

New ignored harness `crates/finitechat-cli/tests/perf_bridge.rs` measures
every leg a hermes message crosses, in release mode against a live server
(local pipeline; the container leg adds one vmnet RTT, ~0.2–0.5 ms, once
the runtime is installed). Question asked: are we doing anything
pathological, and is polling the blessed abstraction?

Measured legs (p50):

| Leg | Cost |
| --- | --- |
| MLS encrypt 1 KiB | 33 µs |
| Client store save (full snapshot) | 141 µs |
| HTTP POST /events (serialize + loopback + WAL) | 269 µs |
| Receiver sync+decrypt+persist | ~98 µs/entry |
| Publish → /sync/wait wake | ~0.6 ms |
| `hermes send` via subprocess bridge | 4.4 ms |
| Subprocess floor (`hermes pin`: spawn + store open) | 3.5 ms |
| End-to-end bridge send → peer decrypted | 5.8 ms |
| Pairing handshake (join submitted → verified member) | ~18 ms |

**Pathological thing found and fixed:** opaque payload bytes serialized as
JSON number arrays — 4.5× wire size for 1 KiB ciphertexts, 3.6× (118 KB!)
for 32 KiB. Now base64 strings (reads stay tolerant of the legacy array
form for stored logs): 1 KiB wire 4,606 → 2,009 bytes, 32 KiB 118 KB →
44 KB, large-payload publish 3.6 → 1.45 ms, bulk receive 9.9 → 6.3 ms per
64 entries.

**Is polling the blessed abstraction? Yes, in its current two-part form.**
Pull-based sync pages are the consistency model (hints never advance
state) and are not up for debate. The wake *transport* on top is the
server-held long-poll, and the numbers validate it: publish→wake is
~0.6 ms, within ~2× of any push channel on the same socket, and the
classic failure mode — an adapter polling too slowly — is structurally
gone because the server paces the wait, not the client (a slow adapter
adds only its own dispatch time, never a polling interval). SSE/WebSocket
would save per-wait request overhead, not meaningful latency; not worth
the connection-lifecycle machinery yet.

**The real remaining hot spot is spawn-per-bridge-call, not polling:**
~3.5 ms of the 4.4 ms send cost is process spawn + encrypted-store open;
the in-process floor for the same work is ~0.4 ms. At agent timescales
(LLM tokens arrive in seconds) this is invisible, so it stays — the
designed next step, when it matters, is a resident bridge process
(`hermes serve`, JSON-lines over stdio, same contract as today's
subcommands) which would also let one long-poll loop run continuously
instead of re-arming per poll call.

### 2026-07-08 - Electron + hosted-runtime Hermes feels fast

Validated product shape:

- `chat.finite.computer` deployed from finitechat `28887848e058`.
- Runtime image `ghcr.io/finitecomputer/finite-agent-runtime:2026-07-08.hermes-stream.1`
  packages the same finitechat commit and Hermes agent `0.18.0`.
- Local proof ran the image under Apple `container` with a clean durable
  mount, the Electron app using the Rust desktop daemon, and the legacy-derived
  chat UI talking to the deployed server.

Why it feels fast:

- **The room server wake path is already hot.** Message publish wakes
  `/sync/wait`/stream waiters immediately. Hints still only mean "pull
  needed"; durable sync remains the consistency model. This keeps latency near
  one network RTT without letting a push callback mutate state directly.
- **Hermes now rides a resident Rust sidecar.** The Python Hermes adapter is
  thin and holds one local `/v1/hermes/inbound` NDJSON stream to
  `finitechat hermes serve`. The Rust sidecar owns invite admission, sync,
  cursoring, inbox persistence, ack, send/edit/activity, and AppState-aligned
  room projection. The hot path no longer respawns a CLI process or reopens the
  encrypted store for every inbound message.
- **The sidecar stream stays resident while idle.** Commit `28887848e058`
  fixed the bad idle behavior where the advisory wait stream could end and
  make `/v1/hermes/inbound` EOF. The sidecar now falls back to a normal bridge
  poll and keeps the NDJSON response alive. The regression in
  `hermes_cli_inits_invites_admits_and_round_trips_messages` asserts an idle
  inbound stream emits heartbeat lines instead of ending.
- **Invite admission no longer waits on the wrong KeyPackage path.** Admission
  uses the verified inline KeyPackage carried by the invite join request, and
  the server validates the commit Add against that stored request. This removes
  the earlier `no KeyPackage available for invited device` failure and avoids a
  slow "waiting for Hermes" state that was actually protocol skew.
- **Hermes inbox cursoring no longer eats the first user message.** The agent
  inbox initializer only advances over the agent's own prior events before the
  first counterparty event. A freshly admitted Electron or iOS message is
  recovered into Hermes instead of being marked consumed.
- **Electron is thin over AppState.** The renderer subscribes to the daemon's
  projected AppState and dispatches Rust actions. It does not implement its own
  protocol loop, delivery semantics, or Hermes polling. Launch-time
  `finite://join` handling is durable: the main process retains the pending
  invite URL until the renderer explicitly consumes it after resolving the
  daemon URL.
- **Bridge health is explicit.** Runtime `/healthz` and `/invite` include a
  `bridge` object. If the Hermes stream is broken, the runtime reports a real
  bridge error instead of leaving the UI stuck on a generic waiting state.

What made earlier runs slow or stuck:

- server/runtime/client version skew;
- invite admission depending on global KeyPackage inventory;
- Hermes inbox cursor initialization advancing past the first user message;
- the inbound stream closing on idle advisory-wait EOF and reconnecting in a
  tight loop;
- Electron losing the initial invite URL before React subscribed to IPC.

The current manual Electron test intentionally skipped a visible invite-code
entry step because the app was launched with a full `finite://join` URL. That
is the right deep-link path for this proof, but it is not the final onboarding
UX by itself. A polished product flow should still present a clear invite
entry/import path when the app is opened normally, and should explain whether
the user is generating a fresh device key or importing an existing account key.

### 2026-07-23 — Core/Electron and resident bridge follow-up

Plan 10's bounded follow-up audited two new hot paths rather than repeating the
June device-save/server-clone work. The full report is
`docs/audits/rust-hot-path-perf-audit-2026-07-23.md`.

- Core's normal runtime tick already applied fresh events/messages to its
  in-memory projection, then loaded/decrypted up to 5,000 stored messages and
  5,000 stored events and rebuilt the same projection. The new ignored harness
  measures the maximum retained history: p50 **88.050 ms → 175 µs**
  (approximately **502×**). Startup remains the full-rebuild boundary.
- The resident Hermes bridge signalled every consumer on every successful
  ten-second reconciliation, even with no updates. Each wake could reopen and
  decrypt the 5,000-event recovery window; cold Hermes collection scanned it
  twice. Empty reconciliations no longer wake consumers, initialized inboxes
  return before store access, and cold initialization/recovery share one scan.
- Finite Private limiter reserve/settle was intentionally deferred until an
  approved load result and rollout path select it.
