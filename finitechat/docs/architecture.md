# Finite Chat Architecture

Date: 2026-06-11. This is the orientation document: what Finite Chat is, how
its pieces fit, and why it is shaped this way. Deep dives live in the
documents it links; domain vocabulary lives in `CONTEXT.md` and
`docs/protocol-glossary.md`.

## 1. What it is

Finite Chat is an end-to-end-encrypted chat and command transport for the
finite computer product: humans talking to humans and to their agent
runtimes, across multiple devices, with the server unable to read anything.
Its design goal, in the words this repo uses everywhere, is **chat that
always works**: durable, crash-atomic, exactly-once, membership-correct — on
flaky networks, across crashes of either side, at the scale of hundreds of
active users with dozens of long chats each.

The architecture in one sentence: **clients own all cryptography and apply
only what they observe in a server-ordered log; the server owns ordering,
durability, and membership bookkeeping over bytes it cannot read.**

## 2. The shape

```
┌────────────────────────────────────────────────────────────────────┐
│ Devices (CLI / Electron / iOS — one FiniteChatDevice each)         │
│  OpenMLS group state · encrypted SQLite snapshot store             │
│  sync worker · link-fanout worker                                  │
└───────────────▲────────────────────────────────────────────────────┘
                │ typed HTTP routes (JSON DTOs, finitechat-http)
┌───────────────┴────────────────────────────────────────────────────┐
│ finitechat-server (Axum, single ordering authority per room)       │
│  typed routes: /commits /events /sync/group /sync/wait /invites/* │
│     /welcomes/* /rooms/* /key-packages/* /account-rooms/* ...      │
│  projections: membership intervals · admins · departed · directory │
│               KeyPackage leases · delivery effects · idempotency   │
│  volatile lanes: /activities (typing) · /devices/liveness          │
├────────────────────────────────────────────────────────────────────┤
│ finitechat-delivery                                                │
│  ordered per-room log · per-device inbox · epoch admission ·       │
│  digest dedup · KeyPackage consume-once · dry-run check_publish    │
├────────────────────────────────────────────────────────────────────┤
│ SQLite (WAL): operation log · state snapshot · projection tables   │
└────────────────────────────────────────────────────────────────────┘
```

Crate map: `finitechat-proto` (wire DTOs, payload kinds, limits, projections
shared by both sides), `finitechat-http` (route request/response types),
`finitechat-server` (the durable server), `finitechat-client` (device state
machine + workers + store), `finitechat-mls` (OpenMLS helpers + device
credentials), `finitechat-blob` (encrypted attachment references),
`finitechat-hermes` (LLM-gateway bridge DTOs), `finitechat-cli` (route
client), `finitechat-transport` (shared transport value types), and
`finitechat-delivery` (the ordered delivery service and conformance suite).

## 3. Identity and cryptography

- An **account** is a Nostr keypair. A **device** is
  `DeviceRef { account_id, device_id }` carrying a
  `FiniteDeviceCredentialV1`: the device's MLS signature key signed by the
  account key, with validity bounds (90-day credentials, renewal via
  self-update commit — ADR 0003 §7). Clients verify these credentials at
  every KeyPackage parse, commit merge, and Welcome activation; **the server
  is never an identity authority** (ADR 0001).
- The account key is the **shared Finite identity** (Finite Identity
  Contract v1, the `finite-identity` crate): one key per user/agent at
  `$FINITE_HOME/identity/identity.json` (hosted runtimes pin
  `FINITE_HOME=/data/agent`), else `~/.finite/identity/identity.json`,
  minted under an exclusive lock by whichever Finite tool runs first and
  found by all others (fsite, fbrain, hosted runtimes). CLI/agent flows load
  it once into memory at open (`FiniteChatRuntime::open` with no explicit
  secret; `finitechat auth status`/`auth import` are the CLI surface); the
  secret is never copied into finitechat's own stores — the legacy
  `account-secret.hex` / `identity.env` / `agent.nsec` locations are
  hard-cut and never read. iOS keeps its keychain identity and passes the
  secret explicitly (the shared file does not apply inside an app sandbox).
- Everything the client needs at rest is **derived** from that account
  secret at runtime via HKDF domain separation
  (`NostrSecretKey::derive_secret_32`), e.g. the client-store encryption key
  under `finitechat.client-store-key.v1` per device id. Derivation domains
  and downstream state formats are pinned by test vectors and do not change
  when the secret arrives via the shared file.
- A **Room** is one OpenMLS group plus one server-ordered delivery log.
  Group key agreement is plain OpenMLS driven directly by the client: commits
  rotate epochs, Welcomes carry group secrets to new devices, application
  messages are MLS ciphertexts the server stores as opaque payloads.
- Client state at rest is a single versioned snapshot (all rooms, cursors,
  pending work, OpenMLS storage records) encrypted with AES-256-GCM under a
  key derived from the account secret.

## 4. The ordered log and the trust split

ADR 0001 is the founding decision: MLS needs every member to process
commits in the same order, and eventually-consistent relays can't promise
that — so one server per room is the *ordering* authority. The trust split
is strict:

**The server is trusted for:** sequence numbers (dense, never reused),
durability (a 200 means fsynced), at-most-once commit admission per epoch,
Welcome release only after the durable accepted commit, and
membership-interval bookkeeping used for delivery filtering and push
routing.

**The server is never trusted for:** identity, message content (always MLS
ciphertext), or cryptographic membership truth. The client's
**pending-commit merge rule** is the keystone: a device does not act on its
own commit because the server said "accepted" — it merges only after pulling
that commit back out of the ordered log, exactly as every other member will.
If an accepted commit is invalid or disagrees with its declared membership
delta, clients report it and the room fails closed into `NeedsRepair`.

Everything is **pull-based**: stream-style hints may only mark "a pull is
needed"; only ordered `/sync/group` pages advance client state. This is why
unreliable push delivery can never corrupt anyone.

## 5. The Delivery Core

The bottom layer is `finitechat-delivery`. It is deliberately tiny: it
sequences opaque `TransportMessage` bytes per group, queues Welcomes per
device inbox, enforces one admitted commit per source epoch, dedups by content
digest, leases KeyPackages consume-once, and exports two things that shape
everything above it:

- **`check_publish`** — a dry-run admission check whose `Fresh` variant
  carries the exact receipt the real publish will return. This is what lets
  the durable server persist before applying (see §8) instead of cloning
  state for rollback.
- **A conformance suite** — executable contract checks (ordering, replay,
  admission, consume-once, restart survival) that any implementation can
  run. `finitechat-server` passes it over SQLite, restart checks included
  (`crates/finitechat-server/tests/http_conformance.rs`).

Typed rooms accept only typed routes; raw delivery-contract publishes exist
only as an internal conformance boundary.

## 6. The server: typed routes over opaque bytes

`finitechat-server` wraps the core with finite's protocol. Every route
operates on metadata the caller *declares*; payloads stay ciphertext.

**Membership lifecycle.** Typed bootstrap creates a room's membership
projection (creator active, protocol slots). Typed `/commits` carry a declared
`MembershipDeltaV1` which the server validates for relay invariants (epochs,
active sender, duplicates, caps, and structural shape) but not for social room
authority. Accepted commits atomically: append the commit to the log, consume
the claimed KeyPackages, release the derived Welcomes to recipient inboxes,
and update the projections. The projection records
membership as **intervals** (`[start_seq, end_seq)` per device), which is
what makes requester-filtered sync work: a new device's history starts at
its add-commit; a removed or departed device can sync through its exit seq
and nothing after.

**KeyPackage economy.** Devices publish one-time "add me" tokens; the
server runs an available → claimed → consumed lease lifecycle with expiry
and reclaim, per-device caps, and exact publication-retry replay — so a
crashed inviter never permanently burns anyone's tokens.

**Welcome lifecycle.** Claim, then a single idempotent activate-ack that
promotes the device from pending to active (ADR 0004 §6: a failed
activation just stays pending and retryable). The release coupling — no
Welcome before its durable accepted commit — is sacrosanct.

**Leave (ADR 0003 §3).** `/rooms/leave` closes all the account's intervals
immediately (server-recognized, whole-account); a `departed` marker tells
member workers that the MLS removal commit is still owed, and that commit
completes the leave. The last admin must hand off before leaving a populated
room.

**Delivery effects.** Every typed event carries a required
`ApplicationDeliveryPolicy` (push / unread / command-inbox), recorded
crash-atomically with the event. This is the push engine: the server knows
*who to wake and how* per message without decrypting anything, because the
sender declares it. `/push-tokens` holds the device tokens (ADR 0003 §5);
the wake payload is exactly `{room_id, seq}` — enough for a notification
extension to pull and decrypt locally, leaking nothing.

**Volatile lanes.** Typing-indicator-style `/activities` (bounded, scoped
per room or conversation, never durable) and `/devices/liveness` heartbeats
deliberately never touch the ordered log.

**Protocol slots (ADR 0003 §1).** Rooms carry
`RoomProtocol { protocol_version, required_capabilities }` from bootstrap;
out-of-range versions get `426 Upgrade Required`. Clients skip-and-advance
unknown application kinds and fail closed on unparseable commit-kind
entries. Reserved application kinds include the agent stream lane
(ADR 0003 §6): durable `StreamStartV1`/`StreamFinishV1` anchors with a
transcript hash, epoch-pinned, with transient deltas never entering the log.

## 7. The client: a crash-safe convergent device

`FiniteChatDevice` holds per-room MLS groups, per-room cursors, and queues
of pending work (welcomes, acks, KeyPackage uploads, link fanouts) — all of
it exportable as one encrypted snapshot, so the process can die at any
instant and resume exactly.

Two workers drive everything:

- **`run_runtime_sync_tick`** — replenish KeyPackages toward target, claim
  and activate Welcomes, ack them, pull ordered pages per room, decrypt and
  apply each entry once (seq-guarded), persisting once per page.
- **`run_link_fanout_tick`** — the multi-device machine: discover the
  account's rooms via the directory, claim a KeyPackage for the new device
  per room, prepare and submit the add commit, retry lost responses with the
  same idempotency keys, re-prepare at the next epoch if a concurrent commit
  won the race, and complete when the new device activates each Welcome.
  Resume state lives in the client's own durable store; the server holds no
  fanout state.

Account recovery is deliberately not a separate mechanism: restore the
account secret, link a fresh device — recovery *is* the link flow
(ADR 0003 §7).

## 8. Reliability engineering

The patterns that make "always works" an engineering property rather than a
slogan:

- **Check → persist → apply (server).** Every mutating route validates
  read-only against live state (using `check_publish`'s predicted receipt),
  writes all durable rows in one SQLite transaction, then applies to memory
  infallibly. A crash before the persist changes nothing; a crash after it
  is repaired by replay. Trigger-injected crash-matrix tests pin this at
  every write boundary.
- **Idempotency everywhere.** Every publish carries a key; exact retries
  replay the original receipt byte-for-byte even across restarts;
  conflicting reuse is rejected. Client retry loops are therefore always
  safe.
- **Snapshot + tail startup.** The server persists an operation log (the
  audit trail and replay source) but boots from a periodic state snapshot
  plus the log tail — full-history replay is a rare recovery action, not
  day-to-day mechanics (standing constraint, ADR 0003). The snapshot is
  proven authoritative for its prefix, which is the contract the future
  retention/horizon compaction (ADR 0003 §4: seqs never reused, sync below
  the horizon returns a marker, expired content is server-deleted) plugs
  into.
- **Page-batched client persistence.** Entries apply in memory and persist
  once per page; crash recovery replays at most one page, idempotent by the
  seq guard.
- **The test suite is the asset.** ~200 Rust tests across route, SQLite
  restart, crash matrix, runtime-worker, live-server, and process-binary
  layers, plus the upstream conformance suite. Every optimization and
  protocol change in this repo's history landed against that suite green.

## 9. Performance characteristics

Measured on the repo's benchmark harness (`perf_baseline` tests, release
mode; history in `docs/perf-log.md`):

| Operation | Result | Design reason |
| --- | --- | --- |
| Durable publish (loaded server) | p50 ~46–50 µs, p99 ~100 µs, **flat with state size** | check/persist/apply does no O(state) work; cost is the WAL fsync |
| Sync page (100 entries, any depth) | ~6 µs | `partition_point` over the seq-sorted log |
| Client apply per entry (decrypt + persist) | ~62 µs | page-batched saves, persistent connection |
| Startup | snapshot + tail replay | full replay only for never-snapshotted stores |
| Message latency after send | ~1 RTT | `/sync/wait` long-poll wake hints replace sleep loops |
| Agent pairing e2e (invite → PIN → admitted → 4 round trips) | < 0.5 s | counts-keyed wake predicates; poll returns on admitted joins |

Capacity is configured for the current phase (hundreds of users, dozens of
long chats): 65,536 rooms, 262,144 entries per room
(`finite_delivery_limits()`), applied before replay so reopening never trips
a smaller cap than the one it was written under. The known scaling wall is
the in-memory full-history mirror (~1 KB/entry RAM); the snapshot/horizon
machinery is its designed exit.

## 10. Governance and document map

Decisions are ADRs; vocabulary is the glossary; running work is logged.

- `docs/adr/0001` — server-ordered delivery (the founding trust split)
- `docs/adr/0002` — finite-owned server + Hermes bridge
- `docs/adr/0003` — protocol v1 hardening: versioning, admin, leave,
  retention, push, streams, recovery (all implemented or reserved)
- `docs/adr/0004` — surface simplifications: typed-only rooms, one event
  route, claim+activate welcomes, direct rooms dissolved, interop only if
  free
- `docs/adr/0005` — home servers and room servers: the sharded target
  topology, the migration principles, and the route taxonomy
- `docs/adr/0006` — the agent invite flow: invite codes (URL/QR), the
  rotating challenge PIN verified before the MLS add, invite sessions,
  and the hermes bridge onboarding surface
- `CONTEXT.md` / `docs/protocol-glossary.md` — domain language and the
  user-promise behind each mechanism
- `docs/perf-plan.md` / `docs/perf-log.md` — performance program and ledger
- `docs/feature-audit-marmot-pika.md` — what adjacent projects taught us

## 11. Deliberately not built yet

Retention/horizon compaction (cursor semantics decided, snapshot proven as
its substrate), idempotency-record expiry against the same horizon, the
pusher daemon (token routes and wake contract exist), the agent stream
delta transport (durable anchors reserved), media transport beyond the
blob-reference design, calls, and the home-server/room-server split
(ADR 0005 — target topology and guardrails recorded; today's deployment is
the degenerate case of one server playing both roles). Each room keeps a
single ordering authority by design at every phase; what eventually shards
is *which* server holds that authority, per room.
