## Issue

- Issue: #19 — surface safe Product Client action failures
- Fixed point before session: `7fc85c4`
- Worker session: `/root/ticket_19_client_feedback`
- Commit: `cae93df`
- Status: complete; integrated browser verification remains in the final shared pass

## Inputs

- Spec issue: #17
- Ticket: #19
- Relevant glossary terms: Product Client, Session Lock, Session Folder Key,
  Ephemeral Client Plaintext, Vault invitation
- Relevant ADRs: 0004, 0009, 0010, 0013, 0014
- Prototype answer and source branch, if any: none

## Implementation

- Public interface used: visible Product Client action failures, including
  existing access-panel actions
- Behaviors covered: one polite, inline status strip shows safe generic copy;
  raw failures, Invite Secrets, Folder Keys, and plaintext do not appear there;
  Session Lock clears the strip; access-panel failures remain inline without a
  duplicate global message; a stale rejected request cannot restore feedback
  after a lock
- `tdd` used: yes; deterministic closure seams cover handled access failures
  and a failure that resolves after Session Lock
- Commands run during implementation:
  - `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
  - `git diff --check`
- Full suite command: deferred until all remediation tickets are integrated

## Review

- Review fixed point: `7fc85c4`
- Initial independent standards/spec findings:
  - retain the status-feedback grid row at the compact breakpoint
  - add runtime coverage for access-result suppression
  - mark stale access failures before the epoch guard so Session Lock stays
    clear after a late rejection
- Worthy fixes applied: all three findings
- Final delta review: no remaining P0–P3 findings
- Findings deferred with reasons: a real protected-action failure is covered by
  the pre-agreed final disposable-Vault browser pass.

## Risks

- The focused Rust asset test currently expects a stale `graph-icon-button`
  HTML class that was already absent at this ticket baseline. The Graph ticket
  owns that assertion and final integration will rerun it.
