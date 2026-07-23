# Rust hot-path performance audit: Core, Electron, and user bridge

Date: 2026-07-23

Status: **IMPLEMENTED LOCALLY — NOT RELEASED OR DEPLOYED**

Scope: plan 10 from `docs/next-work-plans-2026-07-23.md`

## Decision and boundary

Paul selected the one-day static-audit option and prioritized the shippable
Finite Chat path before the harder-to-roll-out Finite Private limiter:

1. `finitechat-core` as exercised by the Electron daemon; and
2. the resident `finitechat-cli` Hermes/user bridge.

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

### P2 — resident bridge no-op wakes caused repeated encrypted-history scans

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

## Correctness and regression boundary

The optimization deliberately keeps these semantics:

- sync hints remain advisory; Core still pulls and validates durable entries;
- heartbeat reconciliation remains in place for missed-hint recovery;
- startup still reconstructs the full projection from encrypted durable state;
- outbox retries retain the same idempotency material and restart behavior;
- Hermes inbox durability, per-room cursors, event ordering, and ACK behavior
  are unchanged; and
- the bridge stream's independent wire heartbeat remains active.

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

## Live acceptance remaining

No production mutation was performed. After review and merge, the normal
Finite Chat/Electron and Runtime release processes still own:

1. packaged Electron smoke with a long retained history;
2. one Runtime canary showing idle bridge CPU/I/O remains flat while the
   heartbeat stream stays connected; and
3. normal component release and rollout approval.
