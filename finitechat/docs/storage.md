# Chat storage

Production server and client stores use SQLite. The server stores opaque
ciphertext and ordered protocol state; clients own decryption and MLS state.

## Client durability

The client store seals its OpenMLS snapshot and application projections with a
key derived from the Nostr secret and Device id. Authenticated associated data
binds records to their owner and record identity. This is application-level
encryption, not SQLCipher: row counts, lookup identifiers, and WAL metadata
remain visible locally.

- MLS state and the applied server cursor persist together. Received message
  projections commit with the cursor that consumed them.
- Claimed Welcomes and prepared fanout Commits persist across restart so retry
  resumes the same operation rather than regenerating cryptographic state.
- Encrypted room, message, profile, and selected-room projections support local
  reopening before network sync; an offline server must not erase saved history.
- Ordinary chat has no durable outbox. Own sends become delivered only after server
  acceptance; rejected sends create no accepted message row or later retry work.
- Attachments upload encrypted bytes before sending the blob reference. SQLite
  never stores plaintext attachment bytes. A later cache miss does not undo
  delivery or change room membership.
- Malformed or unsupported client state fails closed. Do not repair durable MLS
  state by skipping cursors, inventing Devices, or editing individual rows.

### Hermes command refusals

The resident sidecar owns command refusals through the versioned
`refuse-command-v1` operation. It freezes the inbound route and explanation in
a `refusal_v1` lease in `hermes-inbox.json`, and saves the exact prepared encrypted
request before submission. A retry first looks for acceptance; an unaccepted
request is replayed only while its group, epoch, saved own-send mark, and newest
minted id prove it has not been overtaken by later sender traffic. Replay
requires a successful sync through the observed server head; a bounded partial sync or a delivery error is insufficient. Missing replay evidence fails
closed. The existing application-effect receipt recovers an accepted reply even
after the room changes epoch. Core restores the sender's history from the saved
plaintext. Only durable acceptance permits inbox settlement; the ack record
and reply receipt are written together. Ordinary sends keep their existing path.

The private inbox contains plaintext, including the saved refusal, and retains
its existing mode-0600 atomic replacement boundary. The inbox and client crypto
store must be restored as one coherent Recovery Set. An isolated older inbox
restore can forget accepted work and is not a supported recovery procedure.

Pending refusals are not leased to Hermes, expired, or removed by ordinary
ack/release calls. Up to 32 are retained; one recovery worker retries due work
with capped backoff. Readiness reports the pending count and last errors.
Unaccepted requests overtaken by sender traffic or an epoch change remain
blocked work; ordinary sends are not held behind them. They are never silently
replaced or consumed. The runtime has no discard or re-encryption operation to
drain these entries; a future repair must preserve receiver decryptability and
resolve uncertain acceptance before changing delivery identity.

An old reader rejects `refusal_v1`. A downgrade to an incapable runtime must
therefore drain all protected entries before replacing the capable runtime.
If delivery cannot finish, repair forward; do not clear the inbox to permit a
downgrade. Once drained, the pending leases are legacy-readable and the old
reader may safely ignore the optional accepted receipt on the ack ring.

For hosted Kata upgrades, deploy the Runner containing the inbox compatibility
guard before the refusal-capable Agent Runtime image. The image advertises its
reader through `computer.finite.chat.inbox_reader=refusal-v2`. The prepared
wrapper is version 2 with a required own-send watermark (zero is valid). Earlier
unreleased version-1 wrappers are unsupported. Prepared-format changes must
change the reader capability too; the unchanged `refusal_v1` lease tag identifies
the operation, not its nested encrypted payload format. Runner checks the
quiesced inbox before replacement and before rollback cleanup; blocked rollback
preserves the capable handle. The host reads tenant-owned inbox children through
anchored descriptors without following symlinks, rejects special files, and
bounds input to 64 MiB. This is an operator limit, not a bound guaranteed by the
sidecar: its ordinary inbox backlog is unbounded. Oversized or incompatible
state refuses handoff and requires repair; this check does not establish that
the existing fleet fits the limit. An older Runner does not provide this guard and
is not a supported orchestrator for this rollout. Phala upgrades remain disabled.

## Server durability

`finitechat-server/src/store/` owns one SQLite writer and a bounded pool of
query-only readers. Mutations use `BEGIN IMMEDIATE`. WAL synchronous mode
defaults to `NORMAL`; `FINITECHAT_SQLITE_SYNCHRONOUS=FULL` selects the stronger
commit flush policy. Do not equate the default with zero acknowledged-message
loss after host power failure.

Normalized delivery rows allocate sequences under the route write lock and
admit one Commit per source epoch. A typed Commit transaction persists the
Commit and Welcome delivery entries, account-room directory changes,
KeyPackage consumption and publish idempotency rows together; candidate
in-memory state is installed only after commit.

Room membership checkpoints are derived state. Startup replays delivery tails
and fails closed on inconsistent checkpoints; retained historical readers and
startup conversions remain necessary for supported older stores.

Persistence and conformance tests in `crates/finitechat-server/tests/` and
client reopen tests in `crates/finitechat-client/tests/` own these contracts.
Follow [Hosted Web Chat recovery](../../infra/runbooks/hosted-web-chat-recovery.md)
for a consistent Recovery Set; a live SQLite file copy is insufficient.
