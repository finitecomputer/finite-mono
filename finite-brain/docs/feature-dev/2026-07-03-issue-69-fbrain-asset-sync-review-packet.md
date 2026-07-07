# Review Packet: Issue #69 fbrain Asset Sync Enforcement

## Scope

Issue #69 makes the `fbrain` working-tree path asset-aware while preserving the
existing Markdown Page sync path.

## Files To Review

- `crates/finite-brain-core/src/portability.rs`
- `crates/finite-brain-core/src/portability/working_tree.rs`
- `crates/finite-brain-cli/src/sync_engine.rs`

## Review Focus

- Does the scanner detect all relevant non-Markdown files without treating
  generated `.keep` convention files as assets?
- Are invalid Asset locations and missing Source Notes reported as unresolved
  instead of silently ignored?
- Does the write intent carry enough typed content for the CLI to encrypt Page
  and Asset plaintext correctly?
- Are conflicted Asset bytes preserved after a blocked sync?
- Does the existing Markdown Page behavior remain compatible?

## Checks

- `cargo fmt --check`
- `git diff --check`
- `cargo test -p finite-brain-cli scan_detects_asset_pairs_and_reports_invalid_assets -- --nocapture`
- `cargo test -p finite-brain-cli asset_plaintext_round_trips_with_hash_and_content_type -- --nocapture`
- `cargo test -p finite-brain-cli scan_detects_markdown_create_update_and_delete -- --nocapture`
- `cargo test -p finite-brain-core working_tree_change_intents_use_encrypted_product_client_routes -- --nocapture`
- `cargo test -p finite-brain-core`
- `cargo test -p finite-brain-cli`

All checks passed.

## Review Result

Local code-review pass found no blocking issues.

The Source Note matching rule is intentionally simple: a Markdown page in the
same Folder must mention the Folder-local asset path, such as
`raw/assets/source.pdf`.
