# Review Packet: Further Obsidian Parity Polish

## Scope

- Branch: `feature/guided-smoke-brain-reader`
- Base: `staging`
- PR: `finitecomputer/finite-brain#30`
- Files reviewed:
  - `crates/finite-brain-server/src/product-client.html`
  - `crates/finite-brain-server/src/product-client.css`
  - `crates/finite-brain-server/src/product-client.js`
  - `crates/finite-brain-server/src/product-client.test.js`
  - `scripts/verify-obsidian-product-client.mjs`
  - `docs/feature-dev/2026-06-24-obsidian-product-prototype-ledger.md`

## Implementation Summary

- Added safe Markdown reading-mode rendering for decrypted Pages.
- Added Reading/Source toggle while keeping source inspection available.
- Added right-rail Outgoing links and Backlinks panels derived from local
  decrypted Page links.
- Added selected Page/Brain status details to the Obsidian-like status bar.
- Fixed folder tree row layout so labels, details, and badges no longer run
  together.
- Extended deterministic tests and seeded smoke verifier for the new seams.

## Review Axes

### Standards And Security Boundary

- The renderer creates DOM nodes and uses `textContent`; it does not inject
  decrypted Markdown through `innerHTML`.
- Link context is derived client-side from already decrypted readable Pages.
  No server plaintext search, graph, or link-indexing behavior was added.
- Development-only Smoke UI remains behind Advanced client tools, not the
  primary product ribbon.
- Source mode is local-only display state and does not alter encryption,
  signing, or sync behavior.

### Product And UX

- The main Page view now behaves more like Obsidian reading mode instead of a
  raw diagnostics dump.
- The right rail now carries knowledge-base context: properties, mounts,
  outgoing links, backlinks, advanced tools, and activity.
- The status bar now provides low-noise selected Page/Brain context.
- Folder tree details and badges are visually separated after the row layout
  fix.

## Findings

### CodeRabbit Round

- CodeRabbit found three valid helper-level edge cases:
  - wiki aliases like `[[Roadmap|Q3 roadmap]]` should preserve display text;
  - graph filter-empty copy should not mask the zero-readable-pages state;
  - link context should resolve Markdown links by path and basename, not only
    title.
- All three were fixed and pinned in `product-client.test.js`.

### Local Review

- No additional blocking issues found after the CodeRabbit fixes.

## Verification

- `node --check crates/finite-brain-server/src/product-client.js`
- `node --check scripts/verify-obsidian-product-client.mjs`
- `node crates/finite-brain-server/src/product-client.test.js`
- `node scripts/verify-obsidian-product-client.mjs`
- `git diff --check`
- `cargo fmt --check`
- `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build`
- Chromium Computer Use verification against `http://127.0.0.1:4015/client`.
- Playwright fixture smoke against the live client with seeded encrypted Page
  data.
