# Issue #67 Session: Asset Source Notes Convention

## Issue

- GitHub: https://github.com/finitecomputer/finite-brain/issues/67
- Title: Asset source notes: codify the Folder Object and LLM Wiki convention
- Fixed point: `578b68c948533d1b5b297737b4eb87e6a2880c22`
- Branch: `feature/asset-source-notes`
- Session: current Codex thread

## Acceptance

- Spec defines canonical Page and Asset local object shapes without requiring
  server plaintext parsing.
- Spec documents `raw/assets/` plus Markdown Source Note pairing.
- Default Brain and generated working-tree agent instructions teach agents to
  store non-Markdown files under `raw/assets/` and pair every Asset with a
  Source Note.
- Product Client seeded onboarding copy matches the Rust-seeded copy.
- Terminology uses Asset, Source Note, and Asset Source Note Pair consistently.
- Targeted tests cover seeded strings and generated convention files.

## Changes

- Updated the portability spec with canonical Page/Asset plaintext models,
  the current versioned Markdown Page compatibility note, `raw/assets/`
  materialization guidance, OKF readable asset language, and agent-layer Source
  Note rules.
- Updated Rust and Product Client default brain pages so new brains explain
  Assets and Source Notes from `AGENTS.md`, `HUMANS.md`, Folder config, Folder
  index, and Getting Started wiki pages.
- Updated working-tree and CLI fallback folder instructions to create
  `raw/assets/.keep` and tell agents to pair Assets with Source Notes.
- Updated the packaged `finitebrain` skill with the same agent-facing policy.
- Added targeted assertions around seeded copy, generated agent instructions,
  and `raw/assets/.keep`.

## Verification

- `node --check crates/finite-brain-server/src/product-client.js`
- `git diff --check`
- `cargo fmt --check`
- `cargo test -p finite-brain-core exposes_default_brain_pages -- --nocapture`
- `cargo test -p finite-brain-core working_tree_materializes_accessible_pages_and_safe_agent_conventions -- --nocapture`
- `cargo test -p finite-brain-cli empty_readable_folders_stay_materialized -- --nocapture`
- `node crates/finite-brain-server/src/product-client.test.js`
- `cargo test -p finite-brain-core`
- `cargo test -p finite-brain-cli`
- `cargo test -p finite-brain-server`

All checks passed.

## Review

Local code-review pass completed with no findings. Subagent review was not used
because this environment only allows spawning review agents when the user
explicitly asks for subagents.

## Notes

- This slice defines and teaches the convention. Actual Asset encryption,
  materialization, and sync behavior remains for issues #68 and #69.
- The spec now defines the canonical typed Page model while explicitly noting
  the current versioned Markdown Page envelope, so the documentation does not
  overclaim current encoder behavior.
