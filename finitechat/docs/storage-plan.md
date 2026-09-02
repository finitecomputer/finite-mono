# Storage Plan

## Decision

Use three storage profiles:

- Client/device: encrypted local SQLite for MLS client state, pending outbound
  work, inbound event cache, and device-linking state.
- Local/dev, first server proof, and self-hosted single-node production: SQLite.
- Hosted multi-node room server: Postgres.

SQLite is not "just client testing" in this repo. It is the first durable
server proof because it forces every reducer invariant through a transaction and
a restart boundary. For self-hosted Finite Chat, the single-writer property is a
strength: it matches the room-server sequencer model, keeps deployment small,
and makes transaction order easy to reason about.

Postgres is still the production target for hosted multi-node room servers
because it has stronger operational fit for multiple API processes, queue
workers, retention jobs, migrations, backups, observability, and canary
rollback.

## Current SQLite Scope

`finitechat-client` has the first local client SQLite store:

- `client_device_states`
- `client_app_rooms`
- `client_app_state`
- `client_app_messages`
- `client_app_profiles`

`client_device_states` stores one encrypted binary snapshot per
account/device. The plaintext snapshot contains the Nostr-rooted device
profile metadata needed to reload, the Finite Chat room id to MLS group id
mapping, the per-room applied server cursor, pending claimed Welcome payloads,
durable link-fanout plans and prepared Commit replay values, and OpenMLS
storage records for signer, group, and message-secret state.

`client_app_rooms` stores encrypted local application-room metadata that is not
MLS state but is required to render the chat list and resume room work after
restart. Rows are scoped to the owning account/device and keyed by `room_id`;
the payload stores the room display name, visible lifecycle/status, pending
join invite URL, and creator-owned invite watch URL. The row payload is sealed
with the same client-store key as the device snapshot, and the AEAD AAD binds
owner and room id so copied or tampered rows fail closed on load. Room
creation persists the device state and app-room metadata in one SQLite
transaction; scan, PIN submission, join-finalization, and invite-creation
transitions persist their visible room projection before returning to Swift.
Startup reconstructs the chat list from the union of persisted app-room rows
and MLS-known rooms, because a pending invite can be user-visible before the
device has joined the MLS group.

`client_app_state` stores encrypted single-row local application state owned by
Rust. It carries `selected_room_id`, which lets a phone force-close and reopen
into the same selected transcript before any network sync, plus local
revoked-device marks used to render the account device list after relaunch.
Swift mirrors this field into native navigation; it does not decide which room
is selected, persist chat routing, or own device-list repair state on its own.
The AEAD AAD binds the row to the owning account/device so copied state fails
closed.

`client_app_messages` stores the bounded local application-message projection
that powers chat lists and room views. It is not an authoritative server log
and it is not a profile-style cache: each row is scoped to the owning
account/device, keyed by `(room_id, message_id)`, ordered by local insertion,
and contains the authenticated sender plus decrypted application plaintext
encrypted at rest. The structured row also stores the server-projected message
timestamp so Rust can rebuild raw and display timestamp fields without asking
Swift to infer chat semantics. The row plaintext is sealed with the same
client-store key as the device snapshot, and the AEAD AAD binds owner, room
id, sequence, message id, and authenticated sender so copied or tampered rows
fail closed on load. The table has owner and owner/room/seq indexes so startup
can load a bounded recent projection without replaying room history.

There is no durable client outbox. An own chat send is ratcheted, signed, and
appended in one synchronous step: on `EventAccepted` the runtime writes the
accepted app message/event projection with outbound delivery marked delivered,
and on any failure (transport, server rejection, idempotency conflict) it
returns the typed error to the caller and stores nothing, so nothing is queued,
drained, or retried later. Offline composing that survives a force-close may
return as an enhancement when mobile/desktop clients ship. A send rejection is
not a room lifecycle transition: only confirmed missing or unusable local MLS
membership projects the room as unavailable on this device. Inbound messages do
not carry outbound delivery state. Read receipts may change the rendered
checkmark for a delivered outbound message, but they do not alter delivery
state. Writer opens drop a `client_app_outbox` table left by earlier builds.

Attachment sends follow the same contract. Before a sent
attachment message exists, the runtime validates the local file, encrypts it,
uploads ciphertext, and then sends the encrypted blob-reference payload. If
upload is unreachable, the attachment send fails immediately with transient
feedback and creates no sent bubble, no outbound delivery state, and no stored
send. SQLite must not store plaintext attachment bytes.
After the accepted encrypted blob-reference message exists, the local plaintext
cache is not delivery state. If plaintext is absent on a later open, the
runtime treats it as an attachment cache miss. It waits for an explicit
tap/download action before fetching and decrypting from the blob reference, and
reports attachment-view unavailable/download error if that fails while the
message remains delivered.
Before server acceptance, relaunch must not show a sent attachment bubble;
after server acceptance, relaunch must restore the delivered blob-reference
message. There is no durable undelivered offline media row.
Attachment upload/download progress is not inferred by Swift. Until the blob
transport reports real byte counts through Rust projection, the app exposes
only coarse Rust-owned in-flight activity; `0` progress is rendered as
indeterminate activity, not a fabricated percentage. Byte-level progress remains
omitted.
If local attachment staging or delivered attachment cache metadata is missing
or fails validation, the runtime treats that as local attachment
unavailable/cache-miss state, not a server delivery failure. It does not change
room lifecycle, does not become a failed send, and normal test cleanup must not
manufacture it; corrupted-state repair tests must name the fixture explicitly.

The wrapping key is derived from the user's Nostr secret and device id using
HKDF with Finite Chat domain separation. SQLite metadata, row counts, WAL
behavior, and account/device lookup ids remain visible to the local machine;
the client store is application-level SQLite encryption, not SQLCipher.

Client code should use store-backed operations for persisting claimed Welcomes,
Welcome activation, link-fanout preparation/completion, and ordered-log
application. These operations persist MLS state and the applied cursor together,
so a restart after a successful write can skip replayed log entries instead of
asking OpenMLS to reprocess an already-applied Commit or application message.
Persisting claimed Welcomes also means a device can recover after the server has
moved a Welcome from released to claimed, but before local activation has
completed. Persisting prepared fanout Commits means a device can restart after
creating local pending MLS state and still submit the exact server request that
matches that pending Commit.

Received application messages are inserted in the same SQLite transaction that
persists the device cursor that consumed them, including the timestamp carried
by the server room-log entry. Own sends are written only after the server
append is accepted, as the accepted app message/event projection including the
accepted timestamp; a send that is not accepted leaves no row.
Swift and the app runtime render the Rust state and do not own persistence or
timestamp formatting. Startup reads the bounded SQLite
app-state, room, message, and profile projections before network sync;
transport failure during startup must return the saved chat list and selected
transcript as offline local state, not an empty UI. Full room-history sync
remains a repair/recovery path, not the ordinary way the UI gets messages after
launch. Regression coverage must include a remote-synced message that survives
force-close and offline relaunch, because that is the user-visible chat
contract, not an implementation detail.

On iOS, launch overrides for server URL or device id are temporary unless the
caller explicitly supplies `--finitechat-persist-launch-config`. The app may
write the resolved runtime device id back to its config file during normal
startup, but a temporary Xcode/RMP server override must not become product
configuration after the Rust runtime opens. Harnesses that need same-config
force-close relaunches must opt into persistence explicitly. Throwaway
diagnostics must use the explicit transient store flag; ordinary Home Screen
launches are the product path.
Normal startup no longer scans or migrates pre-release `FiniteChat/<device>`
app-support stores. Those old dev stores are reset-only inputs; product launch
opens `FiniteChatStore` or an explicit `FiniteChatTransient/<device>` root, and
`RuntimeConfigTests` assert that legacy directories are ignored rather than
recovered.

Normal runtime startup also no longer imports pre-release `app-messages.json`
or rewrites old app projection tables into the current encrypted shape. If
`client_app_messages` or `client_app_events` already exists with a plaintext
column, or without the current timestamp, nonce, and ciphertext columns, client
store open fails closed with a reset-store error. The fix for that dev/test
state is the documented whole-store reset, not a row rewrite or compatibility
migration. App projection timestamp columns must not carry old `DEFAULT 0`
schema defaults, and encrypted app projection tables must not carry extra
columns beyond the current schema; those schemas fail closed as reset-only
state.
Legacy unencrypted client-store tables (`client_openmls_storage`,
`client_rooms`, `client_profiles`) are reset-only as well, even when empty.
Store open fails closed instead of dropping them as compatibility cleanup.
Encrypted app-room metadata is also strict: rows missing the current
`state`/`status`/`local_read_seq` fields, or carrying the old `Offline` /
`NeedsAttention` lifecycle payloads, fail closed instead of defaulting into a
connected room.
Encrypted app-state and app-profile metadata must carry the current
selected-room, revoked-device, and stale-profile fields. Missing fields fail
closed instead of being silently defaulted into product state.

Production still needs the unlock policy that decides whether the Nostr key
comes from OS keychain, user passphrase, hardware-backed storage, or an
already-unlocked finitecomputer runtime.

`finitechat-blob` owns the first attachment/blob boundary. It does not add a
new database yet. Clients encrypt attachment bytes with per-attachment
AES-256-GCM key material, upload only ciphertext to a Blossom-compatible
content-addressed store, and put the blob reference, hashes, key, nonce,
filename, MIME type, and dimensions inside the encrypted application payload.
The blob store sees ciphertext bytes, a ciphertext content type, ciphertext
hash, object size, URL, timing, and requester metadata. It does not receive the
plaintext filename or MIME type in the Finite Chat abstraction.

Debug logs follow the same boundary. They may record blob reference ids,
hashes, sizes, timing, transfer states, and error categories, but they must not
include plaintext message bodies, attachment bytes, plaintext filenames, or
plaintext media metadata. Export is explicit local copy/share only before first
release; the app must not automatically upload attachment-related diagnostics
or send them through telemetry.

Unit proof uses an in-memory content-addressed store plus a Blossom-shaped HTTP
request/response boundary so the cryptographic and metadata invariants are
tested without adding a networking dependency. If attachment UI ships in v1,
product proof must use the real finitechat-server `/upload` and
`/blobs/{sha256}` routes backed by durable server blob storage. Offline
attachment send still fails fast because upload is required. The finitecomputer
integration should replace runtime-local attachment bytes by executing that
boundary against real Blossom-compatible storage, not by changing the encrypted
reference shape.

`finitechat-store` now uses normalized SQLite tables that mirror the intended
Postgres shape:

- `rooms`
- `direct_rooms`
- `devices`
- `room_log_entries`
- `room_membership_intervals`
- `key_packages`
- `welcomes`
- `link_sessions`
- `idempotency_records`

The store still uses SQLite for local/dev and first-server proof, but the
authoritative state layout is no longer a JSON snapshot.
SQLite connections set `journal_mode = WAL` and `synchronous = FULL`
explicitly so tests do not inherit durability behavior from library defaults.
Write transactions use `BEGIN IMMEDIATE`, and room-head updates include the
epoch and sequence they consumed. Commit rows also have a partial unique index
on `(room_id, epoch)` for `kind = 'commit'`.

It proves:

- accepted and rejected idempotency responses survive reopen;
- idempotency capacity rejects new mutations without breaking existing replay;
- Commit side effects are persisted together;
- Commit transaction rollback after intermediate side effects converges on retry;
- same-epoch Commit losers cannot create duplicate log rows or Welcomes;
- device revocation status survives reopen and blocks new server-mediated
  device material or mutations;
- randomized in-memory-vs-SQLite operation sequences stay state-equivalent
  across mixed accepted/rejected mutations and exact idempotent retries;
- KeyPackage leases, consumption, and opaque payload bytes survive reopen;
- account-level KeyPackage fanout claims return one available package per
  device and persist the leases across reopen;
- Welcome release, claim, ack, failure, resume states, and opaque payload bytes
  survive reopen;
- direct-room identity constraints survive reopen;
- link-session state survives reopen.

The SQLite shape flushed out two production-schema requirements:

- membership rows need stable string/table keys, not serialized struct keys;
- replayable rejects require durable serialization of typed engine errors.

The only JSON stored by the server store is `idempotency_records.response_json`,
which is a bounded typed replay value. Room state, message ordering,
membership, device status, KeyPackages, Welcomes, and link sessions are schema
rows. The client store uses a bounded binary snapshot because OpenMLS storage
is already a local opaque provider snapshot, and encrypting it as one unit
avoids leaking OpenMLS storage-key names into SQLite indexes.
Device status is deliberately small: active or revoked. It is not identity
authority and does not replace client-side Nostr credential verification, but
it gives the server a durable way to block revoked installs from new
KeyPackages, Welcome activation, sends, Commits, and future add-device Commits.
KeyPackage bytes are a `BLOB` column on `key_packages`. Welcome payload and
ratchet-tree bytes are `BLOB` columns on `welcomes`; the server keeps them
opaque and only enforces protocol bounds before mutation.
Account-level KeyPackage fanout is a bounded query over indexed schema state,
not a JSON scan: it claims one available package per device and leaves extra
packages for later group invites or retry flows.
Account-room discovery follows the same rule. The server pages over indexed
current membership rows for an account, returns room head metadata plus the
account's current/pending devices, and rejects duplicate current/pending device
adds before consuming another KeyPackage or releasing another Welcome. The
general devices-per-account-per-room cap keeps group-room fanout bounded, while
direct rooms keep the tighter direct-room cap.

## Production Schema Direction

The Postgres schema should keep this same model:

- `rooms`
- `direct_rooms`
- `devices`
- `room_log_entries`
- `room_membership_intervals`
- `key_packages`
- `welcomes`
- `idempotency_records`
- `link_sessions`
- `repair_reports`
- `push_outbox`

The critical transaction remains the same:

1. lock room row;
2. validate expected epoch and sender membership;
3. validate KeyPackage leases for adds;
4. validate staged Welcome payloads for adds;
5. append exactly one log entry;
6. advance room epoch;
7. update membership interval cache;
8. consume KeyPackages;
9. release Welcomes with opaque Welcome and ratchet-tree bytes;
10. persist idempotency response;
11. enqueue opaque push wakes.

The mutation path must not reconstruct the full room log. Full log validation is
for read/replay paths; append and Commit validation use the indexed room head,
membership intervals, and idempotency rows needed for that single mutation.

Rejected mutations admitted under an idempotency key must also persist their
typed rejection result so client retries receive the same answer after restart.
