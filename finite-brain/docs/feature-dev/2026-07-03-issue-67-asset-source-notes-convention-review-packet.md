# Review Packet: Issue #67 Asset Source Notes Convention

## Scope

Issue #67 codifies the non-Markdown source convention for FiniteBrain agents:
Assets live under `raw/assets/`, and Markdown Source Notes are the durable
agent-readable handles for those Assets.

## Files To Review

- `docs/specs/finitebrain-portability-spec.md`
- `CONTEXT.md`
- `docs/adr/0008-store-assets-with-markdown-source-notes.md`
- `crates/finite-brain-core/src/lib.rs`
- `crates/finite-brain-core/src/portability.rs`
- `crates/finite-brain-core/src/portability/working_tree.rs`
- `crates/finite-brain-cli/src/admin.rs`
- `crates/finite-brain-cli/src/sync_engine.rs`
- `crates/finite-brain-server/src/product-client.js`
- `crates/finite-brain-server/src/product-client.test.js`
- `skills/finitebrain/SKILL.md`

## Review Focus

- Does the spec clearly define Page/Asset/Source Note responsibilities without
  requiring the server to inspect plaintext?
- Do generated and seeded agent instructions match the convention agents should
  follow?
- Did this slice avoid sneaking in partial Asset sync behavior that belongs in
  #68 or #69?
- Are tests targeted at the regression surface that matters for #67?

## Checks

- `node --check crates/finite-brain-server/src/product-client.js`
- `git diff --check`
- `cargo fmt --check`
- `cargo test -p finite-brain-core exposes_default_vault_pages -- --nocapture`
- `cargo test -p finite-brain-core working_tree_materializes_accessible_pages_and_safe_agent_conventions -- --nocapture`
- `cargo test -p finite-brain-cli empty_readable_folders_stay_materialized -- --nocapture`
- `node crates/finite-brain-server/src/product-client.test.js`
- `cargo test -p finite-brain-core`
- `cargo test -p finite-brain-cli`
- `cargo test -p finite-brain-server`

All checks passed.

## Review Result

Local code-review pass found no blocking or follow-up issues for #67.

Subagent review was not used because the available subagent tool is restricted
to cases where the user explicitly requests subagents.

## Known Follow-Ups

- #68: support Assets in the core portability model.
- #69: enforce Asset handling in fbrain working-tree sync.
- #70: make Product Client and OKF asset-aware.
