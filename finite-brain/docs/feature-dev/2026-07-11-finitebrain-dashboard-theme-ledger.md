# FiniteBrain Dashboard Theme Goal Ledger

## Run

- Run ID: `2026-07-11-finitebrain-dashboard-theme`
- Loop: Plebdev Feature Dev v0.4.0
- Target repo: `finitecomputer/finite-mono`, scoped to `finite-brain`
- Base branch: `main` (human-confirmed exception because this monorepo has no `staging` branch)
- Feature branch: `feature/finitebrain-dashboard-theme`
- Human owner: plebdev
- Started: 2026-07-11
- Current status: tracer-bullet ticket planning
- Skill setup status: present under `finite-brain/docs/agents/`; GitHub issue tracker, canonical triage labels, and single-context domain docs are configured

## Goal

Without changing any critical layout or functionality of the FiniteBrain Product
Client prototype frontend, reskin and retheme it end to end so it fits the
Finite dashboard's theme, color system, typography, surfaces, controls, and
overall visual character in a high-quality, elegant, surgical way.

## Durable Artifacts

- CONTEXT updates: added `Dashboard-Aligned Product Theme`
- ADRs: none warranted yet; this is a reversible presentation-layer change
- Prototype source branch, if any: none planned unless visual evidence exposes an unresolved design choice
- Spec issue: [finitecomputer/finite-mono#4](https://github.com/finitecomputer/finite-mono/issues/4)
- Tickets: pending
- Ticket sessions: pending
- Agent briefs: pending
- Review packets: pending
- Local CodeRabbit report: pending
- PR URL: pending

## Commands

- Install: repository dependencies are Nix-managed; no system install. Use `scripts/with-dev-env` for direct commands.
- Typecheck: not applicable to the dependency-free Product Client; use `node --check finite-brain/crates/finite-brain-server/src/product-client.js`
- Test: `node finite-brain/crates/finite-brain-server/src/product-client.test.js`; `scripts/with-dev-env cargo test -p finite-brain-server`
- Build: `scripts/with-dev-env cargo build -p finite-brain-app`
- Visual verification: seed the smoke fixture, run `finite-brain/scripts/verify-obsidian-product-client.mjs`, serve `finite-brain-app`, and inspect `/client` at desktop and mobile widths with screenshots

## Ticket Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| Pending | AFK | awaiting spec and approval | — | — | — |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None | — | — | — | — |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| Pending | Pending | Pending | Pending | Pending | Pending |

## Open Questions

- Approve tracer-bullet granularity and blocking edges before ticket publication.
- Approve tracer-bullet granularity and blocking edges before ticket publication.

## Escalations

- The human approved `main` as the base and PR target because `staging` is absent.
- The human approved system-driven light and dark Product Client themes matching the dashboard.
- The human approved self-hosting the dashboard's exact Funnel Sans, Funnel Display, and JetBrains Mono assets from the Product Client origin.
- The human approved a native Finite dashboard visual identity; the Product Client retains its current Obsidian-like interaction structure and information density, not its purple/charcoal styling.
- The human confirmed shared understanding of the full reskin scope and preservation constraints.
- The human approved the end-to-end browser, Product Client contract, Rust asset-route, and full regression testing seams.
