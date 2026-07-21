# Guided Smoke Brain Reader Ledger

## Run

- Target repo: `finitecomputer/finite-brain`
- Base branch: `staging`
- Feature branch: `feature/guided-smoke-brain-reader`
- Human goal: make the temporary smoke UI less obtuse for testing accessible folders and reading their contents, while keeping the long-term direction Obsidian-like.

## Alignment

- Product decision: the Product Client should show a guided Brain Reader before raw crypto/write controls.
- Recommended behavior: after signer connection, the user should be able to load a Brain, auto-open available development Folder Key Grants, pull/decrypt accessible Pages, click folders, click pages, and read content.
- Boundary: this remains a local smoke/product-client affordance; it does not weaken server encryption or make plaintext server-visible.

## Verification Plan

- Product client deterministic JS seams.
- Targeted Rust product-client asset test.
- Local smoke server at `http://127.0.0.1:4015/client`.
- Seeded smoke DB should show accessible folders and decrypted FiniteBrain pages after the guided reader flow.

## Fixture Fill

- Added `scripts/seed-smoke-doc-pages.mjs` to populate the local smoke SQLite
  brain with FiniteBrain docs-themed encrypted Pages across every seeded
  Folder.
- The script uses the Product Client crypto helpers and `/tmp` smoke Folder Key
  manifest so fixture Pages decrypt through the same client path as normal
  Folder Objects.
