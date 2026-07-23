# Rust hot-path performance audit: Core, Electron, and user bridge

Date: 2026-07-23

Status: **IMPLEMENTED LOCALLY — NOT RELEASED OR DEPLOYED**

Scope: plan 10 from `docs/next-work-plans-2026-07-23.md`

## Decision and boundary

Paul selected the one-day static-audit option and prioritized the shippable
Finite Chat path before the harder-to-roll-out Finite Private limiter:

1. `finitechat-core` as exercised by the Electron daemon; and
2. the resident `finitechat-cli` Hermes/user bridge.

After the first two fixes were reviewed, Paul explicitly asked for continued
scanning under the hard rule that existing Finite Chat users must retain open,
sync, send, restart, and recovery behavior. One further Core sync fix met that
bar and is recorded below.

This audit does not profile production, generate load, change a database
schema, redesign the async runtime, or touch the Finite Private limiter.
Existing work in `finitechat/docs/perf-audit.md` and
`finitechat/docs/perf-log.md` was read first; the previously fixed full-device
serialization and whole-server-clone paths are not reported as new findings.

## Ranked findings

### P1 — Core rebuilt the full chat projection after applying each fresh delta

- **Path:** `AppRuntimeState::runtime_tick` in
  `finitechat/crates/finitechat-core/src/lib.rs`.
- **Trigger:** an Electron/hosted app update hint that runs the normal runtime
  tick.
- **Cost before:** after `sync_with_projection` had returned and the runtime had
  applied its fresh events and messages in memory, the tick called
  `reload_chat_projection_from_store`. That loaded and AES-GCM-decrypted up to
  5,000 stored messages and 5,000 stored events, rebuilt every message/topic/
  reaction/receipt projection, loaded the outbox, and queried/deleted matching
  rows. Cost was **O(retained messages + retained events + outbox) per tick**.
- **Measured evidence:** the checked-in ignored release harness seeds the
  maximum retained history and measures ten idle ticks.
  - before: p50 **88.050 ms**, average **87.814 ms**, max **88.603 ms**;
  - after: p50 **175.167 µs**, average **183.308 µs**, max **289.792 µs**;
  - p50 improvement: approximately **502×** on this development machine.
- **User impact:** a long-lived Electron account could spend roughly one frame
  budget to several frame budgets decrypting unchanged local history on every
  incoming update, delaying the next Rust action and AppState publication.
- **Smallest corrective boundary:** keep the already-applied
  `ChatProjectionState`. `apply_projection_events` and `append_messages`
  already update it incrementally; startup remains the one full rebuild
  boundary. The existing outbox drain remains responsible for retry/removal,
  and startup still removes stale outbox rows whose accepted message is already
  durable.

Reproduce:

```sh
scripts/with-dev-env cargo test --release -p finitechat-core \
  app_runtime_idle_tick_with_full_projection_history -- --ignored --nocapture
```

### P2 — healthy multi-room sync reloaded the full encrypted MLS Device per room

- **Path:** `CoreState::sync_with_projection` in
  `finitechat/crates/finitechat-core/src/lib.rs`.
- **Trigger:** every normal app or Agent bridge sync across joined rooms.
- **Cost before:** Core loaded, AES-GCM-decrypted, validated, and reconstructed
  the complete `FiniteChatDevice` before each room attempt. The complete Device
  contains every MLS group and OpenMLS storage record, so the cost was
  **O(rooms × total MLS device state) per sync**, even when every room was
  healthy and idle.
- **Why the old boundary existed:** MLS processing can mutate an in-memory
  group before rejecting malformed ciphertext. A failed room must not poison
  later rooms or advance its durable cursor.
- **Measured evidence:** the checked-in ignored release harness creates 20
  real encrypted rooms against the live local HTTP server and measures ten
  idle ticks.
  - before: p50 **30.724 ms**, average **31.818 ms**, max **39.715 ms**;
  - after: p50 **3.895 ms**, average **3.898 ms**, max **4.167 ms**;
  - p50 improvement: approximately **7.9×** on this development machine.
- **User impact:** accounts with many rooms paid repeated full-store decryption
  before AppState or Hermes could observe each update. The multiplier grew with
  both room count and accumulated MLS state.
- **Smallest corrective boundary:** run a healthy room against the current
  in-memory Device. `run_room_sync_tick` already guarantees it saves neither
  Device nor application rows when the room attempt fails. On **every** error,
  Core reloads the last encrypted durable snapshot before classifying the
  failure or continuing to another room. Thus the common healthy path avoids
  reloads while the failed-room rollback boundary remains unchanged.

Reproduce:

```sh
scripts/with-dev-env cargo test --release -p finitechat-core \
  app_runtime_idle_tick_with_many_rooms -- --ignored --nocapture
```

The existing
`app_runtime_agent_bridge_quarantines_broken_room_and_delivers_fresh_room_command`
regression corrupts one room's MLS ciphertext and proves that its in-memory and
durable cursor do not advance, a healthy later room still delivers, and the
runtime stays degraded-ready instead of bricking Chat.

### P3 — resident bridge no-op wakes caused repeated encrypted-history scans

- **Path:** `start_resident_bridge_sync` →
  `signal_bridge_update` → `run_hermes_inbound_stream` /
  `collect_hermes_service_inbound_payload` in
  `finitechat/crates/finitechat-cli/src/hermes.rs`.
- **Trigger before:** every successful resident reconciliation, including the
  ten-second no-change heartbeat.
- **Cost before:** every connected consumer woke on the no-change heartbeat,
  reopened the Agent store, and loaded/decrypted up to 5,000 recent events. A
  Hermes inbox with no cursors loaded the same 5,000-event window twice:
  initialization once and recovery once. The idle cost was
  **O(connected consumers × retained events) every ten seconds**, plus SQLite
  opens and AES-GCM decryption; the first collection doubled that scan.
- **Evidence classification:** statically proven call count, not a production
  measurement. The resident loop signalled unconditionally, both initialization
  and recovery called `load_recent_agent_app_events`, and the initialization
  helper performed its load before checking whether cursors already existed.
- **User impact:** idle Agents and multiple bridge consumers could burn CPU and
  local I/O proportional to chat history even when no user or runtime command
  arrived. Under real traffic, cold Hermes inbox collection also paid two
  identical history reads before delivering the first event.
- **Smallest corrective boundary:**
  - signal consumers only when reconciliation produced joined accounts or
    applied events;
  - return before opening the encrypted store when Hermes cursors already
    exist; and
  - share one loaded event window between cold cursor initialization and event
    recovery.

Deterministic regressions prove that an empty bridge sync is not wake-worthy and
an initialized inbox does not create/open its Agent store.

### P4 — precise sync hints triggered all-room network work

- **Path:** `AppRuntimeCommand::ApplySyncHint` /
  `AgentBridgeApplySyncHint` → `AppRuntimeState::runtime_tick` in
  `finitechat/crates/finitechat-core/src/lib.rs`.
- **Trigger before:** every non-heartbeat Electron hint and every resident
  bridge hint, even though `RoomAdvanced` and `ActivityChanged` carry an exact
  `room_id`.
- **Cost before:** an activity change or durable entry in one room fetched
  ephemeral activity for every connected room and ran the bounded MLS sync for
  every joined room. Cost was **O(rooms)** network requests and crypto work for
  an **O(1)** change.
- **Measured evidence:** the ignored release harness creates 20 real encrypted
  rooms and applies one room's `ActivityChanged` hint ten times.
  - before: p50 **3.843 ms**, average **3.795 ms**, max **4.053 ms**;
  - after: p50 **82.417 µs**, average **84.733 µs**, max **116.333 µs**;
  - p50 improvement: approximately **46.6×** on this development machine.
- **Smallest corrective boundary:** `RoomAdvanced` syncs only the named,
  already-known room; `ActivityChanged` fetches only the named room's
  ephemeral routes and performs no MLS sync. Unknown rooms and inbox hints
  retain the full path.
- **Recovery boundary:** resident startup, explicit poll, inbox processing,
  and heartbeat reconciliation still sweep every room. A regression queues
  messages in two rooms, proves a precise hint consumes only its target and
  leaves the other durable cursor unchanged, restarts Core, then proves the
  heartbeat recovers the unhinted room.

Reproduce:

```sh
scripts/with-dev-env cargo test --release -p finitechat-core \
  app_runtime_activity_hint_with_many_rooms -- --ignored --nocapture
```

### P5 — Electron relayed unchanged revisions into React

- **Path:** `DaemonUpdateRelay.update` in
  `finitechat/apps/electron-chat/electron/daemon-process.cjs`.
- **Trigger before:** duplicate full-state deliveries, including Core
  heartbeat responses and the SSE copy of a state already returned by a daemon
  mutation.
- **Cost before:** every duplicate crossed Electron IPC and replaced the React
  root state, rerunning whole-state selectors and render reconciliation even
  though `AppState.rev` had not changed.
- **Evidence classification:** statically proven delivery count, not a
  production timing measurement.
- **Smallest corrective boundary:** suppress the same safe-integer revision
  only within one daemon generation. A new generation clears the cached
  revision and always delivers its first authoritative full state, even when
  the numeric revision moves backward after restart.

## Correctness and regression boundary

The optimization deliberately keeps these semantics:

- sync hints remain advisory; Core still pulls and validates durable entries;
- precise room/activity hints narrow only unambiguous work;
- startup, inbox, explicit-poll, and heartbeat reconciliation remain full
  recovery boundaries;
- startup still reconstructs the full projection from encrypted durable state;
- a failed room still restores the last durable Device before any later room
  runs;
- outbox retries retain the same idempotency material and restart behavior;
- Hermes inbox durability, per-room cursors, event ordering, and ACK behavior
  are unchanged; and
- the bridge stream's independent wire heartbeat remains active; and
- each Electron daemon generation still delivers its first authoritative full
  state before revision deduplication begins.

Focused tests must cover Core unit/integration behavior, Electron daemon HTTP
behavior, CLI bridge unit/integration behavior, rustfmt, and clippy with
warnings denied.

## Deferred observations

- Full `AppState` clones and full-state SSE payloads remain potentially
  O(selected transcript + media gallery + room metadata). The default
  transcript window is 50 and this audit found no load evidence justifying a
  delta protocol. Measure before changing it.
- A real incoming bridge event still scans the bounded 5,000-event recovery
  window. Removing that scan requires a durable global store position or a
  cursor-aware query that remains correct across rooms; that is a separate
  design, not a tiny fix.
- Finite Private limiter reserve/settle remains the next audit candidate after
  an approved load result and rollout path select it. No claim about limiter
  performance is made here.

## Non-Chat follow-on shortlist

Paul asked for the next scan to leave Chat and focus on server infrastructure.
These are static findings only: none was changed or benchmarked in this patch.
Core Postgres concurrency and the Finite Sites global engine mutex are the
selected next priorities. Specialization and Brain remain useful observations,
but implementation is deferred until Paul syncs with Austin.

### N1 / priority 1 — Core serializes every Postgres operation through one client mutex

- **Path:** `PostgresCoreStore.client: Arc<Mutex<Client>>` in
  `finitecomputer-v2/crates/finite-saas-core/src/store.rs`.
- **Evidence:** the production store opens one Postgres connection and has 64
  mutex-acquisition sites before querying or transacting.
- **Amplification:** one slow report, lease, or transaction prevents unrelated
  heartbeat, runtime, billing, and dashboard queries from reaching Postgres.
  The application therefore has an effective database concurrency of one even
  when Postgres and Tokio have spare capacity.
- **Candidate boundary:** introduce a bounded connection pool; preserve each
  existing transaction on one checked-out connection and keep the current SQL
  and row-locking semantics unchanged.
- **Proof before change:** a mixed concurrent benchmark should include runtime
  heartbeats, runner lease polls, and a deliberately slow independent read.
  The slow read must cease head-of-line blocking the other two, while existing
  lease exclusivity and rollback tests remain green.

### N2 / priority 2 — Finite Sites performs synchronous serving work behind one global mutex

- **Path:** `AppState.engine: Mutex<Engine>` and the static/document branches
  of `serve_path` in `finite-sites/crates/finitesitesd/src/server.rs` and
  `sites.rs`.
- **Evidence:** every request resolves its output through the mutex. Static and
  document requests reacquire it for synchronous SQLite lookups, filesystem
  blob reads, Markdown rendering, and response construction. The repository's
  technical-debt ledger already names the blocking-I/O risk.
- **Amplification:** a cold or large asset for one site can occupy an async
  worker and head-of-line block unrelated sites and control-plane operations.
  Document responses also render and hash their HTML again on every
  revalidation before deciding to return `304`.
- **Candidate boundary:** first separate immutable serving metadata from
  control-plane writes, then move blob reads/rendering off the reactor or
  serve files asynchronously. Active-version publication must remain an atomic
  visibility boundary.
- **Proof before change:** a two-site concurrency test should hold one blob
  read open and prove the other site's small asset and a control-plane read
  complete independently; publish/reload tests must prove no mixed
  manifest/blob version is observable.

### N3 / deferred — each video request starts one `ffmpeg` process per sampled frame

- **Path:** `sample_video_frames` in
  `finitecomputer-v2/crates/finite-specialization-worker/src/lib.rs`.
- **Evidence:** after a separate `ffprobe`, the worker loops over up to four
  timestamps and starts a fresh `ffmpeg` process that reopens and seeks through
  the same staged video for each timestamp.
- **Amplification:** default video normalization pays five process launches and
  up to four independent container decodes before making its model request.
- **Candidate boundary:** extract the same bounded timestamp set in one
  `ffmpeg` invocation, keeping the current semaphore, duration limit, output
  dimensions, PNG validation, timestamp labels, and all-or-nothing failure
  behavior.
- **Proof before change:** compare process count and wall time on short,
  long-GOP, variable-frame-rate, and near-duration-boundary fixtures; decoded
  frame count and labels must match the current implementation.
- **Coordination boundary:** do not implement or land this change before Paul
  syncs with Austin on the specialization worker.

### N4 / deferred — Brain's replay cache scans all live authorization events per request

- **Path:** `enforce_auth_replay_cache` in
  `finite-brain/crates/finite-brain-server/src/protected_routes.rs`.
- **Evidence:** every protected request takes one global mutex and calls
  `BTreeMap::retain` over the complete replay window before its point lookup
  and insert.
- **Amplification:** CPU under a steady authenticated rate grows with both
  request rate and the number of events in the auth-skew window, while the
  mutex serializes concurrent callers.
- **Candidate boundary:** pair the event-id map with expiry-ordered eviction,
  retaining exact replay rejection and bounded expiry semantics.
- **Proof before change:** benchmark a full live window under concurrency and
  retain regressions for same-event rejection, expiry acceptance, clock
  saturation, and bounded memory.
- **Coordination boundary:** do not implement or land this change before Paul
  syncs with Austin on Brain.

## Live acceptance remaining

No production mutation was performed. After review and merge, the normal
Finite Chat/Electron and Runtime release processes still own:

1. packaged Electron smoke with a long retained history;
2. one Runtime canary showing idle bridge CPU/I/O remains flat while the
   heartbeat stream stays connected; and
3. normal component release and rollout approval.
