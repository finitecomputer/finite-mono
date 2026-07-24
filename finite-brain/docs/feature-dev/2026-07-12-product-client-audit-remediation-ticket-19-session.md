## Issue

- Issue: #19 — surface safe Product Client action failures
- Fixed point before session: `7fc85c4`
- Worker session: `/root/ticket_19_client_feedback`
- Commit: `cae93df`
- Status: complete; final shared browser verification passed

## Inputs

- Spec issue: #17
- Ticket: #19
- Relevant glossary terms: Product Client, Session Lock, Session Folder Key,
  Ephemeral Client Plaintext, Brain invitation
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
- Final shared suite: `scripts/with-dev-env cargo test -p finite-brain-server --locked` passed

## Review

- Review fixed point: `7fc85c4`
- Initial independent standards/spec findings:
  - retain the status-feedback grid row at the compact breakpoint
  - add runtime coverage for access-result suppression
  - mark stale access failures before the epoch guard so Session Lock stays
    clear after a late rejection
- Worthy fixes applied: all three findings
- Final delta review: no remaining P0–P3 findings
- Final browser proof: an isolated client received a generic protected-action
  failure in the visible safe feedback region; Session Lock then cleared that
  feedback and the session-owned plaintext state.

## Risks

- The final static verifier now rejects the stale `graph-icon-button` HTML
  marker alongside the existing CSS/JS absence checks.
