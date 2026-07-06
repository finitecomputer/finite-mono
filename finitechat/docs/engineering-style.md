# Engineering Style

This repo borrows the parts of Tiger Style that fit Finite Chat's risk profile:
do the production-shaped thing early, keep control flow explicit, and make
invariants executable.

Reference: https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md

## Local Rules

- Authoritative server state must use schema, constraints, and transactions.
  Do not add JSON blobs for state that the room server must query, lock,
  constrain, or recover.
- JSON is allowed for wire DTOs, encrypted application payloads, and bounded
  typed replay values such as idempotency responses.
- Store APIs must not hide database or corruption errors behind `Option`.
- Use typed error enums for crate boundaries. Do not use `anyhow` in protocol,
  engine, store, or simulation code; callers should be able to match errors
  without parsing strings.
- Every mutation that changes room state must have a test covering the positive
  path and at least one negative/replay path.
- Prefer explicit branch structure for validation. Avoid clever `Option` or
  iterator control flow where the code is enforcing safety properties.
- Persist replayable errors intentionally. Rejected mutations admitted under an
  idempotency key are part of durable server state.
- Keep fake-MLS tests honest: if a behavior will later depend on OpenMLS, mark
  the fake gate and keep the server-side invariant separate from crypto truth.
- Do not use recursion in protocol, storage, sync, or client state-machine
  code. Finite Chat state machines should be iterative and visibly bounded.
- Put explicit limits on loops, batches, payloads, fanout, sync windows, and
  retry work. If a loop is intentionally unbounded because it consumes a bounded
  iterator from SQLite or MLS, say why near the loop.
- Prefer explicitly sized domain types at boundaries. Use `u32` or `u64` for
  protocol numbers, sequence numbers, counters, and WASM-facing values; avoid
  exposing `usize` outside local indexing.
- Declare variables at the smallest useful scope.
- State invariants positively. Prefer `if value_is_valid { ... } else { ... }`
  over negated forms for safety checks.
- Centralize control flow and state mutation. Parent functions should decide
  what happens; helpers should either validate, compute, or persist one clear
  change.
- Keep compiler warnings at the strictest practical setting for this repo:
  `cargo clippy --all-targets -- -D warnings`.
- Do not do irreversible work directly in reaction to external events. Inbound
  HTTP, relay, push, or gateway events should be validated, persisted, and then
  interpreted from Finite Chat's own ordered state.
- Always explain why for surprising constraints, explicit limits, schema
  choices, and security-relevant branches.
- Pass important options explicitly at call sites instead of relying on library
  defaults.
- Distinguish the control plane from the data plane. Room creation,
  KeyPackages, Welcomes, link sessions, repair, and idempotency are control
  plane; encrypted application messages and sync are data plane.
- Keep hot loops standalone with primitive arguments when they become visible
  in profiles or performance sketches.
- Treat cache invalidation as a protocol decision. Any derived cache must name
  its source of truth, invalidation trigger, and stale-read behavior.
- Audit every dependency addition before adding it. Prefer the standard library,
  existing workspace dependencies, or a small Rust crate over shell/Python
  tooling. New scripts should be Rust binaries or tests unless a non-Rust tool
  is clearly the better fit.
- Document tolerated technical debt in `docs/technical-debt-ledger.md` before
  relying on it. Each item needs an observed source, risk, first proof, and
  delete condition. A shortcut without a delete condition is not accepted debt;
  it is unfinished design.
- Prefer hard cuts over compatibility shadow paths. Do not keep duplicate
  old/new APIs, fallbacks, launch/test-only shims, or parallel implementations
  merely to preserve pre-release tests or harnesses. Rewrite tests and callers
  to the new shape unless the user explicitly asks for backwards compatibility
  for real shipped users.
- Before the first user release, prefer hard cuts over compatibility work. Do
  not add migrations, legacy shims, or make-work patches to preserve pre-release
  dev/test state; rename or delete the old shape and reset local stores/tests
  unless real user data exists.
- Do not build product harnesses around known-wrong pre-release shapes. Hard-cut
  the domain model first, then write product proof against the model users will
  actually see.
- Dev/test product-state cleanup must go through one documented reset command
  that deletes whole explicit roots. Do not add ad hoc cleanup scripts, targeted
  SQL cleanup, row mutation, or partial config clearing.
- Before the first user release, do not add automatic diagnostic upload or
  telemetry to compensate for weak debugging. Use hidden Developer diagnostics
  with explicit local copy/share export.
- Product E2E should be simulator-first for deterministic automation and fast
  hard cuts. Before first users, repeat the same product matrix on a physical
  phone; do not replace it with a looser device smoke test.
- Product offline tests must preserve runtime identity and the configured
  server URL, then toggle reachability. Wrong-server-URL tests belong to
  diagnostics coverage, not the offline-send matrix.
- In planning/grilling sessions, ask the user for product semantics,
  user-visible behavior, privacy posture, and irreversible tradeoffs. Engineers
  own command-line flags, path fences, test mechanics, and implementation
  guardrails unless those details change product behavior.

## Assert Boundary

Use handled errors for client mistakes and operating conditions:

- wrong epoch;
- stale KeyPackage;
- duplicate idempotency key with a different body;
- missing Welcome;
- room needing repair.

Use assertions or corruption errors for internal contradictions:

- room `last_seq` does not match persisted log rows;
- membership table key does not match the stored device;
- a Welcome ack has no corresponding inactive membership interval;
- persisted idempotency response kind disagrees with its operation.

Assertion policy:

- Target an average of two invariant checks per nontrivial function: one near
  ingress for the assumptions being consumed, and one near egress for the state
  or value being produced.
- For public APIs and mutation functions, entry checks should validate caller
  input, current state, or both. Exit checks should validate the committed state
  or returned value.
- Pair important assertions. Check data before writing it and again after
  reading it back from storage.
- Split compound assertions or corruption checks so failures identify the exact
  broken invariant.
- Pure decode/encode helpers and tiny type constructors may rely on type
  exhaustiveness instead of mechanical assertion count, but they should still
  reject impossible external values explicitly.
- Use handled errors for expected bad input. Use `debug_assert!`, `assert!`, or
  `StoreError::CorruptState` for internal contradictions where continuing would
  make state less trustworthy.

The goal is not to maximize asserts mechanically. The goal is to keep invalid
states from becoming ordinary states and to make the expected state space easy
to review.

## Test Shape

- Every state-machine transition gets valid and invalid tests.
- Every idempotent mutation gets success replay, rejected replay, and
  conflicting-body tests.
- Every storage invariant gets a restart test.
- Add fuzz/property tests before OpenMLS or Postgres canary work changes parser,
  membership, idempotency, or sync cursor logic.

## Allocation Shape

Rust will allocate; the rule is to make allocations visible and bounded.

- Allocate request/session scratch buffers near the cycle boundary, not deep in
  inner validation helpers.
- Keep sync result limits explicit so vectors do not grow with room history.
- Prefer borrowing slices in hot helpers.
- For WASM-facing client code, treat allocations as part of the API budget and
  document where they occur.

## Performance Sketch

Finite Chat should be network- and disk-bound before it is CPU-bound.

Initial sniff-test target for one room-server process:

- direct room Commit: one SQLite/Postgres transaction, one log row, a small
  membership delta, zero plaintext inspection;
- application message append: one transaction touching room head, log entry,
  idempotency record, and push outbox;
- sync: bounded page read by `(room_id, seq)` with opaque payload bytes.

If a local SQLite dev server cannot handle hundreds of small appends per second
on a laptop, or if a Postgres canary design cannot plausibly handle thousands
of appends per second before network/push fanout dominates, assume the design
has accidental complexity until proven otherwise.

Optimize in this order:

1. network: batch sync, cap payload sizes, keep push opaque;
2. disk: single transaction per mutation, indexed cursor reads, no full-room
   rewrites;
3. memory: bounded pages and explicit fanout limits;
4. CPU: standalone hot loops only after the first three are sound.
