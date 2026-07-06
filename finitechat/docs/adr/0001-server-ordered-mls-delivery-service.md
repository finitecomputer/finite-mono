# ADR 0001: Server-Ordered MLS Delivery Service

Status: accepted for v1 seed

## Context

Pika/Marmot proved MLS chat over Nostr can work, but the brittle point is Commit
ordering. MLS requires all members to process handshake operations in the same
epoch order. Eventually consistent relays do not provide a single canonical
room sequence.

Finitecomputer already has a centralized relay/control-plane path for hosted
runtime chat. That makes a server-ordered Delivery Service a natural fit.

## Decision

Finite Chat v1 uses one ordering-authoritative room server per room. The server
is not authoritative for identity. Clients verify account and device identity
from Nostr-rooted MLS credentials.

The server:

- assigns monotonic per-room sequence numbers;
- stores opaque MLS messages;
- leases KeyPackages;
- releases Welcomes only after durable Commit acceptance;
- maintains an untrusted membership interval cache for access and push routing.

Clients:

- own MLS cryptographic state;
- validate device credentials and application policy;
- reject identity claims that are not proven by the Nostr account key;
- advance local MLS state only after observing accepted log entries;
- enter `NeedsRepair` if an accepted Commit is invalid or disagrees with the
  submitted membership delta.

## Consequences

Positive:

- removes relay-derived Commit consensus from v1;
- makes idempotency and crash recovery testable;
- fits finitecomputer's hosted runtime architecture;
- keeps message plaintext away from the server.

Negative:

- leaks more metadata to the room server than a relay-first design;
- requires durable server storage;
- federation and server migration become explicit future workflows;
- total device loss still needs re-add or a separate encrypted backup system.
