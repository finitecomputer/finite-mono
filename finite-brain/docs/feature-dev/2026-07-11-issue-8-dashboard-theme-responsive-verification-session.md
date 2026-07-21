# Issue #8 Session: Dashboard Theme Responsive Verification

## Issue

- Issue: [finitecomputer/finite-mono#8](https://github.com/finitecomputer/finite-mono/issues/8)
- Fixed point before session: `8e36d7a`
- Worker session: `/root/ticket_8_responsive_verification`
- Implementation commit: `296a30d` plus review-evidence follow-up
- Status: complete

## Inputs

- Spec issue: [finitecomputer/finite-mono#4](https://github.com/finitecomputer/finite-mono/issues/4)
- Ticket: `docs/feature-dev/2026-07-11-dashboard-theme-ticket-04-responsive-verification.md`
- Relevant glossary terms: Dashboard-Aligned Product Theme, Product Client,
  Session Lock, Session Folder Key, Ephemeral Client Plaintext, Graph View
- Relevant ADRs: `docs/adr/0004-build-a-first-party-product-client.md`,
  `docs/adr/0005-derive-graph-and-replay-from-client-decrypted-indexes.md`,
  `docs/adr/0010-keep-opened-folder-keys-session-only.md`, and
  `docs/adr/0014-keep-browser-and-desktop-plaintext-ephemeral.md`
- Prototype answer and source branch, if any: none; the integrated Product
  Client after tickets #5 through #7 is the implementation under review

## Implementation

- Public interface used: the real Rust-served `/client` at
  `http://127.0.0.1:4038/client`, using the seeded Product Client fixture and
  explicitly configured development NIP-07 signer
- Browser matrix: locked and resumed desktop and mobile states in light and
  dark; resumed light tablet; Files, Search, Page reading and visual/source
  editing, Graph and replay, Access and expanded forms, quick switcher,
  context menu, slash menu, empty states, disabled/status states, and keyboard
  focus
- Defect found: quick-switcher titles and their context details rendered as
  concatenated inline text because their existing wrapper had no layout
- Fix: made that existing wrapper a small grid and added explicit separation
  from the row's kind column; no HTML, JavaScript, DOM identifier, data flow,
  storage, authorization, or responsive geometry changed
- `tdd` used: no new test was appropriate for the decorative spacing defect.
  The approved public seams were run before and after the fix; no CSS-literal
  assertion was added.

## Verification

- `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
- `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
- `scripts/with-dev-env node --check finite-brain/scripts/seed-smoke-doc-pages.mjs`
- `scripts/with-dev-env node --check finite-brain/scripts/verify-obsidian-product-client.mjs`
- `scripts/with-dev-env node finite-brain/scripts/seed-smoke-doc-pages.mjs`
- `scripts/with-dev-env node finite-brain/scripts/verify-obsidian-product-client.mjs`
- `scripts/with-dev-env cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
- `scripts/with-dev-env cargo test -p finite-brain-server product_client_serves_local_dashboard_fonts -- --nocapture`
- `scripts/with-dev-env cargo fmt --all --check`
- `scripts/with-dev-env cargo test --workspace --locked`
- `scripts/with-dev-env cargo clippy --workspace --all-targets --locked -- -D warnings`
- `scripts/with-dev-env cargo build --workspace --locked`
- `git diff --check`

Seeded verifier result: 11 Folders, 54 encrypted/readable Pages, 54 projected
Graph nodes, and 41 Graph edges. The rendered Graph representative view had 12
visible nodes and 71 links.

### Browser evidence

The final 22-image curated set committed under
`docs/feature-dev/artifacts/2026-07-11-issue-8/` was regenerated from the final
Rust-served client and includes:

- `light-desktop-locked.png`
- `light-desktop-files.png`
- `light-desktop-search.png`
- `light-desktop-quick-switcher.png`
- `light-desktop-graph-replay.png`
- `light-desktop-graph-empty.png`
- `light-desktop-access-forms.png`
- `light-tablet-files.png`
- `light-mobile-locked.png`
- `light-mobile-files.png`
- `light-mobile-access.png`
- `dark-desktop-locked.png`
- `dark-desktop-page-source.png`
- `dark-desktop-context-menu.png`
- `dark-desktop-slash-menu.png`
- `dark-desktop-graph.png`
- `dark-desktop-graph-empty.png`
- `dark-desktop-access.png`
- `dark-mobile-locked.png`
- `dark-mobile-files.png`
- `dark-mobile-access.png`
- `dark-mobile-quick-switcher.png`

All matrix states had zero horizontal page overflow and zero JavaScript page
errors. Keyboard traversal exposed visible focus outlines on ribbon, Brain,
signer, and Resume controls. Mobile retained the existing 44px ribbon plus
sidebar and intentionally hidden workspace; tablet retained the ribbon,
sidebar, and workspace columns. Lock cleared rendered Pages and Resume restored
the seeded readable projection.

A final reduced-motion browser pass used Chromium media emulation through the
real `/client`: `matchMedia("(prefers-reduced-motion: reduce)").matches` was
true; representative button and Graph-node transitions and the Access-panel
entry animation computed to `1e-05s`; Graph hover and Files navigation still
worked; and Graph labels computed to the self-hosted `Funnel Sans` stack.

## Review

- Review fixed point: `8e36d7a`
- Standards findings: pass; no hard violations or baseline smells
- Spec findings: the first review could not see the scratch `/tmp` evidence and
  requested durable screenshots; after the exact paths became visible it
  passed, and the worthy evidence concern was additionally closed by committing
  the curated matrix
- Worthy fixes applied: promoted the original representative final-state
  screenshots, then expanded and regenerated the committed review artifact
  directory as the 22-image matrix listed above
- Re-review result: standards and spec pass with no remaining findings
- PR fallback review follow-up: added a global reduced-motion presentation
  override, moved Graph canvas text onto `var(--font-sans)`, regenerated and
  expanded the durable screenshot matrix, and corrected stale tracker/ADR
  references. The accepted explicit font allowlist remains intentionally
  unchanged because FiniteBrain owns its independent font assets and the small
  fixed route surface is clearer as an allowlist than a dynamic dispatcher.
- PR fallback re-review: standards had no hard findings; spec found two stale
  historical 15-image references, which were corrected to the final 22-image
  matrix. The low-severity duplicate-inventory smell is intentionally retained
  because the session record and review packet are independently consumed,
  self-contained artifacts.

## Risks

- Only deterministic seeded fixture presentation is shown in committed
  screenshots; no Folder Keys, signer secrets, or live data are present.
- No production deployment, production configuration, or live-data operation
  was performed.
