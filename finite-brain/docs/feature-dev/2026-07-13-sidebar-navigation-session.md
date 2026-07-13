# Sidebar Navigation Consolidation — Issue Session

## Issue

- Issue: PR #16 continuation — Sidebar Navigation Consolidation (AFK)
- Fixed point before session: `af12b52`
- Worker session: current Codex thread; tiny isolated low-risk UI write
- Implementation commit: `950509726e474a94246d58ef1fba5fb083b53bec`
- Status: implementation, review, and final checks complete; publish pending

## Inputs

- Spec issue: user request to remove the separate left activity rail and place
  its icons at the top of the File sidebar.
- Ticket: no separate ticket; this is a bounded continuation of the existing
  Settings/Product Client PR.
- Relevant glossary terms: Product Client, Graph View, Vault, Folder, Session
  Lock.
- Relevant ADRs: 0004 (first-party Product Client), 0005 (client-decrypted
  Graph View).
- Prototype answer and source branch, if any: none needed; the existing
  controls and their handlers supplied the runnable implementation seam.

## Implementation

- Public interface used: the Rust-served `/client` Product Client shell and
  its existing Files, Graph View, Search, Quick switcher, and Vault access
  controls.
- Behaviors covered: a single sidebar header navigation landmark contains all
  five controls; no `.app-ribbon` remains; existing click, active-state,
  `aria-pressed`, Escape focus restoration, and Settings routing continue to
  work; desktop, medium, and narrow layouts have no dead rail or overflow.
- `tdd` used: yes, at the served-client structural seam. The updated Rust
  client-asset test failed while the old rail was present, then passed after
  the header navigation and grid changes. Static Product Client contracts were
  updated alongside the rendered shape.
- Commands run during implementation:
  - `scripts/with-dev-env cargo test -p finite-brain-server --locked product_client_serves_spine_assets_and_config`
    (red before markup, then green)
  - `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env node --check finite-brain/scripts/verify-obsidian-product-client.mjs`
  - `scripts/with-dev-env cargo fmt --all --check`
  - `git diff --check`
  - disposable Rust-server Playwright smoke at 1440px, 1000px, 390px, and
    320px
- Full suite command: `scripts/with-dev-env cargo test -p finite-brain-server --locked`

## Review

- Review fixed point: `af12b52`
- Standards findings: pass after removing one redundant base Graph shell rule.
  The hard cut, target sizing, property-specific transitions, and breakpoint
  grid changes conform to the applicable standards.
- Spec findings: pass. The review asked for explicit browser confirmation of
  the narrow layout; the browser smoke confirmed it before finalization.
- Worthy fixes applied: deleted the now-identical base
  `.obsidian-shell[data-workspace-view="graph"]` declaration, leaving shared
  responsive selectors only where Graph View needs its specificity.
- Findings ignored with reasons: none.

## Risks

- The repository's seeded Product Client verifier could not execute because
  this checkout lacks its documented local Folder Key manifest. Its script
  syntax check, served-client static contracts, deterministic suite, full
  finite-brain-server test suite, and live browser smoke all passed instead.
