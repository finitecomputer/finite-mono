# FiniteBrain Product Client Interaction and Accessibility Ledger

## Run

- Run ID: 2026-07-12-product-client-interaction-accessibility
- Loop: Feature Dev continuation
- Target repo: finitecomputer/finite-mono
- Base branch: `main` (explicitly chosen for the existing Product Client PR)
- Feature branch: `feature/finitebrain-settings-vault-ui`
- Human owner: Austin
- Started: 2026-07-12
- Current status: spec and dependency-ordered AFK tickets published; implementation pending
- Skill setup status: present (`finite-brain/AGENTS.md` and
  `finite-brain/docs/agents/`)

## Goal

Resolve the reported misleading/redundant Product Client controls and complete
the keyboard and accessibility paths end to end. Preserve existing FiniteBrain
terminology, Session Lock, encrypted-content, and access-control invariants;
ask for direction only if the current product truth leaves a behavior genuinely
undefined.

## Durable Artifacts

- CONTEXT updates: none; existing Product Client, Graph View, Source Note,
  Session Lock, and Ephemeral Client Plaintext terms resolved the work
- ADRs: none; the accepted first-party client, graph, client-owned OKF,
  session-key, plaintext, and hard-cut ADRs resolve the decisions
- Prototype source branch, if any: none
- Spec issue: #24 — https://github.com/finitecomputer/finite-mono/issues/24
- Tickets: #25, #26, #27, #28
- Ticket sessions: pending
- Agent briefs: read-only affordance, keyboard/accessibility, and product-truth
  audits completed before specification
- Review packets: pending
- Local CodeRabbit report: pending
- PR URL: https://github.com/finitecomputer/finite-mono/pull/16

## Commands

- Install: Nix/direnv-provided development environment
- Typecheck: `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
- Test: `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
- Build: `scripts/with-dev-env cargo build -p finite-brain-app --locked`
- Visual verification: isolated local Rust-served `/client` with disposable
  Vault and Member Identities through headless Chromium/CDP

## Ticket Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| #25 | AFK | ready | pending | truthful Page and Folder affordances | pending |
| #26 | AFK | ready | pending | canonical Vault navigation and legacy-control hard cut | pending |
| #27 | AFK | ready | pending | clipboard and invitation handoff feedback | pending |
| #28 | AFK | ready; blocked by #25 and #26 | pending | keyboard and focus semantics | pending |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None | | | | |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- |
| Pending | | | | | |

## Open Questions

- None. The source audits and prior Settings/Vault spec resolve the user-facing
  choices without an additional product decision:
  - Graph nodes stay hover-only and non-clickable.
  - Folder deletion remains absent until a separate server contract exists.
  - Task ticks update the normal Page draft and persist through an explicit
    signed Save, rather than starting background writes.
  - The raw reader source mode is removed; the retained editor is named
    `Edit Markdown` to avoid overloading the accepted Source Note term.
  - The footer remains the compact switcher; Manage Vaults retains the detailed
    list and explicit Load/Unlock; Settings loses its obsolete embedded picker.
  - Invitation Enter is non-consuming; Accept and Revoke remain explicit.

## Ticket Plan

- #25: truthful Page and Folder affordances — no blockers.
- #26: canonical Vault navigation and legacy-control hard cut — no blockers.
- #27: clipboard and invitation handoff feedback — no blockers.
- #28: keyboard semantics — blocked by #25 and #26 so keyboard behavior lands
  against the final menu/editor/navigation surfaces.

Testing seams are the same agent-verifiable seams used for the immediately
preceding remediation: deterministic Product Client contracts plus an isolated
Rust-served `/client` browser flow with disposable local identities.

## Escalations

- None.
