# Review Packet: Issue #68 Core Asset Model

## Scope

Issue #68 adds the core portability seam for decrypted Assets while preserving
Page-only behavior.

## Files To Review

- `crates/finite-brain-core/src/portability.rs`
- `crates/finite-brain-core/src/portability/working_tree.rs`
- `crates/finite-brain-cli/src/sync_engine.rs`

## Review Focus

- Does `OpenedAsset` model the minimum data needed for readable materialization?
- Does `WorkingTreeProjection` preserve compatibility for text callers while
  giving #69 a clear binary-file surface?
- Does materialization record correct content type and SHA-256 hash for both
  Pages and Assets?
- Does the search/local wiki layer continue to operate only over Markdown Pages?
- Is any CLI Asset sync behavior accidentally partial or misleading?

## Checks

- `cargo fmt --check`
- `git diff --check`
- `cargo test -p finite-brain-core working_tree_materializes_accessible_pages_and_safe_agent_conventions -- --nocapture`
- `cargo test -p finite-brain-core`
- `cargo test -p finite-brain-cli`

All checks passed.

## Review Result

Local code-review pass found no blocking issues.

The CLI currently passes `opened_assets: Vec::new()` and writes only the text
projection; that is intentional for this slice. #69 owns binary disk writes and
sync enforcement.
