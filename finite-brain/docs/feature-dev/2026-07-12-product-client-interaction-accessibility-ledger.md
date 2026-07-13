# FiniteBrain Product Client Interaction and Accessibility Ledger

## Run

- Run ID: 2026-07-12-product-client-interaction-accessibility
- Loop: Feature Dev continuation
- Target repo: finitecomputer/finite-mono
- Base branch: `main` (explicitly chosen for the existing Product Client PR)
- Feature branch: `feature/finitebrain-settings-vault-ui`
- Human owner: Austin
- Started: 2026-07-12
- Current status: all AFK tickets and post-review fixes are committed; local
  verification is complete with recorded environment limits and the existing
  PR refresh is pending
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
- Ticket sessions: #25, #26, #27, #28, and post-review follow-up recorded
- Agent briefs: read-only affordance, keyboard/accessibility, and product-truth
  audits completed before specification
- Review packet:
  `2026-07-12-product-client-interaction-accessibility-final-review-packet.md`
  records scoped standards/spec and independent post-review audits; worthy P2
  findings were fixed
- Local CodeRabbit report:
  `2026-07-12-product-client-interaction-accessibility-local-coderabbit-round.md`
  (three silent free-allowance attempts; independent fallback review completed)
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
| #25 | AFK | complete | scoped review complete | truthful Page and Folder affordances | targeted client, asset, build, and browser checks |
| #26 | AFK | complete | scoped review + post-review P2 complete | canonical Vault navigation and legacy-control hard cut | targeted client, asset, and build checks |
| #27 | AFK | complete | scoped review + post-review P2 complete | clipboard and invitation handoff feedback | deterministic lifecycle/race, asset, and build checks |
| #28 | AFK | complete | scoped review + post-review P2 complete | keyboard and focus semantics | deterministic keyboard/focus, asset, and build checks |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None | | | | |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- |
| #25 | `c7ca3ef` | `/root/ticket_25_truthful_page_affordances` | `7bf2581`, `5d46c75` | accepted | targeted client, asset, build, browser |
| #26 | `2c6de13` | `/root/ticket_26_vault_legacy_cleanup` | `3a2f3c9`, `539ad30`, `8a58f70` | accepted; post-review follow-up | targeted client, asset, build |
| #27 | `8a58f70` | `/root/ticket_27_clipboard_invitation_feedback` | `398236d`, `44c669e` | accepted; post-review follow-up | deterministic client, asset, build |
| #28 | `44c669e` | `/root/ticket_28_keyboard_navigation` | `341df32` | accepted; post-review follow-up | deterministic client, asset, build |
| #26–#28 follow-up | `341df32` | `/root/post_review_interaction_fixes` | `8d41bc7` + final branch follow-up | independent P2 audit fixed | deterministic client, asset, build |

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

## Final Verification Notes

- Passed: Product Client deterministic seams, JavaScript syntax, focused and
  full `finite-brain-server` tests, FiniteBrain app build, Rust formatting,
  workspace clippy, skills/search static checks, and runtime-image contract.
- Passed: dashboard install, lint, unit test suite, and production build.
- The rebuilt local Product Client at `http://127.0.0.1:4039/client` serves
  the current Graph markup: no `graphFilterInput`, and only the real zoom,
  reset-zoom, and full-screen controls.
- Two `cargo test --workspace --locked` runs failed only in unrelated,
  parallel `finite-saas-runner` Kata fake-process tests; the failing tests
  passed individually. No FiniteBrain test failed.
- Visual browser automation could not run because neither `agent-browser` nor
  the Chrome/Chromium application bundle is installed on this machine. The
  dashboard browser suite has the same explicit missing-Chrome failure; no
  unpinned browser download was installed outside the Nix environment.

## Escalations

- None.
