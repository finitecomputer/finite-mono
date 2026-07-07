# Asset Source Notes Feature Dev Ledger

## Run

- Run ID: 2026-07-03-asset-source-notes
- Loop: Plebdev Feature Dev
- Target repo: finitecomputer/finite-brain
- Base branch: main
- Feature branch: feature/asset-source-notes
- Human owner: AustinKelsay
- Started: 2026-07-03T09:20:32-0500
- Current status: PR open; CI green; CodeRabbit fallback recorded; defaults/Smoke follow-up ready; personal restricted bootstrap follow-up ready
- Skill setup status: present; root AGENTS.md, issue tracker docs, triage labels, domain docs, and ADRs exist

## Goal

Make FiniteBrain embrace the simple LLM Wiki rule for non-Markdown sources:
blobs are stored as encrypted assets, Markdown source notes explain them, and
synthesized wiki pages cite the source notes.

## Durable Artifacts

- CONTEXT updates: Asset, Source Note, and Asset Source Note Pair added
- ADRs: docs/adr/0008-store-assets-with-markdown-source-notes.md
- PRD issue: https://github.com/finitecomputer/finite-brain/issues/66
- Slice issues: #67, #68, #69, #70
- Issue sessions: docs/feature-dev/2026-07-03-issue-67-asset-source-notes-convention-session.md; docs/feature-dev/2026-07-03-issue-68-core-asset-model-session.md; docs/feature-dev/2026-07-03-issue-69-fbrain-asset-sync-session.md; docs/feature-dev/2026-07-03-issue-70-product-okf-asset-aware-session.md
- Agent briefs: finitebrain skill, generated `AGENTS.md`, generated `HUMANS.md`,
  and Folder config templates updated with Asset Source Note rules
- Review packets: docs/feature-dev/2026-07-03-issue-67-asset-source-notes-convention-review-packet.md; docs/feature-dev/2026-07-03-issue-68-core-asset-model-review-packet.md; docs/feature-dev/2026-07-03-issue-69-fbrain-asset-sync-review-packet.md; docs/feature-dev/2026-07-03-issue-70-product-okf-asset-aware-review-packet.md
- Local CodeRabbit report: docs/feature-dev/2026-07-03-asset-source-notes-coderabbit-round.md
- PR URL: https://github.com/finitecomputer/finite-brain/pull/71
- Defaults/Smoke follow-up: default Getting Started and AGENTS guidance, CLI
  empty-folder skeletons, Smoke seed content, and Smoke reset runbook now all
  describe the Asset Source Note paradigm.
- Personal restricted bootstrap follow-up: new personal Vaults now bootstrap the
  default `restricted` Folder with restricted access while treating the owner as
  the implicit restricted recipient.

## Commands

- Install: cargo metadata --no-deps --format-version 1
- Typecheck: cargo check --workspace
- Test: cargo test --workspace
- Build: cargo build --workspace
- Visual verification: node crates/finite-brain-server/src/product-client.test.js when Product Client behavior changes

## Branch Note

The feature-dev loop normally targets staging. This run starts from main because
the user explicitly requested "main hardcut" and main is the current Rust
hard-cut branch. Final PR target: main.

## Slice Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| #67 | AFK | committed | no findings | none | package checks passed |
| #68 | AFK | committed | no findings | none | package checks passed |
| #69 | AFK | committed | no findings | none | package checks passed |
| #70 | AFK | committed | CodeRabbit follow-up fixed | none | workspace checks passed |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None | | | | |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| #67 | 578b68c948533d1b5b297737b4eb87e6a2880c22 | current thread | de1ec24 | local review passed, no findings | node --check product-client.js; git diff --check; cargo fmt --check; cargo test -p finite-brain-core exposes_default_vault_pages; cargo test -p finite-brain-core working_tree_materializes_accessible_pages_and_safe_agent_conventions; cargo test -p finite-brain-cli empty_readable_folders_stay_materialized; node product-client.test.js; cargo test -p finite-brain-core; cargo test -p finite-brain-cli; cargo test -p finite-brain-server |
| #68 | de1ec24 | current thread | 1f80c57 | local review passed, no findings | cargo fmt --check; git diff --check; cargo test -p finite-brain-core working_tree_materializes_accessible_pages_and_safe_agent_conventions; cargo test -p finite-brain-core; cargo test -p finite-brain-cli |
| #69 | 1f80c57 | current thread | 63abd59 | local review passed, no findings | cargo fmt --check; git diff --check; cargo test -p finite-brain-cli scan_detects_asset_pairs_and_reports_invalid_assets; cargo test -p finite-brain-cli asset_plaintext_round_trips_with_hash_and_content_type; cargo test -p finite-brain-cli scan_detects_markdown_create_update_and_delete; cargo test -p finite-brain-core working_tree_change_intents_use_encrypted_product_client_routes; cargo test -p finite-brain-core; cargo test -p finite-brain-cli |
| #70 | 63abd59 | current thread | 191e22b | local review passed, no findings | node --check product-client.js; node product-client.test.js; cargo test -p finite-brain-server product_client_serves_spine_assets_and_config; git diff --check; cargo fmt --check; cargo test --workspace; cargo check --workspace; cargo clippy --all-targets -- -D warnings; cargo build --workspace |
| CodeRabbit | 94de197 | current thread | f1b8ff9 | local round 1 found 8 major issues; local round 2 found 0 issues; PR trigger posted with fallback recorded | coderabbit review --agent --type all --base main -c AGENTS.md; @coderabbit full review; cargo fmt --check; node --check product-client.js; node product-client.test.js; git diff --check; cargo test -p finite-brain-core; cargo test -p finite-brain-cli; cargo test -p finite-brain-server product_client_serves_spine_assets_and_config; cargo test --workspace; cargo check --workspace; cargo clippy --all-targets -- -D warnings; cargo build --workspace |
| Defaults/Smoke follow-up | c5e6e7f | current thread | branch tip | local review passed; public Smoke `/health` reachable but `/client` assets require OAuth so unauthenticated deployed bundle verification is blocked | node --check crates/finite-brain-server/src/product-client.js; node --check scripts/seed-smoke-doc-pages.mjs; node crates/finite-brain-server/src/product-client.test.js; node scripts/seed-smoke-doc-pages.mjs; node scripts/verify-obsidian-product-client.mjs; cargo fmt --check; git diff --check; cargo test -p finite-brain-core; cargo test -p finite-brain-cli; cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture; cargo check --workspace; cargo test --workspace; cargo clippy --all-targets -- -D warnings; cargo build --workspace |
| Personal restricted bootstrap follow-up | 16c0f3b | current thread | branch tip | local review passed; personal owner is implicit recipient/admin-equivalent for personal restricted Folders | cargo fmt --check; git diff --check; node --check crates/finite-brain-server/src/product-client.js; node crates/finite-brain-server/src/product-client.test.js; cargo test -p finite-brain-core; cargo test -p finite-brain-store; cargo test -p finite-brain-cli; cargo test -p finite-brain-server; cargo check --workspace; cargo test --workspace; cargo clippy --all-targets -- -D warnings; cargo build --workspace |

## Open Questions

- None.

## Escalations

- None.
