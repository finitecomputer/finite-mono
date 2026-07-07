# Goal Ledger: Rust Portable v1 Core

## Run

- Run ID: `2026-06-23-rust-portable-v1-core`
- Loop: Feature Dev
- Target repo: `finitecomputer/finite-brain`
- Companion repo: `finitecomputer/finite-nostr`
- Base branch: `staging`
- Feature branch: `feature/rust-portable-v1-core`
- Human owner: Austin
- Started: 2026-06-23
- Current status: Feature Dev complete; non-draft staging PR open, reviewed, pushed, and mergeable
- Skill setup status: `AGENTS.md` and `docs/agents/*` created for both repos

## Goal

Implement the FiniteBrain Portable v1 specification end to end in Rust, focused
on core logic, spec correctness, validation, tests, security hardening, and
production-shaped server behavior. Keep reusable Nostr primitive logic in the
new `finite-nostr` Rust crate so other Finite repos can reuse it.

## Durable Artifacts

- CONTEXT updates: root `CONTEXT.md`, `AGENTS.md`, and `docs/agents/*`
  created for the Rust repo; companion `finite-nostr` has matching root
  `CONTEXT.md`, `AGENTS.md`, and `docs/agents/*`
- ADRs: `docs/adr/0001-adopt-rust-workspace-and-finite-nostr.md`,
  `docs/adr/0002-use-sqlite-from-day-one.md`,
  `docs/adr/0003-keep-folder-object-crypto-in-finite-brain-core.md`
- PRD issue: `finitecomputer/finite-brain#1`
- Slice issues:
  - `finitecomputer/finite-nostr#1` reusable Nostr identity, event, and HTTP auth primitives
  - `finitecomputer/finite-nostr#2` reusable NIP-44 and NIP-59 wrapping primitives
  - `finitecomputer/finite-brain#2` Rust workspace and health smoke path
  - `finitecomputer/finite-brain#3` core domain model, path rules, and Vault bootstrap
  - `finitecomputer/finite-brain#4` Folder Object encryption, canonical hashes, and signed record validation
  - `finitecomputer/finite-brain#5` SQLite store for Vaults, Folders, access, and grants
  - `finitecomputer/finite-brain#6` sync append log, current projection, and conflict handling
  - `finitecomputer/finite-brain#7` Nostr-authenticated server shell and Vault metadata APIs
  - `finitecomputer/finite-brain#8` secure object routes and sync APIs
  - `finitecomputer/finite-brain#9` Folder Access, grant, Finish Setup, and rotation flows
  - `finitecomputer/finite-brain#10` singleton Vault Invitations and Share Links
  - `finitecomputer/finite-brain#11` Shared Folder Connections and mounted Folder projection
  - `finitecomputer/finite-brain#12` Encrypted Export, OKF Import/Export, and LLM Wiki privacy rules
  - `finitecomputer/finite-brain#13` development-only Smoke UI
  - `finitecomputer/finite-brain#14` Portable v1 hardening, compatibility, and end-to-end readiness
- Issue sessions: tracked in the Issue Session Ledger below
- Agent briefs: generated slice issues plus this ledger
- Review packets: direct orchestrator review results captured in the Issue Session Ledger
- Local CodeRabbit report: `docs/feature-dev/2026-06-24-local-coderabbit-round-1.md`
- PR CodeRabbit fallback report:
  `docs/feature-dev/2026-06-24-pr-coderabbit-fallback-round-1.md`
- PR URL: `https://github.com/finitecomputer/finite-brain/pull/15`

## Commands

- Install: `cargo fetch`
- Typecheck: `cargo check --all-targets`
- Test: `cargo test`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Format: `cargo fmt --check`
- Build: `cargo build`
- Visual verification: `FINITE_BRAIN_ADDR=127.0.0.1:4015 cargo run -p finite-brain-app`, then `curl http://127.0.0.1:4015/health`

## Slice Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| `finite-nostr#1` | AFK | complete | Direct review gate in orchestrator | None | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings` |
| `finite-nostr#2` | AFK | complete | Direct review gate in orchestrator | None | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#2` | AFK | complete | Direct review gate in orchestrator | Fixed README workspace docs and ledger command evidence | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; local `/health` curl |
| `finite-brain#3` | AFK | complete | Direct review gate in orchestrator | None | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `curl /smoke/bootstrap` |
| `finite-brain#4` | AFK | complete | Direct review gate in orchestrator | Added random-nonce public encrypt helper and deterministic vector helper before commit | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#5` | AFK | complete | Direct review gate in orchestrator | Tightened org-only member/admin mutation and grant issuer authorization before commit | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#6` | AFK | complete | Direct review gate in orchestrator | Folded projection upsert parameters into a typed helper after clippy review | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#7` | AFK | complete | Direct review gate in orchestrator | Added SQLite-backed app router and NIP-98 auth route coverage | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#8` | AFK | complete | Direct review gate in orchestrator | Bumped `finite-nostr` HTTP auth parsing to support authenticated `DELETE` and added route coverage | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#9` | AFK | complete | Direct review gate in orchestrator | Added kind/key-version hardening before commit | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#10` | AFK | complete | Direct review gate in orchestrator | Fixed generated link timestamps before commit | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#11` | AFK | complete | In-thread two-axis review | Fixed shared Folder rotation/member-state atomicity and added idempotent accept assertions before commit | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#12` | AFK | complete | In-thread two-axis review | Kept readable OKF/import/search as client-side core logic over opened plaintext; server exposes encrypted export only and rejects plaintext search | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#13` | AFK | complete | In-thread two-axis review | Expanded the first UI draft to cover invitation, Share Link, shared Folder invitation, connection, and mount route families before commit | `cargo fmt --check`; `cargo test -p finite-brain-server smoke_ui_serves_static_assets_and_sqlite_flow_works -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#14` | AFK | complete | In-thread two-axis review | Tightened default auth skew, request body limits, auth header aliases, sync pull cap tests, SQLite backup/restore checks, and added `docs/readiness/portable-v1-hardening.md` | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local `/health`, `/smoke/bootstrap`, `/smoke/ui`, and `/smoke/ui.js` curl smoke |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None | | | | |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| PR CodeRabbit fallback round 1 | `49b012e` | `Meitner` `019ef729-99cf-7312-8d11-e21f4d600f9e` | `ce212e7` | CodeRabbit stayed silent after PR trigger, so fallback review was used. All 6 fallback findings addressed: NIP-59 grant wrapper validation, RFC3339 signed timestamps, admin/control-record visibility, bootstrap control records, signed event preservation, and diff hygiene. | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check` |
| Local CodeRabbit round 1 | `8baa2cb` | Orchestrator direct cleanup | `f28a55e` | All 11 local findings addressed: bounded identifiers/paths, RFC3339 link timestamps, runtime store timestamps, fallible JSON serialization, bootstrap caps, OKF duplicate bundle paths, route sync limit clamp, dynamic auth clock, grant fallback timestamps, locked object write checks, and generic database errors | `cargo fmt --check`; targeted core/store/server tests; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local `/health`, `/smoke/bootstrap`, `/smoke/ui`, and `/smoke/ui.js` curl smoke |
| `finite-brain#14` | `64ae9e9a39d78db1f7e523982e9bd04e925109cf` | Orchestrator direct implementation | `9bf2b84` | Standards/spec review passed; hardening added explicit Portable v1 limits, compatibility auth header aliases, SQLite restore/rebuild evidence, dependency/security/error/smoke matrix documentation, and runtime route smoke evidence | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; local `/health`, `/smoke/bootstrap`, `/smoke/ui`, and `/smoke/ui.js` curl smoke |
| `finite-brain#13` | `2d9026f2bca5b27e3cf4e1448749749ec0a511d0` | Orchestrator direct implementation | `ff5cb09` | Standards/spec review passed after adding dev-only routes, static HTML/CSS/JS, route controls for bootstrap, metadata, folders, objects, sync, invites, Share Links, shared Folder invitations, connections, mounts, export, and SQLite-backed route coverage | `cargo fmt --check`; `cargo test -p finite-brain-server smoke_ui_serves_static_assets_and_sqlite_flow_works -- --nocapture`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#12` | `ad343867783b1ff622e347781b0eda5569110e4a` | Orchestrator direct implementation | `9740c1b` | Standards/spec review passed; encrypted export route/store filtering landed alongside client-side OKF bundle generation, import conflict planning, LLM Wiki conventions, and local-search privacy boundary | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#11` | `735e4380db485d52936c7cffeedc57b9115afd44` | Orchestrator direct implementation | `75685e4` | Standards/spec review passed after folding shared Folder access rotation plus member/status mutation into one transaction and adding explicit idempotent accept coverage | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#10` | `6b4965ccb56d763bec64ffd2a4a62ee07052daea` | Orchestrator direct implementation | `81dfcd3` | Standards/spec review passed after replacing fixed link timestamp generation with server-clock RFC3339 timestamps | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#9` | `0bd2b718c1452cfefabaa12a3e3b356f06af5b46` | Orchestrator direct implementation | `e7c24ea` | Standards/spec review passed after admin event kind and rotation key-version hardening | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#8` | `90877e65c07dfe860c322167a12e370c0600195c` | Orchestrator direct implementation | `e5cf5b1` | Standards/spec review passed; `finite-nostr#621bb34` consumed for generic HTTP auth method parsing | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#7` | `968895863bd266f27adea2b153003389c30eaf8e` | Orchestrator direct implementation | `704e10b` | Standards/spec review passed | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#6` | `dd692059cc942a6104117692c65cfe9d3aa3e749` | Orchestrator direct implementation | `97c18af` | Standards/spec review passed | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#5` | `bc4802a14942a71ca7127ee9abf35547bb95ad06` | Orchestrator direct implementation | `ac76671` | Standards/spec review passed | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#4` | `48460ee442eac4b5d58c7ab6196e8e3ecbc5d0a5` | Orchestrator direct implementation | `ecc34fe` | Standards/spec review passed | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-brain#3` | `041377794c23ab338bd1dee47b4e209bc2c2ef83` | Orchestrator direct implementation | `c43308d` | Standards/spec review passed | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `curl /health`; `curl /smoke/bootstrap` |
| `finite-nostr#2` | `f5c38f36f0377504d695d5509231fde332fa13d2` | Orchestrator direct implementation | `06cd71d` | Standards/spec review passed | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `git diff --check` |
| `finite-nostr#1` | `c92aaec05eef9f181cf62855743564a13dd4bfd0` | Orchestrator direct implementation | `f5c38f3` | Standards/spec review passed | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings` |
| `finite-brain#2` | `9148111454140fa22568cc035b5ea71db6ad1cfd` | Orchestrator direct implementation | `16ba2e4` | Standards/spec review passed after README and ledger fixes | `cargo fmt --check`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `curl /health` |

## Open Questions

- None.

## Escalations

- None.
