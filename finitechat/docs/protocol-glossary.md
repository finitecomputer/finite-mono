# Protocol & Reliability Glossary

CONTEXT.md owns the user-facing domain language (Room, Conversation, Topic,
Segment, Activity, Runtime State). This glossary owns the delivery-layer and
reliability language underneath it: what each mechanism is, and the user-visible
promise it exists to keep.

## MLS primer (the borrowed vocabulary)

**KeyPackage**:
A one-time-use cryptographic "add me" token a device publishes ahead of time.
Anyone holding a fresh KeyPackage for your device can add it to a group without
your device being online.
_User promise_: you can be added to a group while your phone is in your pocket.

**Welcome**:
The MLS envelope that hands a newly added device the group's secrets. Claiming
and processing a Welcome is how a device actually joins.
_User promise_: "added to the group" becomes "can read the group" without any
extra round trips from existing members.

**Commit**:
The MLS message that changes a group — adds, removes, key rotation. Everyone
must apply commits in the same order.
_User promise_: every member agrees on who is in the room.

**Epoch**:
A counter that increments with each accepted commit. Messages are bound to the
epoch they were sent in.
_User promise_: a removed member's keys stop working at a precise, agreed point.

## Server-side mechanisms (finitechat-server)

**Durable SQLite operation log**:
Every accepted delivery operation is appended inside a SQLite transaction; on
restart the server replays the log to rebuild all in-memory state. Crash points
are tested at every write boundary (the trigger-backed crash matrix).
_User promise_: a server crash or deploy never loses a message, duplicates a
message, or half-applies a join. Either everything about an operation happened,
or none of it did.

**Scoped idempotency**:
Every publish carries an idempotency key. Exact retries replay the original
receipt byte-for-byte (even after restart); reusing a key with different
content is rejected; record capacity is bounded per room/sender.
_User promise_: flaky networks plus client retries never produce duplicate
messages or duplicate group adds.

**Typed `/commits` validation**:
Before durably accepting a membership-changing commit, the route checks the
declared membership delta: epochs line up, commit id matches, no duplicate
adds/removes, no add/remove overlap, device caps respected, direct-room account
pairs respected, KeyPackages actually claimed. Rejection happens before any
side effect.
_User promise_: a buggy or malicious client cannot wedge a room into a state
that breaks the chat for everyone else. Failures are clean and retryable.

**Welcome-release coupling**:
A Welcome is published to the recipient's inbox only after the commit that adds
the device is durably accepted — never before, and exactly once.
_User promise_: no ghost invites. You never join a group whose membership
change didn't actually land, and a retried add never delivers two Welcomes.

**KeyPackage available → claimed → consumed lifecycle**:
The server tracks each published KeyPackage through a lease state machine:
available (claimable), claimed (leased to an in-flight add, can expire back to
available), consumed (burned by an accepted commit, frees cap space). Caps are
enforced per device.
_User promise_: an inviter that crashes mid-add doesn't permanently burn your
"add me" tokens, and you never run out of them silently.

**Room-membership interval projection**:
The server's record of which device was a member of which room for which
sequence ranges (`[start_seq, end_seq)`), derived from typed bootstrap,
commits, and Welcome acks. Group sync pages are filtered per requester against
these intervals; cursors still advance over hidden entries.
_User promise_: leaving and joining mean something. A new device doesn't
receive pre-invite history it couldn't decrypt anyway; a removed device can
sync up to its own removal and nothing after.

**Account-room directory**:
A query-side index answering "which rooms do this account's devices belong
to," kept in step with bootstrap, accepted commits, and Welcome activations.
_User promise_: a newly linked device can discover every conversation it needs
to be added into, without any member manually telling it.

**Link session**:
A short-lived rendezvous slot holding an opaque encrypted pairing payload, with
a create/upload/claim/ack/release/expire lifecycle. The server never interprets
the payload.
_User promise_: pairing a new phone (QR-code style) works over plain HTTP and
the server learns nothing about the pairing secret.

**Fanout checkpoints**:
Server-stored progress markers for the "add my new device to all N rooms" job:
per-room plan, prepared commit id, accepted sequence, done state.
_User promise_: linking a new device finishes even across crashes and lost
responses — no room skipped, no room added twice.

**Delivery-effect projection**:
The caller-supplied push/unread/command-inbox policy recorded atomically with
each durable application event, without the server reading the encrypted
payload.
_User promise_: badges and notifications are right. A new message pings you; an
edit, reaction, or read receipt doesn't — and the counts survive restarts.

**Revoked-device projection**:
A terminal per-device blocklist that gates every server-mediated path:
KeyPackage publish/claim, Welcome claim/ack, typed events, typed commits, and
commits that try to add a revoked device.
_User promise_: a lost or stolen device is cut off everywhere, immediately and
permanently.

**Ephemeral activity cache / device liveness**:
Volatile, bounded, never-persisted state for typing-indicator-style activity
(scoped to a room or conversation) and device heartbeats. Cleared on restart by
design.
_User promise_: typing indicators and presence feel live but never bloat
history, never page anyone, and never outlive a restart.

## Client-side mechanisms (finitechat-client)

**FiniteChatDevice state machine**:
The single state holder for a device: per-room MLS group state, per-room sync
cursors, pending Welcomes, pending acks, pending KeyPackage uploads, in-flight
link fanouts. Every mutation is snapshot-exportable.
_User promise_: the app can be killed at any instant and resume exactly where
it was — no lost decryption state, no re-processing.

**Pending-commit merge rule**:
A device does not trust its own membership commit just because the server said
"accepted." It merges only after observing that commit in the server-ordered
log, like any other member would.
_User promise_: two of your devices (or two members) changing a group at the
same moment cannot fork the room; everyone converges on the same winner.

**Sync worker (`run_runtime_sync_tick`)**:
The periodic loop: replenish KeyPackages toward target, claim and activate
Welcomes, ack them, pull ordered room pages from the cursor, apply each entry
once. Strictly pull-based: stream-style hints only mark "pull needed"; only
pulled ordered pages advance state.
_User promise_: all your devices converge to the same history even when push
notifications are dropped, duplicated, or reordered.

**Link-fanout FSM**:
The "add my new device everywhere" orchestrator: discover rooms via the
account-room directory → claim a KeyPackage for the new device per room →
prepare the add commit → submit → on lost response, retry the same idempotency
keys → on same-epoch loss, reprepare at the next epoch → new device claims and
activates each Welcome. Per-room status: Pending → Prepared → Done.
_User promise_: "link my new laptop" reliably joins it to every conversation,
even while other members are concurrently changing those same groups.

**Encrypted persistence (client store)**:
The full device snapshot (including OpenMLS storage records) serialized and
encrypted at rest with AES-256-GCM under a key derived from the account secret.
_User promise_: a stolen disk or backup leaks neither messages nor group keys.

## Topology vocabulary (ADR 0005 — target, not yet built)

**Room server**:
The single ordering authority for one or more rooms: hosts their logs,
membership projections, and welcome lifecycles. Self-hostable by anyone; holds
no push credentials and no account registry. Today every room shares one
server; the protocol does not require it.
_User promise_: your group can run on a server your group controls, and a bad
or dead server costs you exactly that one room — never your identity or your
other chats.

**Home server**:
The account-resident server: KeyPackage inventory, push tokens, device
linking, liveness, fast-block revocation, and the account-room directory.
Typically run by the app vendor, because push delivery requires vendor-held
APNs/FCM credentials. Replaceable by design — identity is key-derived and
every home-server record is client-regenerable.
_User promise_: if your home server goes out of business, you migrate; you do
not lose your identity, your rooms, or your history.

**Wake relay**:
In the sharded topology, room servers forward the wake hint `{room_id, seq}`
to the device's home server, which holds the push credentials. The payload is
frozen content-free and state-free (pull-based sync), so relaying through an
untrusted room server is safe.
_User promise_: pushes work even for self-hosted rooms, without handing your
device token or your messages to a stranger's server.

## One-line altitude check

Everything in "Server-side mechanisms" exists to make the server a trustworthy
*orderer and bookkeeper of opaque bytes* — it never decrypts content.
Everything in "Client-side mechanisms" exists to make a device *crash-safe and
convergent* against that ordered log. The MLS primer terms are the only shared
vocabulary with Marmot/OpenMLS; the rest is finite's language.
