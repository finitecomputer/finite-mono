# Issue #7 Session: Dashboard-Themed Access Workflows

## Issue

- Issue: [finitecomputer/finite-mono#7](https://github.com/finitecomputer/finite-mono/issues/7)
- Fixed point before session: `ac6564a`
- Worker session: `/root/ticket_7_access_workflows`
- Commit: `2960af7` plus review-fix follow-up
- Status: complete

## Inputs

- Spec issue: [finitecomputer/finite-mono#4](https://github.com/finitecomputer/finite-mono/issues/4)
- Ticket: `docs/feature-dev/2026-07-11-dashboard-theme-ticket-03-access-workflows.md`
- Relevant glossary terms: Dashboard-Aligned Product Theme, Product Client,
  Brain, Folder, Member Identity, Folder Key Grant, Session Lock
- Relevant ADRs: `docs/adr/0010-keep-opened-folder-keys-session-only.md`,
  `docs/adr/0014-keep-browser-and-desktop-plaintext-ephemeral.md`, and
  `docs/adr/0015-deny-plaintext-egress-by-default.md`
- Prototype answer and source branch, if any: none; the current dashboard and
  ticket #5 token foundation are the visual authorities

## Implementation

- Public interface used: the real Rust-served `/client`, exercised against the
  seeded Product Client fixture and development NIP-07 signer
- Behaviors covered: Brain and Access tabs; Folder selection and summaries;
  people, grant, share-link, invitation, email-bootstrap, Brain-administration,
  shared-Folder, cross-Brain, result, busy, warning, error, and destructive
  surfaces; existing DOM and request behavior preservation
- `tdd` used: the approved Product Client contract seam exposed one obsolete
  assertion coupled to the exact former purple decoration. It failed after the
  semantic theme implementation and was removed; the retained tests continue
  to assert the access DOM, geometry, hooks, and behavior through public seams.
- Commands run during implementation:
  - `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
  - `scripts/with-dev-env node finite-brain/scripts/verify-obsidian-product-client.mjs`
  - Playwright against `http://127.0.0.1:4037/client` at `1440x900`, with
    `colorScheme: light` and `colorScheme: dark`
  - `scripts/with-dev-env cargo test -p finite-brain-server`
  - `scripts/with-dev-env cargo fmt --all --check`
  - `scripts/with-dev-env cargo clippy -p finite-brain-server --all-targets -- -D warnings`
  - `scripts/with-dev-env cargo build -p finite-brain-app`
  - `git diff --check`

## Review

- Review fixed point: `ac6564a`
- Standards findings: two low-severity findings: the session artifact named
  stale ADR paths, and two access tokens were declared without consumers
- Spec findings: pass; no missing, incorrect, or scope-creep work
- Worthy fixes applied: corrected the ADR references to the authoritative
  current files and removed the unused tokens
- Findings ignored with reasons: none
- Re-review result: standards and spec pass with no remaining findings

## Risks

- Ticket #8 owns the final integrated responsive and full-client verification
  pass. This ticket preserves the existing access layout and breakpoint rules.
