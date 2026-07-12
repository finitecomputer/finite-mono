## Issue

- Issue: #18 — persist and tombstone Pages from the Product Client
- Fixed point before session: `3c828e0`
- Worker session: `/root/ticket_18_page_persistence`
- Commit: `7fc85c4`
- Status: complete; integrated browser verification remains in the final shared pass

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
- Full suite command: deferred until all remediation tickets are integrated

## Review

- Review fixed point: `3c828e0`
- Standards findings: none
- Spec findings: the behavior matches the signing requirement. The deterministic
  guard is source-contract coverage rather than a full browser handler test.
- Worthy fixes applied: none beyond the implemented signer wiring
- Findings ignored with reasons: the live `/client` save/delete path is the
  pre-agreed final integration seam and will be exercised against a disposable
  Vault after all tickets land.

## Risks

- A focused Rust test currently expects a stale `graph-icon-button` HTML class
  that was already absent at the ticket baseline. It is outside this ticket and
  will be handled only if final verification proves it relevant.
