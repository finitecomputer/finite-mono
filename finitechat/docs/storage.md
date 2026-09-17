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
- There is no durable outbox. Own sends become delivered only after server
  acceptance; rejected sends create no accepted message row or later retry work.
- Attachments upload encrypted bytes before sending the blob reference. SQLite
  never stores plaintext attachment bytes. A later cache miss does not undo
  delivery or change room membership.
- Malformed or unsupported client state fails closed. Do not repair durable MLS
  state by skipping cursors, inventing Devices, or editing individual rows.

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
