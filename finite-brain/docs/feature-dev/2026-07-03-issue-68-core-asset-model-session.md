# Issue #68 Session: Core Asset Model

## Issue

- GitHub: https://github.com/finitecomputer/finite-brain/issues/68
- Title: Asset source notes: support Assets in the core portability model
- Fixed point: `de1ec24`
- Branch: `feature/asset-source-notes`
- Session: current Codex thread

## Acceptance

- Core portability types can represent decrypted Markdown Pages and decrypted
  Assets.
- Working-tree materialization can project Markdown Page text and Asset bytes
  at their readable paths.
- Working-tree state records content type and content hash for both object
  kinds.
- Search/local wiki projections remain Page-only and do not index raw Asset
  bytes.
- Tests cover a Page and Asset in the same readable Folder.
- Existing Page-only behavior remains compatible.

## Changes

- Added `OpenedAsset` as the core representation for an already-opened
  non-Markdown Folder Object.
- Added `opened_assets` to `WorkingTreeMaterializeInput`.
- Added `binary_files` to `WorkingTreeProjection`, keeping the existing
  `files` text map intact for compatibility.
- Materialized Assets into `binary_files`, added their entries to
  `working-tree-state.json`, and computed hashes from plaintext bytes.
- Updated Page-only callers to pass an empty asset list.
- Extended the working-tree materialization test with one Page and one PDF-like
  Asset in the same readable Folder.

## Verification

- `cargo fmt --check`
- `git diff --check`
- `cargo test -p finite-brain-core working_tree_materializes_accessible_pages_and_safe_agent_conventions -- --nocapture`
- `cargo test -p finite-brain-core`
- `cargo test -p finite-brain-cli`

All checks passed.

## Review

Local code-review pass completed with no blocking findings.

Known follow-up: #69 must wire `binary_files` into `fbrain` disk writes, local
change scanning, encrypted Asset write intents, and Source Note enforcement.
