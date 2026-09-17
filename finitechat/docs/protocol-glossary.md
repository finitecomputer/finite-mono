# Chat protocol vocabulary

The product terms are in [CONTEXT](../CONTEXT.md); the full wire contract is
[protocol-v1](protocol-v1.md).

- **KeyPackage**: a Device's one-use MLS admission material. Server leases move
  it from available to claimed to consumed by an accepted Commit.
- **Welcome**: encrypted MLS group state for a new Device, released only after
  the corresponding membership Commit is durably accepted.
- **Commit / Epoch**: an ordered membership or key update and its resulting
  group generation. Clients apply Commits in server order.
- **Scoped idempotency**: an exact retry receives its durable original result;
  reuse with a different payload fails.
- **Membership interval**: the sequence range a Device may fetch. A cursor
  can advance over entries outside that Device's authorized interval.
- **Applied cursor**: the last server position durably consumed with the
  client's MLS state and message projection. It cannot be advanced by guesswork.
- **Delivered**: accepted by the server. It does not mean every recipient has
  fetched, decrypted, or read the message.

Server SQLite transactions and encrypted client projections are described in
[storage](storage.md). Delivery ordering is server-owned; authenticity and MLS
membership validation remain client-owned.
