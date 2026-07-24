# Issue #129 Session

## Issue

- Issue: #129 — fbrain: make redundant Folder grants truthful and idempotent
- Fixed point before session: `5435f62`
- Worker session: `/root/ticket_129_redundant_grant`
- Commit: `ff6b5261fb228448fb8275f75752cfd641704975`
- Status: complete

## Inputs

- Spec issue: #127
- Ticket: #129
- Relevant glossary terms: effective Folder Access, Folder Key Grant, Brain
  Admin, sync record
- Relevant ADRs: none; this slice restores truthful idempotency at the existing
  signed mutation boundary
- Prototype answer and source branch, if any: none

## Implementation

- Public interface used: signed Folder grant HTTP endpoint and
  `fbrain permissions grant-folder` text and JSON output
- Behaviors covered: a current-version grant for an identity with effective
  access returns `alreadyHasAccess`; a new grant returns `granted`; only the
  winner writes access, grant, control, or sync state
- `tdd` used: yes; store, signed HTTP concurrency, and CLI tests were red first
- Commands run during implementation: `cargo test -p finite-brain-cli`,
  `cargo test -p finite-brain-server`, `cargo test -p finite-brain-store`,
  `cargo fmt --all -- --check`, affected-crate clippy with warnings denied, and
  `git diff --check`
- Full suite command: deferred to the final feature gate

## Review

- Review fixed point: `5435f62`
- Standards findings: none remaining
- Spec findings: none remaining
- Worthy fixes applied: simplified one clippy-identified boolean expression
- Findings ignored with reasons: none

## Risks

- A stale current-version grant without effective policy access is not treated
  as success; this intentionally fails rather than broadening access.
