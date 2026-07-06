# Daemon Survival Testing

Status: planned strategy.

First implementation checkpoint: `crates/finitechat-sim/tests/daemon_survival.rs`
now proves the pure daemon state machine with a fake runtime adapter. It covers
Hermes absent at startup, Hermes hung during sync, attachment download while the
gateway is down, restart-after-ledger-write, and a deterministic bounded fuzzer
for user messages, restart commands, gateway state changes, daemon restarts, and
crash points. The broader layers below remain the production hardening plan.

Finite Chat is the last-resort control surface for an agent runtime. Hermes can
crash, hang, misconfigure itself, or lose inference access. Finite Chat must
still let the user observe runtime state, sync ordered room events, receive
non-notifying status snapshots, and send a small allowlisted set of recovery
commands as long as the host is online and the Finite Chat device store is
usable.

This is a stricter bar than "chat works when Hermes works." The daemon must
fail independently from Hermes, inference, and bridge adapters.

## Survival Boundary

The daemon-owned survival surface is:

- room sync and local projection;
- encrypted client/device state;
- runtime liveness heartbeat;
- `runtime.state.snapshot` publication;
- durable command request/result/cancel processing;
- command ledger and idempotent recovery;
- minimal runtime recovery commands, especially Hermes gateway restart;
- attachment/blob bookkeeping that does not require Hermes to be healthy.

The daemon must not depend on Hermes or inference for:

- receiving room events;
- publishing `runtime.gateway` or `runtime.inference` state;
- accepting and recording a recovery command;
- restarting the Hermes gateway;
- reporting that an agent reply cannot currently be produced.

If Hermes is down, ordinary assistant replies may fail. The Finite Chat daemon
should still preserve user messages, project status, and recovery commands. A
dead agent is a runtime state, not a dead chat transport.

## Core Invariants

- External events only trigger sync. They never execute work directly.
- Sync, state projection, and command ledger writes are independent of Hermes
  process health.
- Runtime state snapshots are produced by daemon observation, not by asking
  Hermes to tell us whether Hermes is alive.
- Recovery commands are typed, allowlisted, and executable without the broken
  bridge they are meant to repair.
- Every command that mutates runtime state records the request before execution
  and records a terminal result or retryable pending state after restart.
- A failed or timed-out inference request cannot block room sync, heartbeat,
  state snapshots, KeyPackage replenishment, Welcome ack, or command cancellation.
- User-facing status distinguishes host offline, daemon unavailable, gateway
  down, inference degraded, and stale encrypted state.
- Stale snapshots are rendered as stale. They do not imply the host is offline
  while heartbeat is fresh.
- Recovery loops are bounded. A broken gateway must not create unbounded
  restart attempts, command results, activity refreshes, or log growth.

## Harness Shape

Use Rust test harnesses before shell scripts:

1. `finitechat-client` unit/integration tests own the daemon state-machine
   proofs: sync tick, command ledger, snapshots, crash points, and idempotent
   recovery.
2. `finitechat-store` tests own server durability: ordered events, push policy,
   idempotency, snapshot projection inputs, and retry behavior across reopen.
3. A small fake runtime adapter crate or test module should model Hermes,
   inference, and bridge health with deterministic fault injection.
4. finitecomputer local-loop tests should only come after those state-machine
   proofs pass. They prove wiring, not the first source of correctness.

The fake runtime adapter should expose primitive operations instead of shelling
out:

- `observe_gateway()`;
- `restart_gateway(command_id)`;
- `poll_agent_messages()`;
- `submit_user_message()`;
- `run_inference()`;
- `observe_connections()`.

Each operation should support `Ok`, `Down`, `Hung`, `Timeout`, `InvalidOutput`,
`PermissionDenied`, and `RestartedMidCall` outcomes. Tests should drive these
outcomes deterministically and assert daemon state after every tick.

## Test Layers

### Layer 1: Pure Daemon State Machine

No real Hermes, no browser, no subprocesses.

Prove:

- inbound room events persist before command interpretation;
- command requests enter a local ledger before execution;
- gateway restart command can run when gateway status is `down`;
- timeout leaves retryable or terminal state, never half-applied state;
- state snapshots update by `(room, source device, state_key, revision)`;
- state snapshots are `push_policy = never` and do not create unread or inbox
  work;
- daemon restart resumes sync, pending command results, pending Welcome acks,
  and pending KeyPackage uploads.

### Layer 2: Fault Matrix With Deterministic Runtime Adapter

Run the same command/status scenarios under a bounded matrix:

| Fault | Expected Behavior |
| --- | --- |
| Hermes absent at daemon startup | daemon starts, syncs, publishes `runtime.gateway=down`, accepts restart command |
| Hermes crashes after command ledger write | command is retried or terminally failed after restart; ledger remains coherent |
| Hermes hangs during message poll | sync and snapshots continue; poll timeout is recorded; no command queue stall |
| Hermes returns invalid JSON | daemon marks gateway degraded and preserves raw diagnostic summary within limits |
| Inference provider times out | user message is durable; assistant result fails or remains pending by policy; sync continues |
| Inference rate-limited | runtime state publishes degraded provider status; no restart loop |
| Gateway restart succeeds | command result and post-mutation `runtime.gateway` snapshot converge |
| Gateway restart fails | command result is terminal failure; state snapshot remains down/degraded |
| Attachment download while gateway is down | blob retrieval and hash verification succeed without Hermes or gateway health |
| Room server temporarily unreachable | daemon keeps local state, retries bounded sync, heartbeat/status reflect relay trouble |
| SQLite busy or interrupted | no partial command execution before durable ledger write; retry converges |

### Layer 3: Crash Points

Inject daemon crashes at every edge that crosses durable state:

- after receiving an SSE/push hint, before sync;
- after fetching room page, before storing applied cursor;
- after storing applied cursor, before interpreting command;
- after recording command request, before scheduling work;
- after performing gateway restart, before recording result;
- after recording command result, before publishing state snapshot;
- after publishing state snapshot, before clearing local pending work;
- after claiming Welcome, before activation;
- after activation, before server ack;
- after KeyPackage generation, before upload response is observed.

Every crash test should restart from encrypted client SQLite and prove either:

- exact retry of the same idempotent operation; or
- no retry because the terminal result is already durable.

### Layer 4: Local Loop With Broken Hermes

Once the Rust harness proves the state machines, finitecomputer should add a
local-loop scenario that starts:

```text
dashboard -> finited relay -> finitec/Finite Chat daemon -> broken Hermes stub
```

Acceptance checks:

- dashboard loads topics and latest runtime state;
- heartbeat is fresh while `runtime.gateway` is down;
- sending a user message creates durable chat state but does not pretend an
  assistant replied;
- "restart gateway" sends a typed command, not an ad hoc dashboard action;
- restart result is visible without push notification;
- attachment preview/download works through the blob store while the gateway is
  down;
- after Hermes stub becomes healthy, the daemon resumes ordinary gateway flow
  without resetting room/device state.

### Layer 5: Soak And Fuzz

Add a deterministic survival fuzzer after the first harness tests exist. It
should generate bounded sequences of:

- user messages;
- command requests, cancels, duplicate retries, and conflicting retries;
- gateway up/down/hung transitions;
- inference up/down/timeout transitions;
- daemon restarts;
- room-server disconnect/reconnect;
- SSE drop/duplicate/reorder hints;
- state snapshot refreshes and expiries.

The oracle is not "the agent answered." The oracle is:

- room sequence never regresses;
- applied cursor never regresses;
- command ledger has one terminal state per request;
- duplicate retries replay the same result;
- gateway/inference failures do not block sync;
- no recovery command executes without a durable request;
- no status snapshot creates push, unread, or command inbox work;
- bounded queues stay within protocol limits.

## Minimal Canary Gate

Before finitecomputer canary enables Finite Chat as the primary chat path, these
survival cases should pass:

- Hermes down at startup, dashboard still opens and shows gateway down;
- inference unavailable, user message persists and daemon remains responsive;
- gateway restart command works while Hermes is down;
- daemon restarts while gateway restart is pending and converges;
- room server SSE drops while Hermes is down and pull sync repairs state;
- stale runtime state snapshot is shown as stale while heartbeat remains fresh;
- broken Hermes output cannot corrupt the encrypted chat projection;
- attachment download does not require Hermes or the gateway to be healthy;
- all recovery commands are allowlisted and idempotent.

## Debt Trigger

If any finitecomputer feature requires Hermes to be healthy before the user can
observe status, send a recovery command, or read existing room state, add a row
to `docs/technical-debt-ledger.md` before merging it.
