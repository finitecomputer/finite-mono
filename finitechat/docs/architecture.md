# Finite Chat architecture

Clients own cryptography and MLS membership validation. The server orders and
persists opaque payloads, enforces routing and structural constraints, and
never receives room decryption keys. SaaS Account Auth is separate from the
Nostr Principal and per-Device MLS identity.

## Ownership

| Component | Responsibility |
| --- | --- |
| `finitechat-proto`, `finitechat-http` | Wire values, bounds and typed HTTP contracts |
| `finitechat-delivery` | Ordered opaque transport and conformance suite |
| `finitechat-server` | Typed admission, durable SQLite delivery, membership projections and encrypted blobs |
| `finitechat-mls`, `finitechat-client` | Nostr-rooted MLS credentials, group state, ordered sync and encrypted local persistence |
| `finitechat-core` | Application state and actions |
| `finitechat-hosted-device` | Durable server-side Device for dashboard chat |
| `finitechat-hermes`, `integrations/hermes` | Resident Rust service and thin Hermes adapter |
| `finitechat-cli` | Identity, agent onboarding and diagnostic commands |

A Room is one MLS group and one ordered delivery log. Topics and Chat segments
are application context inside that Room; they do not create independent
membership or delivery authority.

## Durability and recovery

The server's normalized SQLite transaction commits delivery and protocol side
effects together. Clients persist applied cursors with MLS state and received
message projections. Exact retries preserve message identity. A server receipt
means delivered, not read by every recipient; failed sends are not a durable
outbox. See [storage](storage.md).

Realtime hints wake ordered sync; they are not authoritative history. The Hermes
adapter consumes the resident Rust inbound stream, reconnects with bounded
backoff and acknowledges through Rust-owned inbox leases and reply routing.

Missing or ambiguous durable state fails closed. Do not choose a Room by sort
order, advance a cryptographic cursor past a failure, or mint a replacement
Device to hide missing history. A same-volume restart is not an independent
restore proof.

See the [protocol contract](protocol-v1.md), [test map](scenario-coverage.md),
[Hermes integration](../integrations/hermes/README.md), and
[production deployment](../../infra/runbooks/deploy-finitechat-server.md).
