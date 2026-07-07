# Goal Ledger: Rust v1 Product Parity

## Run

- Run ID: `2026-06-24-rust-v1-product-parity`
- Loop: Feature Dev
- Target repo: `finitecomputer/finite-brain`
- Base branch: `staging`
- Feature branch: `feature/rust-portable-v1-core`
- Human owner: Austin
- Started: 2026-06-24
- Current status: all product parity slice issues through `finite-brain#22`
  implemented, reviewed, verified, and committed; branch push and PR refresh
  are next
- Skill setup status: `AGENTS.md`, `CONTEXT.md`, and `docs/agents/*` already exist

## Goal

Get the remaining FiniteBrain v1 parity work done end to end on the Rust
implementation, with no backwards compatibility or legacy migration bridge.
This is a hard-cut continuation from the Rust Portable v1 core PR.

## Durable Artifacts

- CONTEXT updates: root `CONTEXT.md` now distinguishes `Product Client`,
  `Product Client Spine`, `Graph View`, `Graph Replay`, `OKF Import
  Execution`, `Vault Working Tree`, `Smoke UI`, and `Hard Cut`.
- ADRs: `docs/adr/0004-build-a-first-party-product-client.md`,
  `docs/adr/0005-derive-graph-and-replay-from-client-decrypted-indexes.md`,
  `docs/adr/0006-keep-okf-import-execution-client-owned.md`
- PRD issue: `finitecomputer/finite-brain#16`
- Slice issues:
  - `finitecomputer/finite-brain#17` Product Client spine with NIP-07 auth states
  - `finitecomputer/finite-brain#18` Client-side Page decrypt edit encrypt sync loop
  - `finitecomputer/finite-brain#19` Graph View and replay from decrypted client index
  - `finitecomputer/finite-brain#20` Product OKF import execution
  - `finitecomputer/finite-brain#21` Agent Vault Working Tree materialization
  - `finitecomputer/finite-brain#22` Portable v1 product hardening and runbook
- Issue sessions: `finite-brain#17`, `finite-brain#18`,
  `finite-brain#19`, `finite-brain#20`, `finite-brain#21`, and
  `finite-brain#22` completed.
- Agent briefs: pending.
- Review packets: pending.
- Runbook: `docs/runbooks/product-client-parity-local-staging.md`
- Local CodeRabbit report:
  `docs/feature-dev/2026-06-24-local-coderabbit-product-parity-round-2.md`
- PR CodeRabbit/fallback report:
  `docs/feature-dev/2026-06-24-pr-coderabbit-product-parity-round-2.md`
- PR URL: existing staging PR is `https://github.com/finitecomputer/finite-brain/pull/15`.

## Commands

- Install: `cargo fetch`
- Typecheck: `cargo check --all-targets`
- Test: `cargo test`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Format: `cargo fmt --check`
- Build: `cargo build`
- Local app: `cargo run -p finite-brain-app`

## Scope

In scope:

- Product Client: a real trusted browser client, not the development Smoke UI.
- Product Client Spine: connect NIP-07 signer, load a Vault, open Folder Key
  Grants, decrypt/read accessible Pages, edit one Page, encrypt/write a signed
  revision, and pull/apply sync without discarding unresolved local edits.
- NIP-07 workflow: signer discovery, auth signing, NIP-44 encrypt/decrypt,
  Folder Key Grant opening, and session keyring behavior.
- Graph View and replay: local decrypted graph/search indexes plus an
  Obsidian-like graph and replay-capable projection.
- OKF import execution: client-side conflict planning, encryption, and upload.
- Agent working tree materialization: accessible decrypted content on disk with
  agent discovery rules and encrypted-sync boundaries.
- Production hardening: replay cache, rate limits, CORS allowlist, and
  deploy/runbook readiness that can be proven without production changes.

Out of scope:

- Backwards compatibility shims.
- Legacy route compatibility.
- Turnkey migration from old v1 storage/client runtime.
- Production deployment, production migrations, production config changes, or
  live data operations.

## Slice Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| `finite-brain#17` | AFK | complete | Direct two-axis review; sub-agent review skipped because sub-agent tool policy requires explicit user delegation | None | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo fmt --check`; `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local `/client`, `/client/config.json`, `/client/app.js`, and `/client/app.css` curl smoke |
| `finite-brain#18` | AFK | complete | Direct two-axis review; sub-agent review skipped because sub-agent tool policy requires explicit user delegation | Pinned prepared Page writes to their original Folder/Object target before commit | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo fmt --check`; `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local `/client`, `/client/config.json`, and `/client/app.js` curl smoke |
| `finite-brain#19` | AFK | complete | Direct two-axis review; sub-agent review skipped because sub-agent tool policy requires explicit user delegation | None | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo fmt --check`; `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local `/client`, `/client/app.js`, and `/client/app.css` curl smoke |
| `finite-brain#20` | AFK | complete | Direct two-axis review; sub-agent review skipped because sub-agent tool policy requires explicit user delegation | Tightened planner object-id allocation so skipped/overwritten entries do not consume generated ids | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo fmt --check`; `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local `/health`, `/client`, `/client/app.js`, and `/client/app.css` curl smoke |
| `finite-brain#21` | AFK | complete | Direct two-axis review; sub-agent review skipped because sub-agent tool policy requires explicit user delegation | Added object revision/key-version to the opened-page seam so working-tree intents carry real base revisions; clamped locked Folder marker reasons to safe metadata | `cargo fmt --check`; `cargo test -p finite-brain-core portability::tests::working_tree -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check` |
| `finite-brain#22` | AFK | complete | Direct two-axis review; local CodeRabbit completed with all findings addressed | Normalized default CORS origins for path-based public base URLs; CodeRabbit follow-up sanitized OKF omission reasons and bounded Product Client import object-id allocation | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo fmt --check`; `cargo test -p finite-brain-server protected_routes -- --nocapture`; `cargo test -p finite-brain-server cors_preflight_is_allowlist_driven -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local `/health`, `/client`, `/client/config.json`, and `/client/app.js` curl smoke |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None yet | | | | |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| `finite-brain#17` | `29b1486` | Orchestrator direct implementation | `cc2b1e5ec5af93681f1ee96a7e6841ec3f053426` | Standards/spec direct review passed; route/static asset diff follows existing Smoke UI route pattern, Product Client is distinct from Smoke UI, deterministic JS seams cover signer state, auth event template, and Folder locked-state projection | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo fmt --check`; `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local curl smoke |
| `finite-brain#18` | `14ceb56` | Orchestrator direct implementation | `26fd2540bf89d374bf02f17d5c7d465b1b801b44` | Standards/spec direct review passed after fixing prepared-write target drift; Product Client now has in-memory Folder Key opening, AES-GCM Folder Object encrypt/decrypt, signed revision request preparation, protected save path wiring, sync bootstrap merge, dirty draft conflict preservation, and duplicate event de-dupe seams | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo fmt --check`; `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local curl smoke |
| `finite-brain#19` | `e111755` | Orchestrator direct implementation | `e61bd85f4afee7352aea662ec71164f8280db3e3` | Standards/spec direct review passed; Product Client now builds graph nodes/edges from decrypted ready Pages only, omits locked content, extracts wiki/Markdown links, renders an SVG graph surface, and derives replay frames from ordered de-duplicated local Page changes | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo fmt --check`; `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local curl smoke |
| `finite-brain#20` | `ceb000f` | Orchestrator direct implementation | `6422105ccf7f8f0bd80d897bc519e9224b954dd9` | Standards/spec direct review passed; Product Client now parses readable OKF bundles, plans skip/copy/overwrite conflicts, rewrites imported relative links when copied targets move, rejects locked destination Folders without opened Folder Keys, prepares encrypted signed Folder Object revisions, and uploads through normal secure object routes | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo fmt --check`; `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local `/health`, `/client`, `/client/app.js`, and `/client/app.css` curl smoke |
| `finite-brain#21` | `7773ee6` | Orchestrator direct implementation | `9ae12cbd9a167b14faf9f2d7ce093bc70c0705a6` | Standards/spec direct review passed; core portability now projects accessible decrypted Pages into Vault Working Tree manifests/files, emits `AGENTS.md`, `_index.md`, `_wiki/`, `raw/`, `compiled/`, and `output/` conventions, represents locked Folders as safe metadata, preserves object revision/key version, and maps upsert/rename/delete file changes to Product Client encrypted object write/move/delete intents | `cargo fmt --check`; `cargo test -p finite-brain-core portability::tests::working_tree -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check` |
| `finite-brain#22` | `30d0a4b` | Orchestrator direct implementation | `87fc3dd11a7bc7849c0f4040a6799e7512521b60` | Standards/spec direct review passed; server now rejects replayed Nostr auth events, rate-limits protected routes, enforces allowlist-driven CORS, normalizes path-based public base URLs to origins, and documents Product Client local/staging verification plus hard-cut production handoff boundaries | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo fmt --check`; `cargo test -p finite-brain-server protected_routes -- --nocapture`; `cargo test -p finite-brain-server cors_preflight_is_allowlist_driven -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local curl smoke |
| Local CodeRabbit product parity | `87fc3dd` | Orchestrator direct cleanup | `455ad643d792c6c9e7cb0013d966e39025b90347` | All 3 local CodeRabbit findings addressed: app binary metadata, OKF omission reason sanitization, and bounded Product Client OKF import object-id allocation | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo fmt --check`; targeted OKF omission test; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local Product Client curl smoke |

## Resolved Decisions

- The Product Client is a first-party browser app served by the Rust app/server,
  not a SilverBullet compatibility surface. See
  `docs/adr/0004-build-a-first-party-product-client.md`.
- The minimum Product Client Spine must land before graph/replay, OKF import
  execution, and agent working-tree materialization build on top.
- Graph View and Graph Replay are derived from the Product Client's decrypted
  local Page index and applied sync history. The server remains sync/object
  metadata aware, not graph aware. See
  `docs/adr/0005-derive-graph-and-replay-from-client-decrypted-indexes.md`.
- OKF import execution is Product Client owned. The client parses readable OKF,
  plans conflicts, opens Folder Keys, encrypts imported Pages, signs revisions,
  and uploads through normal secure object routes. The server does not receive
  plaintext OKF imports. See
  `docs/adr/0006-keep-okf-import-execution-client-owned.md`.

## Open Questions

- None for the current executable slice.

## Escalations

- None.
