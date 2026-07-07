# Smoke Alpha Hardening Feature Dev Ledger

## Run

- Run ID: `2026-06-27-smoke-alpha-hardening`
- Loop: Feature Dev
- Target repo: `finitecomputer/finite-brain`
- Base branch: `main`
- Feature branch: `feature/smoke-alpha-hardening`
- Human owner: Paul
- Started: 2026-06-27
- Current status: verified locally, ready for hard-cut main merge
- Skill setup status: present (`AGENTS.md`, `docs/agents/issue-tracker.md`, `docs/agents/triage-labels.md`, `docs/agents/domain.md`)

## Goal

Make the first internal smoke alpha ready across the selected remaining gaps:
replace/archive the old SilverBullet smoke route, harden Product Client Folder
Key Grant opening, add a real agent sync daemon/file watcher path, productize
organization Vault invite/admin UX, and add a backup/restore runbook.

## Durable Artifacts

- CONTEXT updates: none yet
- ADRs: none yet
- PRD issue: `finitecomputer/finite-brain#47`
- Slice issues:
  - `finitecomputer/finite-brain#48` Harden Product Client Folder Key Grant wrapping and opening
  - `finitecomputer/finite-brain#51` Add fbrain daemon watch loop for Vault Working Trees
  - `finitecomputer/finite-brain#49` Productize organization Vault invitations in the Product Client
  - `finitecomputer/finite-brain#50` Add smoke alpha backup, restore, and SilverBullet cutover handoff
- Issue sessions: current thread
- Agent briefs: in-thread Feature Dev artifacts
- Review packets: `docs/feature-dev/2026-06-27-smoke-alpha-hardening-review-packet.md`
- Local CodeRabbit report: manual review fallback recorded in review packet
- PR URL: pending

## Commands

- Install: `cargo build --workspace`
- Typecheck: `cargo check --workspace`
- Test: `cargo test --workspace`
- Build: `cargo build --workspace`
- Product Client verification: `node crates/finite-brain-server/src/product-client.test.js`; `node scripts/verify-obsidian-product-client.mjs`
- Smoke verification: local `finite-brain` server plus `fbrain` vault create/open/sync/status flow

## Branch And Loop Notes

- Feature Dev normally targets `staging`, but this run intentionally uses
  `main` because the repo has just hard-cut `main` to the Rust Product Client
  plus `fbrain` stack, and `origin/staging` is an ancestor of `origin/main`.
- Live smoke route changes are deployment-loop work. This Feature Dev run will
  implement product/code/runbook readiness and produce a deployment handoff
  rather than directly modifying the live smoke box route.

## Slice Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| `#48` | AFK | verified | current thread | none known | focused JS test, seeded Product Client smoke, workspace tests |
| `#51` | AFK | verified | current thread | none known | CLI package tests, workspace tests |
| `#49` | AFK | verified | current thread | none known | focused JS, server asset test, seeded Product Client smoke |
| `#50` | AFK | verified | current thread | none known | backup verifier and store backup test |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| `#50` smoke route cutover | Live smoke routing and old SilverBullet archive/delete are deployment-loop operations. | Production/smoke promotion only | Approve Deployment loop promotion when PR is ready. | Include deployment handoff in PR |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| `#48` | `7473dc95aa0fd2ac292f7485401896828bd93735` | current thread | branch tip | passed after one minor invite-panel fix | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `cargo test --workspace` |
| `#51` | `7473dc95aa0fd2ac292f7485401896828bd93735` | current thread | branch tip | passed | `cargo test -p finite-brain-cli`; `cargo test --workspace`; `cargo clippy --all-targets -- -D warnings` |
| `#49` | `7473dc95aa0fd2ac292f7485401896828bd93735` | current thread | branch tip | passed after personal-vault create/revoke button disable fix | `node crates/finite-brain-server/src/product-client.test.js`; `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config -- --nocapture`; `node scripts/verify-obsidian-product-client.mjs` |
| `#50` | `7473dc95aa0fd2ac292f7485401896828bd93735` | current thread | branch tip | passed | `scripts/verify-smoke-alpha-backup-restore.sh`; `git diff --check` |

## Open Questions

- None. Working assumption: hard-cut means no legacy SilverBullet compatibility
  shims; the old live route is replaced during deployment, not preserved in the
  product.

## Escalations

- None.
