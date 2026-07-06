# Marmot HTTP Transport Investigation

Date: 2026-06-09
Branch: `marmot-investigation`

Primary source:

- <https://github.com/marmot-protocol/darkmatter/blob/claude/http-server-transport-adapter-tBKl7/docs/marmot-architecture/overview/http-server-transport.md>

Related local sources:

- `README.md`
- `docs/protocol-v1.md`
- `docs/adr/0001-server-ordered-mls-delivery-service.md`
- `docs/implementation-plan.md`
- `docs/scenario-coverage.md`
- `docs/storage-plan.md`

## Short Answer

Do not adopt Marmot wholesale as a replacement for the Finite Chat protocol.

Marmot's proposed HTTP single-server transport is directionally close to Finite
Chat: it puts durable queues, KeyPackage handout, and same-epoch Commit
admission behind one server while keeping Nostr-rooted identity and MLS/CGKA
state client-verified. That validates our core direction.

It does not yet go far enough to replace our v1 protocol surface. The largest
gap is multi-device. A server-hosted KeyPackage directory is necessary, but not
sufficient: Finite Chat needs explicit device identity, account-level fanout,
later-device linking into existing rooms, Welcome claim/ack recovery, revocation,
bounded inventories, and crash-safe retry semantics.

The practical path is to treat Marmot as prior art for MLS/Nostr identity,
KeyPackage validation, and maybe transport/peeler factoring. Keep Finite Chat's
server-ordered Delivery Service contract as the v1 product protocol.

## Where Marmot Aligns

- Nostr account key remains the user identity root.
- MLS/CGKA remains the group cryptographic state machine.
- The server is not identity authority; clients validate credentials and
  KeyPackages.
- A single server can provide durable queues, delivery cursors, and KeyPackage
  storage.
- Same-epoch Commit races are handled by server admission: one Commit wins, the
  loser re-commits from the newer epoch.
- The server cannot read or forge MLS group state.
- The design accepts a metadata and availability tradeoff versus relay-style
  redundancy.

These are already the key Finite Chat v1 choices.

## Core Difference: Who Owns Ordering Truth

Finite Chat v1 has one authoritative room sequence. Clients process server log
entries in sequence order, and server sequence is the application ordering
boundary. Clients still validate cryptography and application policy, but they
do not run a branch-selection convergence protocol for normal operation.

Marmot's HTTP note is more conservative about server authority. The server may
sequence delivery and admit only one Commit per epoch, but clients still treat
server order as delivery evidence, not protocol truth. Marmot keeps its
distributed convergence model: client-side branch selection remains
content-derived and independent of transport arrival order or server sequence.

Tradeoff:

- Marmot preserves a transport-agnostic trust model and is more robust against a
  buggy or malicious server trying to define CGKA state.
- Finite Chat is simpler and fits the product better: one server-ordered log is
  the transcript, command ledger, status projection, and retry boundary.
- If we adopted Marmot's convergence-first model, we would need to accept more
  client complexity, retained branch material, possible invalidation/reorg
  behavior, and looser coupling between server log order and application state.

## Multi-Device Assessment

Your fear is justified: "the server hosts KeyPackages" does not by itself solve
multi-device.

Marmot's HTTP note says the server should publish a KeyPackage pool,
fetch-and-consume one-time KeyPackages, serve a last-resort KeyPackage, and track
consumption without vouching identity. That is the classic MLS delivery-service
primitive.

Finite Chat needs more:

- each application install is a `Device`, and every device is its own MLS leaf;
- KeyPackages are inventory-bounded per device, not just per account;
- account fanout claims at most one available KeyPackage per registered device;
- a multi-device invite may add several devices for one account in one Commit;
- each added device gets a Welcome, can sync from its add Commit, and cannot send
  until its own Welcome is activated and acked;
- later-linked devices join all existing rooms through normal add-device Commits;
- link fanout must survive response loss, restarts, same-epoch Commit loss, and
  partial room failure;
- revoked devices cannot upload or claim KeyPackages, claim Welcomes, send, or be
  added again;
- removed devices can sync through their removal Commit but cannot decrypt later
  ciphertext.

Marmot could support these if its HTTP profile grows a Finite-like device
registry, account-room discovery, per-device KeyPackage inventory, deterministic
account fanout, Welcome lifecycle, and replayable idempotency. The current note
does not specify those behaviors.

## Other Tradeoffs

### Metadata

Marmot's single-server profile openly trades away relay-style metadata privacy:
the server learns membership, message timing, volume, and KeyPackage-fetch
signals. Finite Chat already accepts similar leakage in v1 and adds intentional
server-visible routing fields such as room id, sender device, envelope class,
conversation id, push policy, and device liveness.

Marmot retains an outer transport encryption layer to hide MLS framing metadata
from the server. Finite Chat currently exposes enough envelope metadata for the
server to enforce ordering, push/unread/command policy, and activity routing.
Adopting Marmot's peeler shape would require deciding which metadata remains
clear for server admission and product projections.

### Redundancy And Availability

Marmot labels the HTTP profile reduced-assurance because it violates Marmot's
redundant-delivery principle. Finite Chat v1 already chooses one authoritative
room server per room, so this is not a blocker for us. It is a product disclosure
and operations issue: backup, migration, federation, and total device loss remain
future work.

### Server API Completeness

Marmot's note scopes components and responsibilities. Finite Chat already has a
more complete API/reducer surface:

- session and device status;
- KeyPackage upload, device claim, account fanout claim, release, and inventory;
- room create, ordered events, ordered Commits, bounded sync pages, and SSE hints;
- Welcome release, claim, ack, and failure recovery;
- link sessions and later-device fanout;
- repair reports;
- ephemeral activity separate from durable sequence;
- runtime command and state snapshot semantics;
- idempotency records with replayable accepted and rejected outcomes.

This is the surface we would still have to build on top of Marmot.

### Product Fit

Finite Chat is not only encrypted chat. It is also the encrypted command
transport for finitecomputer runtimes. That needs ordered durable command
requests/results, non-notifying runtime state snapshots, daemon survival when
Hermes or inference is down, and stream hints that never directly execute work.
Marmot's application layer boundary is compatible with this, but it does not
define these product-level semantics.

## Adoption Options

1. Keep Finite Chat protocol, borrow Marmot ideas.
   This is the recommended path. Reuse conceptual pieces: Nostr-rooted account
   identity, KeyPackage proof validation, transport/peeler separation, and the
   explicit reduced-assurance framing for single-server mode.

2. Build a Finite profile on top of Marmot.
   Possible, but the profile would be large. It would need to define Finite's
   authoritative room log, device registry, account fanout, Welcome lifecycle,
   idempotency, app-event policies, runtime command ledger, and history policy.
   At that point, the protocol surface is still mostly ours.

3. Adopt Marmot HTTP as-is.
   Not recommended. It is a design note, not a complete server API or conformance
   contract, and it does not currently cover Finite's multi-device and runtime
   recovery requirements.

## Recommendation

Use the Marmot HTTP design as validation that our server-ordered MLS Delivery
Service is not an outlier. Do not replace Finite Chat's protocol with Marmot
unless Marmot grows the same concrete device, Welcome, idempotency, sync, and
runtime-command guarantees we already require.

For multi-device specifically: server-hosted KeyPackages are only the starting
point. The hard requirement is deterministic, recoverable, per-device fanout
through accepted MLS Commits and durable Welcome handling. Finite Chat already
models and tests that; Marmot's current HTTP note does not.
