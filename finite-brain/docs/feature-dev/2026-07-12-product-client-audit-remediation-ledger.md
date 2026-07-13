# FiniteBrain Product Client Audit Remediation Ledger

## Run

- Run ID: 2026-07-12-product-client-audit-remediation
- Loop: Feature Dev continuation
- Target repo: finitecomputer/finite-mono
- Base branch: `main` (`origin/main`)
- Feature branch: `feature/finitebrain-settings-vault-ui`
- Human owner: Austin
- Started: 2026-07-12
- Current status: all remediation tickets implemented; final browser acceptance and independent review passed; bounded CodeRabbit follow-up timed out without findings
- Skill setup status: present (`finite-brain/AGENTS.md` and `finite-brain/docs/agents/`)

## Goal

Deal with every audit must-fix end to end, using the existing FiniteBrain
terminology, ADRs, and product truths; pause only if behavior is genuinely
undefined or ambiguous.

## Durable Artifacts

- CONTEXT updates: none; established terms govern this continuation
- ADRs: none planned; no new hard-to-reverse decision is needed
- Prototype source branch, if any: none
- Spec issue: #17 — https://github.com/finitecomputer/finite-mono/issues/17
- Tickets: #18, #19, #20, #21, #22, #23
- Ticket sessions: `finite-brain/docs/feature-dev/2026-07-12-product-client-audit-remediation-ticket-{18,19,20,21,22,23}-session.md`
- Agent briefs: audit and alignment evidence are recorded in this ledger/spec
- Review packets: `finite-brain/docs/feature-dev/2026-07-12-product-client-audit-remediation-ticket-{18,19,20,21,22,23}-review.md`
- Local CodeRabbit reports: `finite-brain/docs/feature-dev/2026-07-12-product-client-audit-remediation-local-coderabbit-round.md` (completed, findings fixed) and `finite-brain/docs/feature-dev/2026-07-12-product-client-audit-remediation-local-coderabbit-final-round.md` (bounded follow-up timed out without findings)
- PR URL: https://github.com/finitecomputer/finite-mono/pull/16

## Commands

- Typecheck: `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
- Test: `scripts/with-dev-env node --test finite-brain/crates/finite-brain-server/src/product-client.test.js`; focused Rust Product Client/server tests
- Build: `scripts/with-dev-env cargo build -p finite-brain-server`
- Visual verification: isolated Rust-served `/client` with disposable local Vault and Member Identities

## Ticket Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| #18 | AFK | complete | standards pass; spec pass | signed Page save/delete | Node contract; browser signed PUT/tombstone DELETE; syntax; diff |
| #19 | AFK | complete | standards/spec pass after three P2 corrections | safe visible client feedback | Node contract; browser generic failure then Session Lock purge; syntax; diff |
| #20 | AFK | complete | focused self-review pass + final integration proof | authorization-loss Session Lock | Node contract; real two-identity browser accept/remove/metadata-403 lock + immediate re-unlock; syntax; diff |
| #21 | AFK | complete | independent focused review pass + final integration correction | invitation Session Lock/reactivity/revoke | browser locked controls, recipient accept lock/render, admin create/revoke; Node contract; syntax; diff |
| #22 | AFK | complete | standards/spec review pass | Child Folder hierarchy metadata | browser parent-aware Folder POST/grants/default access; Node contract; syntax; diff |
| #23 | AFK | complete | standards/spec review pass | hidden Graph filter removal | browser desktop/narrow visual/DOM graph-control proof; Node contract; fixture verifier; Rust tests; syntax; diff |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None | | | | |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- |
| #18 | `3c828e0` | `/root/ticket_18_page_persistence` | `7fc85c4` | standards pass; spec pass | Node contract; syntax; diff |
| #19 | `7fc85c4` | `/root/ticket_19_client_feedback` | `cae93df` | standards/spec pass after corrections | Node contract; syntax; diff |
| #20 | `65b98a7` | `/root/ticket_20_access_loss` | `390c801` | focused self-review pass + real final flow | Node contract; real recipient revocation `403`/lock; syntax; diff |
| #21 | `fe577fc` | `/root/ticket_21_invitations` | `dfc8a0b` + `4909693` | independent focused review pass | Node contract; visible accept lock/render + admin revoke; syntax; diff |
| #22 | `ca86dbe` | `/root/ticket_22_child_folders` | `f4d66f8` | standards/spec pass | Node contract; syntax; diff |
| #23 | `b946ed6` | `/root/ticket_23_graph_filter` | `72b5d82` | standards/spec pass after legacy-test cleanup | Node contract; fixture verifier; server tests; desktop/narrow browser proof; syntax; diff |

## Open Questions

- Child Folder access intentionally remains independent. This continuation
  preserves current creation defaults rather than inventing restricted-access
  inheritance.

## Escalations

- None. Existing Product Client, Session Lock, Folder, invitation, and NIP-98
  replay product truths resolve the remediation behavior. Final browser
  verification exposed a same-second replay collision in Product Client auth;
  the existing CLI nonce precedent resolved it without a new product decision.
