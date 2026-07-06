# Protocol V1

## Entities

`Account`

A Nostr public key. This is user-level identity.

`Device`

One application install for one account. Every device is its own MLS leaf.

## Identity And Secret Roots

Finite Chat v1 uses the Nostr account key as the user identity root. WorkOS or
finitecomputer login may authorize product access, but cryptographic chat
identity is proof that the user controls the Nostr private key for the account
public key in the room.

The room server is authoritative for room ordering only. It is not authoritative
for who an account or device is. Identity claims are accepted by clients only
when the Nostr-rooted credential and MLS state validate locally.

Persistent Finite Chat device secrets must be rooted in that Nostr private key,
using explicit domain separation for Finite Chat, version, account, and device
purpose. MLS is still allowed to create ephemeral or per-epoch secrets internally;
those are MLS protocol state, not a replacement account identity.

`FiniteDeviceCredentialV1` is the credential payload carried in MLS credential
identity bytes. It binds:

- Nostr account public key;
- Finite Chat device id;
- MLS leaf signing public key or credential key material;
- credential version and expiry/rotation metadata;
- Nostr account signature over the binding.

Clients must reject MLS credentials whose Nostr account signature, device id,
or MLS leaf key binding does not match the expected account/device. Changed
LeafNodes use the same binding rule.

The Nostr key authenticates the device and any persistent device root. The MLS
key material performs room encryption. These are not independent identities.
They are one account identity with per-device MLS participation.

`Room`

One MLS group plus one server-ordered log. V1 has exactly one authoritative
server per room.

`Conversation`

An application-level session inside a room. A Hermes or finitecomputer "new
chat" is a conversation, not a separate MLS group. Conversations do not define
membership, encryption, ordering authority, or delivery boundaries; the room
does.

`Topic`

A first-class user-facing conversation lane inside a room. Protocol messages use
`conversation_id` for topics; "topic" is product language, not a second delivery
or encryption boundary.

`Segment`

A bounded context window inside a conversation. A `/new` command inside an
existing topic starts a new segment, not a new conversation or room.
Finite Chat records only the durable segment boundary and current active
segment id. The app/runtime owns prompt trimming, memory selection, and any
Hermes session reset mapped from that boundary.

`Room Server`

Delivery Service for KeyPackages, ordered room log entries, Welcomes, sessions,
membership intervals, repair reports, and push wake outbox records.

`Durable Application Event`

An encrypted room event that is part of the durable room log. Chat messages,
conversation updates, command requests, and command results are durable
application events unless their kind explicitly says otherwise.

`Ephemeral Activity Event`

An encrypted, TTL-bound room event for intermediate state such as typing,
thinking, working, uploading, or presence refreshes. The room server may route
and cache it briefly, but it does not consume the canonical room sequence, is
not durable history, and never creates unread state or push notifications.
Ephemeral activity uses the same active-member authorization boundary as
durable sends. Senders refresh long-running activity by sending a newer
activity event before expiry, and end it early with an explicit clear.

`Activity Kind`

An encrypted application value inside an ephemeral activity event. Finite Chat
reserves a small generic namespace for shared UX: `typing`, `thinking`,
`working`, `uploading`, `recording`, and `present`. Application-specific kinds
must use a namespaced value such as `finitecomputer.indexing` or
`hermes.tool_calling`.

## Invariants

- A room has one canonical server sequence.
- At most one Commit is accepted per room epoch.
- Clients process entries in sequence order.
- Clients validate cryptography and application policy.
- The server is authoritative for ordering, not identity.
- The server validates only routing envelopes and structural metadata in v1.
- A Welcome is released only after the linked Commit row is durable.
- Mutations are idempotent by account, device, method, path, and key.
- Rejected mutations after idempotency admission are replayable.
- Removed devices can fetch through their removal Commit.
- Removed devices cannot send new events or Commits after the removal Commit is
  the room head.
- Removed devices must not be able to decrypt post-removal application
  ciphertext, even if they obtain those bytes outside normal sync.
- `NeedsRepair` blocks normal sends.
- Protocol limits are enforced before state mutation. Limit failures must not
  create log entries, consume KeyPackages, release Welcomes, or write
  idempotency responses.
- Encrypted application messages use MLS protection. Do not add an extra
  application-message encryption layer unless a future threat model names the
  additional boundary. Local database encryption is separate at-rest protection.
- Durable application events and ephemeral activity events are distinct
  envelope classes. The server can see the class and push policy, but not the
  encrypted activity kind.
- Ephemeral activity payloads use MLS protection under the sender's current
  room epoch. They must not use plaintext activity kinds or a second
  application-message encryption layer.
- Ephemeral activity events must carry `push_policy = never` and an explicit
  expiry. They must not create push outbox records, unread counts, durable
  transcript entries, or command inbox work.
- The canonical room sequence advances only for MLS Commits and durable
  application events. Ephemeral activity events must not occupy `seq`, create
  cursor gaps, or block durable sync.
- Ephemeral activity events must be rejected unless the sending device is
  active, non-revoked, and currently a member at the room head. Pending invited
  devices and removed devices cannot send activity.
- `conversation_id` is optional server-visible routing/index metadata scoped to
  a room. It must not grant access, define identity, replace MLS membership, or
  carry activity semantics.
- Client activity projection state is keyed by device first, then rolled up for
  identity-level UX. A specific device can be active without every device for
  that account or agent becoming active.
- Ephemeral activity expiry is a lease. A newer activity event for the same
  projection key replaces the previous expiry, and an explicit clear removes
  the matching device-scoped activity before expiry. Clears must not remove
  sibling devices, unrelated activity kinds, or a different activity id.
- Decrypted durable terminal events may clear matching activity projection
  state for the sender. This is a client-side projection update, not a server
  mutation or room-log side effect.
- Generic activity kinds are reserved Finite Chat values. Unknown namespaced
  activity kinds must be preserved in projection state and ignored by generic
  UI unless an application-specific renderer understands them.
- `present` is a v1 ephemeral activity kind, not a separate global presence
  system. Without `conversation_id`, `present` means the device is live in the
  room; with `conversation_id`, it means the device is live in that app-level
  conversation.
- Runtime command requests, command results, and command cancellations are
  durable application events. Ephemeral activity can describe progress, but it
  must not create command inbox work.
- Command execution is driven from ordered durable sync. A runtime must decrypt,
  validate, and persist request ledger state before scheduling work; it must not
  execute directly from a stream or push callback.
- Command targeting lives in encrypted payloads. Optional server-visible wake
  hints may reduce unnecessary device wakes, but clients must treat them as
  non-authoritative routing hints, not access control or execution policy.
- `chat.receipt` is a durable encrypted application event for read/delivered
  state. It must use `push_policy = never`, must not create unread state, and
  should be optional by client or account policy.
- V1 transport uses HTTP mutations, cursor-based pull sync, and SSE hints.
  Streams and push wakes are never authoritative; clients repair gaps through
  bounded sync pages. A stream hint can make a client pull; it must not advance
  the applied cursor or directly execute command work.
- Attachment blobs are encrypted before upload to a Blossom-compatible blob
  service. Blob encryption protects bytes stored outside MLS; the room message
  still uses MLS for the attachment reference and metadata.

## V1 Limits

These are protocol constants, not tuning hints:

- envelope payload: `256 KiB`;
- sync page: `100` entries and `4 MiB` of envelope payload bytes;
- devices per account per room: `32`;
- direct room devices per account: `8`;
- explicit KeyPackage claims per request: `1`;
- account fanout KeyPackage claims per request: `8`, one available package per
  device;
- KeyPackage inventory per device: `64` unconsumed packages, counting
  available and leased packages;
- KeyPackage payload: `64 KiB`;
- Welcomes claimed per request: `32`;
- staged Welcomes per Commit: `32`;
- account room discovery page: `256` rooms;
- Welcome payload: `1 MiB`;
- ratchet-tree payload: `1 MiB`;
- idempotency records per room/device: `4096`;
- link-session payload: `1 MiB`;
- attachment plaintext: `32 MiB`;
- device liveness heartbeat freshness window: `60 seconds`;
- runtime state snapshot payload: `64 KiB`;
- runtime state snapshot freshness window: `5 minutes`;
- runtime state keys per room/device: `128`;
- conversation projection entries per client: `4096`;
- conversation metadata payload: `16 KiB`;
- segments per conversation: `1024`;
- conversation segment payload: `16 KiB`;
- runtime command JSON payload: `128 KiB`;
- runtime command activity clears per result: `16`;
- runtime command ledger records per daemon/client: `1024`;
- decrypted ephemeral activity payload: `64 KiB`;
- decrypted ephemeral activity projection entries per client: `4096`;
- ephemeral activity expiry: `30 minutes` from server receipt;
- ephemeral activity cache entries per room/conversation/device route: `64`;
- idempotency key: `128` bytes;
- account id, device id, room id, MLS group id, object ids, state keys:
  `128` bytes each.

The numbers are intentionally small for v1. They keep WASM memory behavior
predictable, bound retry/fanout work, and make accidental full-room reads show
up as test failures.

## Product Trust Modes

Finite Chat distinguishes protocol capability from product disclosure.

- `local_device_e2ee`: device secrets stay on the user's device, so the client
  may describe the chat as end-to-end encrypted.
- `hosted_trusted_server_client`: a hosted server-side Rust client decrypts on
  behalf of the web UI. This is useful web chat, but it must not be labeled
  E2EE.
- `plaintext_archive`: imported legacy finitecomputer chats are read-only
  archive material. They must not be labeled E2EE or treated as writable
  Finite Chat room state.

These modes do not change MLS semantics. They keep product copy and migration
behavior honest while finitecomputer moves from trusted-server web chat toward
true local-device clients.

Common product client kinds map to those modes without changing room protocol
semantics:

- `hosted_web_bridge`: a server-side trusted client for hosted web chat;
- `native_device`: a local-device E2EE user client;
- `electron_daemon`: a desktop local-device E2EE user client;
- `runtime_device`: an agent/runtime participant whose device secrets stay on
  the runtime host; it is not a user-facing disclosure surface;
- `plaintext_archive`: read-only imported legacy chat.

## Server API Sketch

Session:

- `POST /v1/session/challenge`
- `POST /v1/session/login`
- `POST /v1/devices`
- `POST /v1/devices/{device_id}/revoke`

Device records are a server-side control-plane ledger, not identity proof.
Clients still decide whether a device identity is valid by verifying its
Nostr-rooted MLS credential. The server records only whether a device is
currently usable for server mutations. Revocation is terminal in v1: a revoked
device cannot upload or claim KeyPackages, claim or activate Welcomes, create
rooms, send application events, submit Commits, or be added to a room again.
MLS remove Commits are still required for the cryptographic cutoff; the device
status ledger prevents the revoked install from acquiring new server-mediated
material while room removals fan out.

KeyPackages:

- `POST /v1/key-packages`
- `POST /v1/key-packages/invite-availability`
- `GET /v1/devices/{account_id}/{device_id}/key-packages/inventory`
- `POST /v1/key-packages/claim`
- `POST /v1/accounts/{account_id}/key-packages/claim`
- `POST /v1/devices/{account_id}/{device_id}/key-packages/claim`
- `POST /v1/key-packages/release`

Uploaded KeyPackages include opaque serialized MLS KeyPackage bytes plus the
metadata the server uses for routing/cache checks. Claiming a KeyPackage returns
those exact bytes to the adding client; clients parse and verify MLS credential
identity locally.

Each device has a bounded KeyPackage inventory. The cap counts available
packages plus leased packages because both are unconsumed server-held material;
accepted add Commits consume leased packages and free inventory space. Clients
use the inventory view to keep a small target number of available packages
without pushing an unbounded upload pile into the Delivery Service. Runtime
clients persist generated upload requests in encrypted local state before
uploading generated packages, then clear each request only after server
acceptance. Exact duplicate uploads are idempotent retry; a duplicate id with
different owner, ref, hash, or payload is rejected. V1 client helpers derive
package ids from the serialized MLS KeyPackage payload hash so replenishment
does not need a persisted counter.

Account fanout claim returns at most one available KeyPackage per registered
device for the target account, ordered deterministically by device id and
KeyPackage id. This is the invite primitive for multi-device users: the server
routes packages to devices, but the adding client still verifies every
Nostr-rooted MLS credential before constructing the Commit.

Invite availability is a read-only batch projection over account KeyPackage
inventory. Given account ids, the home server returns whether each account has
at least one available KeyPackage for a non-revoked device. It never returns
device ids, KeyPackage ids, or KeyPackage bytes, and it never claims or leases
inventory. Product UI uses this to distinguish people who can currently be
invited to a Room from Nostr follows who do not yet have Finite Chat invite
material.

Device fanout claim returns one available KeyPackage for a specific target
device, ordered deterministically by KeyPackage id. The runtime link-fanout
worker uses this when adding a later-linked device to all existing rooms: each
room gets one claimed KeyPackage, one staged Welcome, and one ordered add
Commit.

Rooms:

- `POST /v1/rooms`
- `GET /v1/accounts/{account_id}/rooms?after_room_id=...&limit=N`
- `GET /v1/rooms/{room_id}/events?after_seq=N`
- `GET /v1/rooms/{room_id}/stream`
- `POST /v1/rooms/{room_id}/events`
- `POST /v1/rooms/{room_id}/commits`

V1 room transport is explicit:

- durable mutations use HTTP `POST`;
- durable recovery uses cursor-based `GET` sync pages;
- live updates use SSE as a hint channel;
- ephemeral activity may be sent by HTTP mutation and delivered through SSE or
  a short TTL cache;
- WebSockets are out of scope for v1.

SSE does not carry authority. If a client misses, duplicates, or reorders SSE
items, it repairs by polling durable sync with its last applied cursor. A stream
callback must not directly execute application work.

Welcomes:

- `POST /v1/welcomes/claim`
- `POST /v1/welcomes/{welcome_id}/ack`
- `POST /v1/welcomes/{welcome_id}/release`

Repair:

- `POST /v1/rooms/{room_id}/repair-reports`

Device linking:

- `POST /v1/link-sessions`
- `POST /v1/link-sessions/{id}/payload`
- `POST /v1/link-sessions/{id}/claim`
- `POST /v1/link-sessions/{id}/ack`

A newly linked device joins existing rooms through normal add-device Commits.
Because MLS KeyPackages are single-use, the device must replenish enough
KeyPackages for the rooms it is being linked into; each accepted room add
releases a distinct Welcome for that room. The replenishment loop should query
inventory, upload at most the missing packages needed to reach the device's
target available count, and stay under the unconsumed inventory cap.
The account-room discovery endpoint is a control-plane helper for that worker:
it pages over current/pending membership rows for an account and returns room
head metadata plus the account's current devices. It is not an authorization
oracle for identity; clients still verify Nostr-rooted MLS credentials and the
server only orders the resulting Commits.

## History Policy

V1 room history starts for a device at that device's accepted add Commit. A
newly added device may sync the add Commit and later room entries, including
messages sent before it acked its Welcome, but the room server must not replay
pre-membership room log entries as ordinary history for that device.

Pre-invite history recovery is a separate product protocol. It must be provided
by encrypted backup or an explicit member-to-member history-share message, not
by making the server authoritative over old plaintext or hidden key access.

## Message Ids

`seq` is a room-local cursor. It is not a stable message id.

`message_id` is derived from serialized message bytes:

```text
SHA256("finite-message-id-v1" || canonical_finite_envelope_bytes)
```

`message_id` is unique per room log. A second mutation with a different
idempotency key but identical envelope bytes is rejected as a duplicate message,
not appended as a second log entry.

## Sync Page

Sync returns an explicit page:

```json
{
  "entries": [],
  "next_after_seq": 42,
  "has_more": false
}
```

Clients must use `next_after_seq` as their next cursor, not the last visible
entry they happened to receive. This matters for removed devices: the server may
scan entries after the requested cursor that the requester is no longer allowed
to receive, and the requester must still be able to advance past those filtered
entries.

`has_more` means the server stopped because a page bound was reached. It does
not mean the room is quiescent forever.

## Idempotency Capacity

Idempotency records are durable retry state. The room server must replay an
existing record even when the room/device ledger is full.

When a room/device already has `4096` idempotency records, a new mutation with a
new idempotency key is rejected with `IdempotencyCapacityExceeded` before side
effects. The server must not silently delete old records to make room, because
that would turn a lost response retry into a possible duplicate mutation.

## Application/RPC Payloads

Finite Chat orders encrypted application messages. The room server sees the
`FiniteEnvelope` routing fields and opaque payload bytes; clients decrypt and
interpret the plaintext.

The envelope distinguishes durable application events from ephemeral activity
events before decryption:

- `durable`: ordered room-log data, push-eligible according to room and client
  policy;
- `ephemeral`: best-effort activity state, always `push_policy = never`, with a
  bounded explicit expiry.

Both envelope classes may carry an optional cleartext `conversation_id`.
Clients use it to place messages and live activity in the right app-level
session without scanning every decrypted payload in a room. Rich conversation
state such as title, preview text, runtime status, and activity kind remains in
the encrypted payload.

Finite Chat clients may present conversations as topics. A topic is still a
conversation: it has a stable `conversation_id`, encrypted title/settings, and
conversation-scoped messages, receipts, activity, and command requests. External
topic systems such as Telegram `message_thread_id` or Hermes `thread_id` should
map into this layer, not into rooms.

Finite Chat reserves generic durable chat kinds:

- `conversation.create`: creates an app-level conversation inside a room;
- `conversation.update`: updates encrypted conversation metadata;
- `conversation.archive`: marks a conversation archived for the app projection;
- `conversation.segment.start`: starts a new context segment inside an existing
  conversation;
- `chat.message`: user-visible message;
- `chat.edit`: user-visible message edit;
- `chat.reaction`: reaction to a message;
- `chat.receipt`: read, delivered, or seen state.

FiniteChat-native poll creation is a `chat.message` payload with
`type = "finitechat.chat.poll.v1"` so the poll appears as a normal
user-visible transcript item and creates the same unread/push semantics as
other messages. Poll votes are durable namespaced application events using
`name = "chat.poll.vote.v1"` and `ApplicationDeliveryPolicy::NON_NOTIFYING`.
Clients project votes into the poll message from ordered durable replay; the
server only sees an opaque non-notifying event.

Conversation creation should be explicit when the sender can do so. A client may
lazily materialize a conversation when it sees the first durable event for an
unknown `conversation_id`, but explicit `conversation.create` is preferred for
clear ordering and projection behavior.

Clients project topics by `(room_id, conversation_id)`. A
`conversation.create` explicitly creates the topic and carries bounded encrypted
metadata such as title, description, external topic reference, and skill
binding. A `conversation.update` replaces that encrypted metadata. A first
`chat.message` with a new `conversation_id` may lazily materialize it for simple
clients and imports. `conversation.archive` is scoped to that one topic. A
`conversation.segment.start` requires `conversation_id`, adds a bounded segment
record to that topic, and updates `active_segment_id`.

Topic display names work like group chat names. The stable identifier is the
non-human `conversation_id`; the visible title is encrypted conversation
metadata. Any member with the app's admin permission may append
`conversation.update` to rename the topic. The server orders the update but does
not decide whether the sender was allowed to rename it; clients verify the
sender against decrypted room/conversation role state and ignore unauthorized
metadata updates in their projection. Concurrent valid renames are resolved by
room order: the later accepted update is the visible title.

`conversation.segment.start` is used when an app wants a fresh context inside an
existing topic, for example Hermes `/new` in a Telegram topic. It is a durable
encrypted event so every device agrees on the boundary, but it does not create a
new conversation, room, membership set, delivery log, or cryptographic state.

Push policy is part of the server-visible envelope, not the encrypted semantic
kind. V1 defaults are:

- `chat.message`: push-eligible according to room and account notification
  policy;
- `chat.edit`, `chat.reaction`, `chat.receipt`, and conversation metadata:
  `push_policy = never`;
- namespaced `chat.poll.vote.v1`: `push_policy = never`;
- `conversation.segment.start`: `push_policy = never`;
- `runtime.state.snapshot`: `push_policy = never`;
- `runtime.command.request`: may wake the encrypted target runtime device, but
  should not create a user notification by default;
- explicit runtime status refresh commands use `push_policy = never` and still
  create command inbox work for the target runtime;
- `runtime.command.result`: push-eligible only when the receiving app maps it to
  user-visible output or a user-requested alert;
- `ephemeral`: always `push_policy = never`.

Receipts are encrypted application payloads. A `chat.receipt` may reveal read or
delivery state to room members after decryption, but the room server only sees
an opaque durable event with `push_policy = never`.

The encrypted activity payload owns the semantic kind. The server does not need
to know whether an ephemeral event means typing, thinking, working, uploading,
or another generic chat activity. The server-visible envelope carries only the
fields needed to route and discard it: room id, optional conversation id,
sender device, delivery class, push policy, expiry, and bounded opaque
MLS-protected bytes.

Ephemeral activity may be delivered over a live stream or short TTL cache, but
it is not returned by durable room-log sync and cannot be used as a replay
cursor. Clients must tolerate dropped, duplicated, reordered, or expired
activity events. If an activity event arrives for an old epoch, a future epoch,
or otherwise fails MLS processing, the client drops it without repair.

Human typing indicators should use short expiries. Agent `thinking` or
`working` indicators may last for minutes, but remain bounded by the v1 expiry
limit. Long-running senders should refresh the activity while work continues
and send an explicit clear when a durable message, command result, or terminal
failure makes the intermediate state obsolete.

The encrypted activity payload may carry an `activity_id`. Clients normalize a
missing `activity_id` to a reserved default value for short-lived single-state
activity such as human typing. Long-running agent activity should set
`activity_id` to the command, request, or run id that caused the work. Refreshes
and clears match on the normalized activity id, so a delayed clear for an old
operation cannot erase a newer `working` indicator from the same device.

The encrypted activity payload also carries an `activity_kind`. Generic clients
may render reserved Finite Chat kinds consistently across human chat and agent
chat: `typing`, `thinking`, `working`, `uploading`, `recording`, and `present`.
Application-specific activity uses namespaced kinds and must not change generic
Finite Chat behavior unless the client opts into that namespace.

Default expiry guidance is kind-specific and remains bounded by the v1 maximum:
`typing` should normally expire within `30 seconds`; `present`, `uploading`,
and `recording` should normally expire within `2 minutes`; `thinking` and
`working` should normally use a `5 minute` lease, with longer leases up to the
`30 minute` maximum only for known long-running agent work. Senders refresh
before expiry while the state remains true.

Durable terminal events may also carry encrypted activity-clear declarations,
such as `(activity_kind, activity_id)`. Clients apply these clears to the
durable event sender's device-scoped activity in the same room and optional
conversation. A normal chat message can clear that device's default `typing`
activity; a `runtime.command.result`, assistant response, or terminal failure
can clear the matching `thinking` or `working` activity for its run id. This
gives correctness when the explicit ephemeral clear was dropped.

For `runtime.command.result`, clients validate the terminal result shape before
applying its bounded clear list. Invalid result payloads must not mutate
activity projection state.

The server authorizes ephemeral activity against its current device ledger and
membership cache before forwarding or caching it. This check is not identity
proof; clients still verify Nostr-rooted MLS credentials locally. It only keeps
non-members, pending devices, and removed or revoked devices from creating live
room activity.

The server TTL cache stores bounded opaque activity events by room, optional
conversation id, and sender device route. Because `activity_kind` and
`activity_id` are encrypted, the server must not coalesce by those fields. It
expires entries by server receipt time, enforces the per-route cache-entry
limit, and may drop old activity without affecting durable sync. Clients replay
cached activity after decryption and coalesce by the full projection key.

After decryption, clients project activity by `(room_id, conversation_id,
account_id, device_id, activity_kind, normalized_activity_id)`. Normal UI may
roll this up to an identity-level display such as "Alice is typing" or
"Runtime is working", but device and activity id remain the source of truth.
Device-specific views can expose the exact active device when that matters,
such as targeting a runtime device with GPU access.

Clients normalize missing `activity_id` to the reserved `default` id. A set
refreshes only the matching projection key, a clear removes only the matching
projection key, and expiry removes entries whose lease has elapsed. Durable
terminal events may clear activity for the durable event sender using the same
sender-scoped key; this repairs dropped ephemeral clear events without granting
cross-device clear authority.

Finitecomputer dashboard/runtime RPC should live inside the encrypted
application payload. The plaintext can be JSON because it is client-owned
application data, not authoritative room-server state.

The intended deployment model is portable: a `finite` or `finitec` daemon can run
inside an agent hosted anywhere and connect outward to Finite. Protocol features
should not assume Kubernetes, pod exec, dashboard-reachable HTTP servers, or a
central control-plane database. If a capability only works because Finite hosts
the runner, it is hosted-runner admin, not a generic Finite Chat command.

Chat and management are separated by application kind. Runtime commands are
typed, allowlisted management requests with idempotent handlers; chat messages,
attachments, receipts, and topic events are normal chat application data and
must not be transported over a generic management queue.

Read-mostly runtime status should usually be represented as encrypted
latest-state projection data, not as a command request for every UI render.
Commands are for work that needs runtime scheduling, authorization, mutation, or
an explicit refresh. Polling a dashboard page must not by itself append durable
command traffic to a room log.

Finite Chat reserves a generic durable state kind:

- `runtime.state.snapshot`: publishes structured current runtime state.

`runtime.state.snapshot` is the structured version of an old instant-messenger
status message: current, user-facing enough to display, bot-readable, but not a
chat message. It is durable so new devices and restarted dashboards can recover
the latest state, and it must use `push_policy = never`, must not create unread
state, and must not create command inbox work.

The encrypted snapshot payload owns:

- `state_key`: stable application key such as `runtime.inference`,
  `runtime.gateway`, `runtime.connection.matrix`, `runtime.connection.telegram`,
  `runtime.published_apps`, or `runtime.capabilities`;
- `schema`: application schema id for the typed JSON body;
- `revision`: monotonically increasing value for this `(room, source device,
  state_key)`;
- `observed_at`: when the runtime observed the state;
- `expires_at`: when clients should mark the projection stale if no newer
  snapshot arrives;
- `status`: the typed JSON value for the app.

Clients project runtime state by `(room_id, source_account_id,
source_device_id, state_key)`. A newer revision replaces the prior projection
for that key. If two snapshots race with the same revision, clients keep the one
with the later accepted room sequence. Unknown schemas are preserved for
specialized clients and ignored by generic UI.

Runtime daemons should publish snapshots on meaningful changes and on a slow
refresh cadence. The snapshot freshness window is bounded to `5 minutes`; it is
not a heartbeat substitute. Liveness remains a small server-visible heartbeat,
while `runtime.state.snapshot` carries encrypted application state. A command
result may include or be immediately followed by a snapshot for the state it
changed.

Device liveness is server-visible delivery state, not encrypted runtime status.
It says a registered, non-revoked device has checked in recently enough to be
woken or shown as reachable. It does not advance the room log, produce push or
unread work, or satisfy a typed `runtime.state.snapshot` read.

Finite Chat reserves generic durable command kinds:

- `runtime.command.request`: asks a target identity or device to do work;
- `runtime.command.result`: terminal result for a request;
- `runtime.command.cancel`: durable cancellation request.

The encrypted command payload owns `request_id`, command name, target identity,
optional target device, body, terminal status, result body, error details, and
activity-clear declarations. The room server does not parse or validate those
fields. It only orders the durable event bytes, applies envelope limits, and
replays idempotent append results.

V1 command payloads use typed JSON envelopes with a schema-tagged bounded JSON
body. The request target is part of the encrypted payload. A runtime records a
request only when that decrypted target matches the local account/device; any
cleartext wake hint is merely a way to decide who should sync sooner.

`request_id` is an encrypted app-level correlation id, not a server mutation id.
The server-level retry identity remains the serialized envelope `message_id`
plus the mutation idempotency key. If a sender loses the append response for a
command request, result, or cancel, it retries the exact same envelope bytes
with the same idempotency key. Retrying with a new idempotency key and the same
envelope bytes is still a duplicate message error.

Command progress splits by durability:

- ephemeral activity: `thinking`, `working`, progress pings, tool-running
  state, and other intermediate status that can be dropped;
- durable application events: user-visible output, durable logs or checkpoints,
  terminal success, terminal failure, and terminal cancellation.

A runtime processes command requests by syncing ordered durable events,
decrypting them, validating sender and target policy locally, and recording a
request ledger entry before scheduling execution. The request ledger should
deduplicate replays by request id, sender, conversation, and original message
id, reject conflicting reuse, and remain bounded. Execution workers read the
ledger; live streams and push wakes only cause sync.

Commands that name an encrypted `resource_key` are scheduler-serialized per
room, target account/device, and resource key. The runtime still records every
durable request in sequence order, but it exposes only the oldest pending
command for a keyed resource as ready work until that command reaches terminal
state. Conversation id is intentionally not part of the resource lock:
`hermes.config` updates from different topics still mutate the same runtime
resource.

Command terminal state is also ordered-log state. A result or cancel may close
a pending ledger record only when its accepted sequence is after the request
sequence. The ledger stores the first terminal event's message id and sequence;
an exact replay of that terminal event is idempotent, and any later competing
result or cancel is ignored. This gives result/cancel races a visible
first-terminal-wins rule without asking the server to parse encrypted command
payloads.

Finite Chat only records ordered segment boundaries. The app/runtime owns what a
segment means for its prompt context or local memory. A Hermes bridge can map
`conversation.segment.start` to Hermes' existing `/new` session reset behavior,
while the Finite Chat transcript remains visible and durable.

Cancellation is also durable. A `runtime.command.cancel` references the
encrypted `request_id`. If cancellation wins before terminal result, the
runtime emits a durable `runtime.command.result` with `cancelled` status. If a
terminal result is already accepted by the client projection, a later cancel is
ignored for that request.

Optional wake hints may appear in the server-visible envelope when useful, for
example to wake only a runtime device with GPU access. A wake hint must not be
trusted by the runtime as proof that the command targets it. The decrypted
payload and local policy decide whether the runtime records or executes the
request.

## Attachments And Blobs

Attachments are encrypted blob references carried inside durable encrypted
application payloads. The room server does not store plaintext attachment bytes
and does not parse attachment metadata.

V1 uses a Blossom-compatible blob storage shape:

1. validate plaintext size before encryption;
2. encrypt the file locally for the room attachment;
3. upload encrypted bytes to one or more Blossom-compatible blob servers;
4. verify the uploaded ciphertext hash;
5. send a durable `chat.message` or app event containing the encrypted
   attachment reference;
6. on download, verify ciphertext hash, decrypt locally, then verify plaintext
   hash.

The encrypted attachment reference should include the blob URL, ciphertext hash,
plaintext hash, blob encryption nonce/key material or equivalent media
reference, scheme version, MIME type, filename, and optional dimensions. Blob
servers may learn URL, ciphertext hash, object size, timing, and requester
metadata; they must not receive plaintext bytes, plaintext filename, or
plaintext MIME type unless a future product decision explicitly accepts that
metadata leak.

The first implementation proof lives in `finitechat-blob`. It defines the v1
encrypted reference shape, encrypts bytes with per-attachment AES-256-GCM key
material, uses a local content-addressed blob-store abstraction, and exposes a
small Blossom-shaped HTTP upload/download boundary. That boundary carries only
ciphertext bytes and verifies the returned descriptor before producing an
attachment reference. The HTTP executor itself stays outside the protocol crate
so finitecomputer can reuse its existing networking stack without changing
encrypted chat payload semantics.

This blob-encryption layer is for bytes stored outside the MLS room log. It does
not add another encryption layer to ordinary room messages.

Example plaintext before MLS encryption:

```json
{
  "type": "runtime.command.request",
  "request_id": "req_123",
  "command": "dashboard.send_message",
  "target": {
    "account_id": "agent_abc",
    "device_id": null
  },
  "body": {
    "project_id": "proj_abc",
    "text": "run tests"
  }
}
```

Server-side invariants still live in schema rows and transactions, not inside
this JSON.

## Membership Delta

Commit requests carry cleartext `MembershipDeltaV1` beside the opaque Commit.
The server uses it for cache and routing. Clients validate actual MLS effects
by processing ordered Commit log entries with OpenMLS before sending or
decrypting messages in the next epoch.

Required structural checks:

- `base_epoch == expected_epoch`;
- `post_commit_epoch == base_epoch + 1`;
- update/rekey Commits may have no membership delta rows;
- no duplicate add devices;
- no duplicate remove devices;
- no add and remove of the same device;
- every add has a KeyPackage id/ref/hash;
- every add has exactly one matching staged Welcome;
- every staged Welcome has non-empty opaque Welcome bytes and non-empty
  ratchet-tree bytes, both bounded to `1 MiB`;
- every remove has a removed leaf index;
- `commit_message_id` matches the submitted Commit envelope.

The room server stores staged Welcome and ratchet-tree bytes as opaque payloads
linked to the accepted Commit. It validates ids, sizes, and one-to-one matching
with membership adds; it does not parse or trust the MLS contents. Claiming a
Welcome returns these exact bytes to the recipient device.

For multi-device invites, one MLS Commit may add several devices from the same
account. Each added device receives its own Welcome record, but the opaque MLS
Welcome bytes may be the same batch Welcome containing secrets for all added
leaves. A device becomes a member interval at the accepted Commit seq even
before it acks the Welcome, so it can sync messages after that seq; it cannot
send until its own Welcome is claimed, activated, and acked.
