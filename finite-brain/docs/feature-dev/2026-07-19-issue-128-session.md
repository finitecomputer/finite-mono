# Issue #128 Session

## Issue

- Issue: #128 — fbrain: report explicit and effective Folder Access
- Fixed point before session: `fc6dad8`
- Worker session: `/root/ticket_128_effective_access`
- Commit: `b8dcb7ac4aaae2a7d70c1902f5fd53e71030ec69`
- Status: complete

## Inputs

- Spec issue: #127
- Ticket: #128
- Relevant glossary terms: Folder Access, Vault Admin, Personal Agent, Folder
  Key Grant
- Relevant ADRs: none; this slice exposes existing policy without changing it
- Prototype answer and source branch, if any: none

## Implementation

- Public interface used: `fbrain access list` text and JSON output
- Behaviors covered: explicit Folder identities remain available; effective
  identities include Organization admins and the Personal Vault owner and
  Personal Agent according to the existing Folder Key recipient policy
- `tdd` used: yes; public CLI tests were written before the implementation
- Commands run during implementation: focused `access_list` tests,
  `cargo fmt --all -- --check`, `cargo test -p finite-brain-cli`,
  `cargo clippy -p finite-brain-cli --all-targets -- -D warnings`, and
  `git diff --check`
- Full suite command: deferred to the final feature gate

## Review

- Review fixed point: `fc6dad8`
- Standards findings: none remaining
- Spec findings: none remaining
- Worthy fixes applied: preserved the existing `no folders` text response and
  the existing `accessUserIds` JSON field for compatibility
- Findings ignored with reasons: none

## Risks

- The report exposes cryptographic principals, not friendly resolved identity
  labels; friendly labels remain a separate presentation concern.
