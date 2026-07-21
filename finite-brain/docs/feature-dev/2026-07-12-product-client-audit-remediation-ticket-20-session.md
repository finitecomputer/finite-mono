## Issue

- Issue: #20 — fail closed after Brain authorization loss
- Fixed point before session: `65b98a7`
- Worker session: `/root/ticket_20_access_loss`
- Commit: `390c801`
- Status: complete; final shared browser verification passed

## Inputs

- Spec issue: #17
- Ticket: #20
- Relevant glossary terms: Brain, Member Identity, Session Lock, Session Folder
  Key, Ephemeral Client Plaintext, Folder
- Relevant ADRs: 0004, 0007, 0010, 0013, 0014, 0016
- Server contract: Brain membership loss is the exact `403` error reason
  `brain access required`; other authorization failures have different reasons

## Implementation

- Public interface used: Product Client active-Brain refresh/open state reads
- Behaviors covered: a confirmed membership loss on active-Brain metadata,
  export, or sync bootstrap increments the Session epoch, clears Session Folder
  Keys and temporary plaintext, locks the session, and retains a safe notice;
  late protected continuations cannot repopulate the cleared session
- Narrowness preserved: generic 401/403, stale/replayed auth, admin-only
  routes, another Brain, and independent Folder denial do not lock the whole
  Brain session
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
- Final browser proof: a disposable admin created an organization Brain and
  invited a disposable Member Identity through the Product Client. The Member
  accepted the invitation, saw the newly locked Session and safe unlock
  notice, then unlocked the organization Brain. The admin removed that Member
  through Brain People; the Member's real metadata refresh received the
  server's exact `403` reason `brain access required`, purged the Session,
  locked, and retained the Brain-change notice. The same isolated flow also
  proved immediate Lock → Unlock works after the client began adding a fresh
  signed HTTP auth nonce per protected request.

## Risks

- The client relies intentionally on the server's stable membership-loss
  reason. Widening that server contract in the future requires revisiting this
  predicate rather than treating every 403 as Brain revocation.
- The server correctly rejects reused auth event ids. Product Clients must
  mint a fresh signed auth nonce rather than weakening that replay boundary or
  relying on second-resolution timestamps.
