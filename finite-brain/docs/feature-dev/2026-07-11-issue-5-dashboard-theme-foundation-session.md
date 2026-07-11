# Issue #5 Session: Dashboard Theme Foundation

## Issue

- Issue: [finitecomputer/finite-mono#5](https://github.com/finitecomputer/finite-mono/issues/5)
- Fixed point before session: `6c32dbb`
- Worker session: `/root/ticket_5_theme_foundation`
- Commit: `aa3b7a1` plus review fix follow-up
- Status: complete

## Inputs

- Spec issue: [finitecomputer/finite-mono#4](https://github.com/finitecomputer/finite-mono/issues/4)
- Ticket: `docs/feature-dev/2026-07-11-dashboard-theme-ticket-01-foundation.md`
- Relevant glossary terms: Dashboard-Aligned Product Theme, Product Client,
  Session Lock, Session Folder Key, Ephemeral Client Plaintext
- Relevant ADRs: `docs/adr/0008-clear-browser-secrets-on-session-boundaries.md`,
  `docs/adr/0015-deny-automatic-plaintext-egress.md`
- Prototype answer and source branch, if any: none; the current dashboard
  source is the visual authority

## Implementation

- Public interface used: the real Rust-served `/client`, `/client/app.css`,
  and explicit `/client/fonts/*.ttf` routes
- Behaviors covered: exact local font delivery; system light/dark presentation;
  shell, ribbon, Vault controls, Session Lock, common control/focus/disabled
  states, and locked workspace presentation; preservation of the existing DOM,
  JavaScript, geometry, and responsive breakpoint behavior
- `tdd` used: yes. The font asset route test failed with `404` before the
  routes/assets existed, then passed after implementation. The stylesheet
  contract assertions failed before the local font faces and theme tokens were
  introduced, then passed after the presentation layer was implemented.
- Commands run during implementation:
  - `scripts/with-dev-env cargo test -p finite-brain-server product_client_serves_local_dashboard_fonts -- --nocapture`
  - `scripts/with-dev-env cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `scripts/with-dev-env cargo test -p finite-brain-server product_client_serves_ -- --nocapture`
  - `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
  - `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env node finite-brain/scripts/verify-obsidian-product-client.mjs`
  - Playwright against `http://127.0.0.1:4035/client` at `1440x900` and
    `390x844`, each with `colorScheme: dark` and `colorScheme: light`
  - `scripts/with-dev-env cargo test -p finite-brain-server` (40 passed)
  - `scripts/with-dev-env cargo fmt --all --check`
  - `scripts/with-dev-env cargo clippy -p finite-brain-server --all-targets -- -D warnings`
  - `scripts/with-dev-env cargo build -p finite-brain-app`
  - `git diff --check`
- Full suite command: `scripts/with-dev-env cargo test -p finite-brain-server`
  (passed after review fixes)

## Review

- Review fixed point: `6c32dbb`
- Standards findings: repeatable Node commands in the ledger and ticket
  artifacts omitted the repository's required `scripts/with-dev-env` wrapper;
  a judgement-call duplication smell noted the intentionally explicit font
  route/handler declarations
- Spec findings: pass; no missing, partial, incorrect, or scope-creep findings
- Worthy fixes applied: browser review found and fixed hard-coded dark Page and
  editor-drawer surfaces that initially made the locked light workspace black;
  the recorded Node commands were wrapped and rerun through the Nix environment
- Findings ignored with reasons: the bounded explicit font route declarations
  remain because the approved contract calls for explicit public routes and a
  dynamic path dispatcher would make this small static allowlist less direct

## Risks

- Later tickets still need to migrate knowledge-workspace and access-specific
  decorative literals onto the shared token layer. This ticket intentionally
  establishes their token foundation without taking their scoped work.
