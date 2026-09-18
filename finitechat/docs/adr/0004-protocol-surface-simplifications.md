# Typed Chat protocol surface

The implemented surface has these boundaries:

1. Fanout resume/retry state is client-owned durable `LinkFanoutState`, with
   idempotent `/commits`; no public server fanout checkpoint routes exist.
2. Typed rooms accept typed routes. Raw publish is internal conformance only.
3. `/events` requires `ApplicationDeliveryPolicy` for application side effects.
4. A DM is an ordinary Room. The server enforces no unique account pair or
   separate direct-room cap. Topics remain lanes within a Room.
5. There is no lifetime 4,096-record per-sender idempotency admission cap.
6. Welcomes are claimed and activated. Failed activation stays pending and
   retryable; the linked Commit must be durable before releasing a Welcome.

The server checks structural/routing constraints; clients own cryptographic
membership and application policy. See [the current protocol](../protocol-v1.md).
