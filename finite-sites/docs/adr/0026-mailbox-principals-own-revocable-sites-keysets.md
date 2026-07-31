# ADR 0026: Mailbox principals own revocable Sites keysets

## Status

Accepted.

## Decision

Finite Sites owns two authorization records:

- `sites_email_principals`: a durable Sites owner named by a verified,
  deliverable mailbox;
- `sites_authorized_keys`: revocable native npubs authorized to act for that
  Sites owner, with proof provenance and revocation time.

Adding or revoking a key requires a fresh Identity Email Challenge redeemed by
an exact NIP-98 signer. Identity returns a short-lived Mailbox Proof and does
not create a Principal Link. Sites consumes that proof once and mutates only
Sites authorization.

Projects and outputs retain their originating native publisher for audit and
may additionally name a mailbox publisher. An active key for that mailbox can
exercise the mailbox owner's Sites permissions. Revoking one key does not
revoke other keys, email shares, native shares, URLs, visibility, project
collaborators, or Git credentials belonging to other principals.

Legacy `email_keys` and `principal_email_links` are reconciliation evidence,
not the new write model. Reconciliation is additive and idempotent. Ambiguous
records remain unchanged and produce durable `conflict` or `needs_proof`
repair rows. An email-shaped legacy Managed Agent NIP-05 is never mailed or
made a mailbox principal: live Identity resolution adds the corresponding
native grant while preserving the original row. When Core also verifies the
active account-to-Agent association, its verified account mailbox may create a
missing Authorized Key. Automated evidence is insert-only: a revoked key is a
durable tombstone and only a new mailbox challenge may reactivate it.

## Product boundary

This keyset is not a NIP-05 mapping, Chat recipient identity, Brain encryption
identity, Core account membership, or Google connection. Chat continues to
resolve one NIP-05 name to one npub. Brain continues to encrypt to explicit
npubs. The independent `fsite` CLI can request mailbox proof and manage the
Sites keyset without a platform hatch.

## Compatibility

Existing authorization rows remain readable and unchanged. New authorization
checks accept either the legacy path or the Sites keyset during the rollout.
No production mutation is implied by this ADR; reconciliation must first run
against synthetic/restored state and produce a reviewed report.
