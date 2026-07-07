# Rust v1 CI And Module-Depth Goal Ledger

## Run

- Run ID: 2026-06-24-rust-v1-ci-module-depth
- Loop: Feature Dev
- Target repo: `finitecomputer/finite-brain`
- Companion repo: `finitecomputer/finite-nostr`
- Base branch: `staging`
- Feature branch: `feature/rust-portable-v1-core`
- Human owner: Austin
- Started: 2026-06-24
- Current status: in progress
- Skill setup status: present in both repos (`AGENTS.md`, `docs/agents/domain.md`, `docs/agents/issue-tracker.md`, `docs/agents/triage-labels.md`)

## Goal

Add CI guardrails and deepen the Rust v1 module boundaries end to end:
FiniteBrain server routes, SQLite store subdomains, reusable finite-nostr
primitives, Portable v1 portability helpers, and one full lifecycle integration
test for sharing/sync/access behavior.

## Durable Artifacts

- CONTEXT updates: not needed; no new glossary terms or hard-to-reverse domain decisions.
- ADRs: not needed; this follows ADR 0001 through ADR 0006.
- PRD issue: finitecomputer/finite-brain#23
- Slice issues:
  - finitecomputer/finite-brain#24 Add CI guardrails for Rust Portable v1 repos
  - finitecomputer/finite-brain#25 Split finite-nostr reusable primitive modules
  - finitecomputer/finite-brain#26 Split finite-brain server route domains
  - finitecomputer/finite-brain#27 Split finite-brain store subdomains
  - finitecomputer/finite-brain#28 Split finite-brain portability helpers
  - finitecomputer/finite-brain#29 Add full lifecycle sharing and sync integration test
  - finitecomputer/finite-nostr#3 Split reusable Nostr primitive modules
- Issue sessions:
  - `docs/feature-dev/2026-06-24-issue-24-ci-guardrails-session.md`
  - `docs/feature-dev/2026-06-24-issue-25-finite-nostr-module-split-session.md`
  - `docs/feature-dev/2026-06-24-issue-26-server-route-split-session.md`
  - `docs/feature-dev/2026-06-24-issue-27-store-subdomain-split-session.md`
  - `docs/feature-dev/2026-06-24-issue-28-portability-helper-split-session.md`
  - `docs/feature-dev/2026-06-24-issue-29-lifecycle-integration-test-session.md`
- Agent briefs: current thread owns implementation directly; no worker handoff yet
- Review packets:
  - `docs/feature-dev/2026-06-24-issue-24-ci-guardrails-review-packet.md`
  - `docs/feature-dev/2026-06-24-issue-25-finite-nostr-module-split-review-packet.md`
  - `docs/feature-dev/2026-06-24-issue-26-server-route-split-review-packet.md`
  - `docs/feature-dev/2026-06-24-issue-27-store-subdomain-split-review-packet.md`
  - `docs/feature-dev/2026-06-24-issue-28-portability-helper-split-review-packet.md`
  - `docs/feature-dev/2026-06-24-issue-29-lifecycle-integration-test-review-packet.md`
- Local CodeRabbit report: `docs/feature-dev/2026-06-24-local-coderabbit-module-depth-round.md`
- PR URL: finitecomputer/finite-brain#15; finitecomputer/finite-nostr PR pending if companion repo changes need a separate PR

## Commands

- Install: `cargo fetch`
- Typecheck: `cargo check --workspace`
- Test: `cargo test`
- Build: `cargo build`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Format: `cargo fmt --check`
- JavaScript smoke: `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`
- Visual verification: not expected unless Product Client UI changes

## Slice Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| finitecomputer/finite-brain#24 | AFK | complete | current thread | private dependency CI fetch fixed with read-only deploy key | local commands pass; remote CI pending rerun |
| finitecomputer/finite-brain#25 / finitecomputer/finite-nostr#3 | AFK | complete | current thread | none | finite-nostr fmt/test/clippy/build pass; finite-brain tests/clippy pass against pinned split commit |
| finitecomputer/finite-brain#26 | AFK | complete | current thread | none | local commands pass; remote CI pending push |
| finitecomputer/finite-brain#27 | AFK | complete | current thread | none | local commands pass; remote CI pending push |
| finitecomputer/finite-brain#28 | AFK | complete | current thread | none | local commands pass; remote CI pending push |
| finitecomputer/finite-brain#29 | AFK | complete | current thread | none | local commands pass; remote CI pending push |

## Parked HITL Slices

None.

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| finitecomputer/finite-brain#24 | finite-brain `7283b6c0affe7f718b26b8d93cdbd0de2dda31ce`; finite-nostr `621bb347f9734f2dcb891333ed8e7c2862ca73e1` | current thread | finite-brain this commit (`Add Rust CI guardrails`), plus follow-up CI auth commit; finite-nostr `baaa13a05f3691cf207f78f640f99c8bbd76cb0b` | pass | `cargo fmt --all --check`; `cargo test --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo build --workspace`; JS syntax/smoke; finite-nostr fmt/test/clippy/build |
| finitecomputer/finite-brain#25 / finitecomputer/finite-nostr#3 | finite-brain `13a4f1230add97969246a585f345e2e4a1c61716`; finite-nostr `baaa13a05f3691cf207f78f640f99c8bbd76cb0b` | current thread | finite-brain this commit (`Pin split finite-nostr primitives`); finite-nostr `0ecf25abc3198f357a7b922865829b37a7fe5d13` | pass | finite-nostr fmt/test/clippy/build; finite-brain `cargo update -p finite-nostr`; finite-brain test/clippy |
| finitecomputer/finite-brain#26 | finite-brain `01bba95` | current thread | this commit (`Split Rust modules and add lifecycle test`) | pass | `cargo test -p finite-brain-server --no-run`; `cargo test --workspace`; clippy; build |
| finitecomputer/finite-brain#27 | finite-brain `01bba95` | current thread | this commit (`Split Rust modules and add lifecycle test`) | pass | `cargo test -p finite-brain-store`; `cargo test --workspace`; clippy; build |
| finitecomputer/finite-brain#28 | finite-brain `01bba95` | current thread | this commit (`Split Rust modules and add lifecycle test`) | pass | `cargo test -p finite-brain-core`; `cargo test --workspace`; clippy; build |
| finitecomputer/finite-brain#29 | finite-brain `01bba95` | current thread | this commit (`Split Rust modules and add lifecycle test`) | pass | lifecycle test; `cargo test --workspace`; clippy; build |

## Open Questions

- None.

## Escalations

- None.
