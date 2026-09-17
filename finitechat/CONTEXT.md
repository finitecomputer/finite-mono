# Finite Chat

Finite Chat owns encrypted chat transport and durable ordered history.

- **Principal**: the account-level Nostr identity. Human and Agent Principals
  are distinct; profile metadata never authorizes access.
- **Device**: a separately revocable client with its own key, MLS state and
  durable store. It is not the Principal.
- **Hosted Web Device**: a Finite-operated Device serving the authenticated web
  account. Its key and state are server-held; this is not browser-local E2EE.
- **Room**: one MLS group and server-ordered delivery log. A two-person Room is
  not a separate protocol type.
- **Room Admission**: authorization and membership establishment that makes a
  Room usable on a Device. Knowing its ID does not establish membership.
- **Topic / Segment**: an application conversation lane and its context boundary
  within a Room. Neither is a new MLS group.
- **Chat**: a resumable session backed by a Segment. Archiving organizes the
  session; it does not erase history or make the transcript read-only.
- **Delivered Message**: a message accepted by the server log. Sends that fail
  before acceptance return an error; they are not a durable local outbox.
- **Activity**: transient, expiring intermediate state; it is not durable chat.
- **Cursor**: a Device's position in the ordered log. Advancing it is not proof
  that the corresponding MLS state or transcript is recoverable.
- **Canonical Agent Room**: the Core navigation binding for a user and Agent.
  It does not authorize replacing a Room, Device or Principal.
- **History Recovery**: restoring or explicitly sharing encrypted history.
  Account login, replay or a new Device alone cannot restore old decryption keys.

Keep Device identity, MLS state and durable history together across restart,
upgrade and recovery. Protocol details live in [protocol-v1](docs/protocol-v1.md);
operational recovery lives in the [Chat recovery runbook](../infra/runbooks/hosted-web-chat-recovery.md).
