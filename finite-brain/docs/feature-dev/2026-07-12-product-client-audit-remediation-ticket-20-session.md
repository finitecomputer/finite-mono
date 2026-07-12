## Issue

- Issue: #20 — fail closed after Vault authorization loss
- Fixed point before session: `65b98a7`
- Worker session: `/root/ticket_20_access_loss`
- Commit: `390c801`
- Status: complete; integrated browser verification remains in the final shared pass

## Inputs

- Spec issue: #17
- Ticket: #20
- Relevant glossary terms: Vault, Member Identity, Session Lock, Session Folder
  Key, Ephemeral Client Plaintext, Folder
- Relevant ADRs: 0004, 0007, 0010, 0013, 0014, 0016
- Server contract: Vault membership loss is the exact `403` error reason
  `vault access required`; other authorization failures have different reasons

## Implementation

- Public interface used: Product Client active-Vault refresh/open state reads
- Behaviors covered: a confirmed membership loss on active-Vault metadata,
  export, or sync bootstrap increments the Session epoch, clears Session Folder
  Keys and temporary plaintext, locks the session, and retains a safe notice;
  late protected continuations cannot repopulate the cleared session
- Narrowness preserved: generic 401/403, stale/replayed auth, admin-only
  routes, another Vault, and independent Folder denial do not lock the whole
  Vault session
- `tdd` used: yes; a deterministic predicate/state-transition test was added
  before the protected-request implementation
- Commands run during implementation:
  - `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
  - `git diff --check`
- Full suite command: deferred until all remediation tickets are integrated

## Review

- Review fixed point: `65b98a7`
- Review method: focused self-review; independent slot capacity was consumed by
  concurrent read-only ticket analyses
- Findings: none. The error predicate is exact and only invoked after the
  response has been authenticated and correlated with the active session epoch.
- Final browser proof: an isolated local Product Client receives the exact
  active-Vault metadata `403` reason `vault access required`; it purges the
  session, locks, and retains the Vault-change notice. The same disposable
  flow also proved immediate Lock → Unlock works after the client began adding
  a fresh signed HTTP auth nonce per protected request.

## Risks

- The client relies intentionally on the server's stable membership-loss
  reason. Widening that server contract in the future requires revisiting this
  predicate rather than treating every 403 as Vault revocation.
- The server correctly rejects reused auth event ids. Product Clients must
  mint a fresh signed auth nonce rather than weakening that replay boundary or
  relying on second-resolution timestamps.
