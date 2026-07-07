# Issue #69 Session: fbrain Asset Sync Enforcement

## Issue

- GitHub: https://github.com/finitecomputer/finite-brain/issues/69
- Title: Asset source notes: enforce Asset handling in fbrain working-tree sync
- Fixed point: `1f80c57`
- Branch: `feature/asset-source-notes`
- Session: current Codex thread

## Acceptance

- The working-tree scanner detects non-Markdown files under readable Folder
  roots.
- Non-Markdown files under `raw/assets/` produce encrypted Asset write intents
  with content type, path, bytes, and content hash.
- Markdown Source Notes remain ordinary Page write intents.
- Valid Asset Source Note pairs use the normal encrypted object write route.
- Non-Markdown files outside `raw/assets/` are reported unresolved.
- Assets under `raw/assets/` without a Markdown Source Note are reported
  unresolved, including unchanged existing assets.
- Tests cover valid Asset create/update, invalid location, and missing Source
  Note enforcement.

## Changes

- Generalized working-tree write intents from Markdown-only payloads to typed
  Page/Asset content.
- Added `WorkingTreeChange::UpsertAsset` and planner validation for
  `raw/assets/` plus Source Note pairing.
- Updated `fbrain` scanning to walk all files under readable Folder roots,
  keeping `.md` files as Page changes and treating other files as Asset
  candidates.
- Implemented typed Asset plaintext encoding/decoding in the CLI using base64
  bytes, content type, size, and SHA-256 hash.
- Taught remote materialization to decode Assets into `OpenedAsset` and write
  binary projection files.
- Preserved conflicted Asset bytes after a blocked sync, matching existing
  Markdown conflict preservation.
- Updated generated wiki counts from Pages to Objects where Assets can be
  included.

## Verification

- `cargo fmt --check`
- `git diff --check`
- `cargo test -p finite-brain-cli scan_detects_asset_pairs_and_reports_invalid_assets -- --nocapture`
- `cargo test -p finite-brain-cli asset_plaintext_round_trips_with_hash_and_content_type -- --nocapture`
- `cargo test -p finite-brain-cli scan_detects_markdown_create_update_and_delete -- --nocapture`
- `cargo test -p finite-brain-core working_tree_change_intents_use_encrypted_product_client_routes -- --nocapture`
- `cargo test -p finite-brain-core`
- `cargo test -p finite-brain-cli`

All checks passed.

## Review

Local code-review pass completed with no blocking findings.

Known follow-up: #70 should make the Product Client and OKF readable flows
understand typed Assets instead of treating all opened plaintext as Page text.
