## Issue

- Issue: #23 — remove the hidden Graph filter affordance
- Fixed point before session: `b946ed6`
- Worker session: `/root/ticket_23_graph_filter`
- Commit: `72b5d82`
- Status: complete; final shared browser verification passed

## Inputs

- Spec issue: #17
- Ticket: #23
- Relevant glossary terms: Graph View, local graph, decrypted accessible Page,
  Session Lock, workspace controls
- Relevant ADRs: 0004, 0005, 0010, 0013, 0014
- Product truth: Graph View is a client-side projection of currently unlocked,
  decrypted, accessible Pages and must not imply server plaintext indexing

## Implementation

- Public interface used: Graph header, context-menu Show in Graph View, and
  floating graph controls
- Behaviors covered: hidden title filter markup/CSS/bindings/session input,
  context injection, filtering projection, and filter-specific statistics/empty
  state were removed; Graph View now renders the full local accessible graph
- Existing real controls retained: Zoom in, Zoom out, Reset zoom, Full screen
- Asset contracts updated: deterministic tests, Rust served-asset assertions,
  and static fixture verifier assert the filter is absent and floating controls
  are present
- `tdd` used: yes; red absence assertions preceded the full removal
- Commands run during implementation:
  - `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
  - `scripts/with-dev-env node --check finite-brain/scripts/verify-obsidian-product-client.mjs`
  - isolated fixture seed and `verify-obsidian-product-client.mjs` (11 Folders,
    54 Pages, 41 edges, 54 nodes)
  - `scripts/with-dev-env cargo fmt --check`
  - `scripts/with-dev-env cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
  - `scripts/with-dev-env cargo test -p finite-brain-server`
  - `git diff --check`

## Review

- Review fixed point: `b946ed6`
- Standards review: one P2 found legacy filter-era test call shapes; fixed and
  rechecked clean
- Spec review: pass; the graph continues to use decrypted accessible Pages only
  and the verifier cleanup removes only markers already absent at baseline
- Final browser proof: isolated desktop and supported narrow (768×844) browser
  checks confirmed no Graph header filter/action, fully fitted Graph stats and
  title, and reachable Zoom, Reset, and Full screen controls. The narrow
  screenshot artifact remains outside the repository because it contains only
  disposable local test content.

## Risks

- There is intentionally no retained invisible filtering API. A future Graph
  search/filter feature must be designed as a visible, accessible interaction.
