## Issue

- Issue: #18 — persist and tombstone Pages from the Product Client
- Fixed point before session: `3c828e0`
- Worker session: `/root/ticket_18_page_persistence`
- Commit: `7fc85c4`
- Status: complete; final shared browser verification passed

## Inputs

- Spec issue: #17
- Ticket: #18
- Relevant glossary terms: Product Client, Member Identity, Page, Session Lock,
  Session Folder Key, Ephemeral Client Plaintext
- Relevant ADRs: 0004, 0010, 0014
- Prototype answer and source branch, if any: none

## Implementation

- Public interface used: Product Client Save and context-menu Delete Page
- Behaviors covered: both Page revision and tombstone builders receive the
  existing session-aware NIP-07 signer; local state changes remain after the
  signing/request succeeds, so a failure does not delete or discard local data
- `tdd` used: yes; a red deterministic source-contract guard was added before
  the two caller fixes
- Commands run during implementation:
  - `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
  - `git diff --check`
- Final shared suite: `scripts/with-dev-env cargo test -p finite-brain-server --locked` passed

## Review

- Review fixed point: `3c828e0`
- Standards findings: none
- Spec findings: the behavior matches the signing requirement. The deterministic
  guard is source-contract coverage rather than a full browser handler test.
- Worthy fixes applied: none beyond the implemented signer wiring
- Final browser proof: the isolated Product Client submitted a signed Page
  revision through the visible Save action and a signed tombstone through the
  visible Delete Page action; both server requests succeeded.

## Risks

- The final static verifier now rejects the stale `graph-icon-button` HTML
  marker alongside the existing CSS/JS absence checks.
