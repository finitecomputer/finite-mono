# Chat protocol contract

Executable wire types and bounds live in `crates/finitechat-proto/` and
`crates/finitechat-http/`; public routes live in `crates/finitechat-server/`.
The application payload contract below complements those definitions.
See [storage](storage.md) for persistence and recovery constraints.

## Identity and ordering

A Principal is a Nostr key; each Device has its own MLS leaf. The account
signature binds the Device id, MLS signing key and credential lifetime in
`FiniteDeviceCredentialV1`. Clients validate that binding on KeyPackages,
Welcome activation and changed leaves. The server does not attest identity.

Human and Agent Runtime Finite Homes have separate local identity secrets.
Client persistent secrets use explicit versioned domain separation from that
identity. MLS epoch secrets remain MLS state, not a replacement identity root.

A Room has one authoritative ordered log and one MLS group. Clients apply
entries in order and validate cryptography and application policy locally.
Topics and segments are application context, not membership boundaries.

- At most one Commit is admitted per source epoch. Commit side effects and
  linked Welcomes are durably committed before publication.
- Removed Devices cannot send after removal and cannot decrypt later epochs.
  Fetch authorization respects membership intervals, including the removal
  Commit needed to learn the membership change.
- Typed rooms accept typed routes. Raw publish is an internal conformance
  surface, not a public membership-check bypass.
- `/events` requires explicit application delivery policy. A DM is an ordinary
  Room; the server does not enforce uniqueness of account pairs.
- Welcome admission is claim then activate; a transient activation failure
  remains retryable. Current admission is KeyPackage/Add/Welcome, not PIN or
  invite-session rendezvous.
- Idempotency and digest dedup preserve exact retry semantics. There is no
  lifetime 4,096-message-per-sender limit.
- Realtime hints and TTL-bound activity do not replace ordered sync. Activity
  consumes no durable room sequence and creates no durable unread state.
- Missing or invalid local MLS state fails closed. Repair must preserve the
  Principal, supported historical readers and the recovery boundary.

## Encryption boundary

MLS protects application messages; encrypted blob references identify separately
encrypted attachments. The server may validate bounded routing metadata but
must not become the application-content or membership-policy authority.
Claims of recovery require the same encrypted state plus independently
recoverable key authority. Retained older-state fixtures are compatibility
contracts, not obsolete material to delete during documentation cleanup.

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

Finite's product projection stores chat names and archived state as encrypted,
non-notifying namespaced durable events scoped to the owning `conversation_id`
and `segment_id`. `finitechat.chat.archive.v1` carries the desired archived
boolean; later accepted room order wins. Archiving is organizational only:
clients choose its presentation, the transcript remains readable and sendable,
and new messages do not implicitly restore the chat. Clients retain the
encrypted current-value projection alongside the append-only encrypted event
journal; neither local projection is a server-side source of truth.

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

For multi-device adds, one MLS Commit may add several devices from the same
account. Each added device receives its own Welcome record, but the opaque MLS
Welcome bytes may be the same batch Welcome containing secrets for all added
leaves. A device becomes a member interval at the accepted Commit seq even
before it acks the Welcome, so it can sync messages after that seq; it cannot
send until its own Welcome is claimed, activated, and acked.
