# Issue #70 Session: Product Client Asset Awareness

## Issue

- GitHub: https://github.com/finitecomputer/finite-brain/issues/70
- Title: Asset source notes: make Product Client and OKF asset-aware
- Fixed point: `63abd59`
- Branch: `feature/asset-source-notes`
- Session: current Codex thread

## Acceptance

- Product Client plaintext decode helpers distinguish Pages from typed Assets.
- Page write/import behavior remains compatible.
- Asset write/import helpers preserve bytes, content type, size, content hash,
  path, and filename.
- OKF import parsing and planning preserve accessible Assets alongside Markdown
  Source Notes.
- Search, graph, replay, and reader page rows continue to use Markdown Pages
  and Source Notes rather than raw Asset bytes.
- Product Client deterministic tests cover Page compatibility, Asset decode,
  OKF Asset import, and readable-index exclusion.

## Changes

- Added a general `decodeFolderObjectPlaintext` helper and kept
  `decodeFolderObjectPagePlaintext` as a Page-only compatibility wrapper.
- Added `encodeFolderObjectAssetPlaintext` for typed Asset plaintext with
  base64 bytes and SHA-256 content hash.
- Enforced the same `raw/assets/` Asset path convention in Product Client Asset
  decode, encode, and OKF planning paths.
- Updated `openFolderObject` to return typed Page or Asset objects after
  client-side decryption.
- Extended OKF parsing to collect non-Markdown manifest objects and explicit
  `assets` entries.
- Extended OKF import planning and write preparation to create encrypted Asset
  objects while preserving existing Page conflict modes.
- Kept link rewriting Markdown-only and kept graph/search/replay/list rows
  filtered to readable Pages.
- Updated Product Client deterministic tests for typed Asset decode and OKF
  Asset round-trip.

## Verification

- `node --check crates/finite-brain-server/src/product-client.js`
- `node crates/finite-brain-server/src/product-client.test.js`
- `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`
- `git diff --check`
- `cargo fmt --check`
- `cargo test --workspace`
- `cargo check --workspace`
- `cargo clippy --all-targets -- -D warnings`
- `cargo build --workspace`

All checks passed.

## Review

Local code-review pass completed with no blocking findings.

Known follow-up: none for this issue.
