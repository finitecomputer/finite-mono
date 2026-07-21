# Email Invite Frontend Smoke Ledger

## Run

- Run ID: `2026-07-07-email-invite-frontend-smoke`
- Loop: Feature Dev
- Target repo: `finitecomputer/finite-brain`
- Base branch: `staging` requested by loop; repository currently exposes `main` only locally/remotely.
- Feature branch: `feature/email-invite-frontend-smoke`
- Human owner: plebdev
- Started: 2026-07-07
- Current status: PR open against `main`; local review, CodeRabbit, browser smoke, and CI passed; PR CodeRabbit fallback recorded
- Skill setup status: present (`AGENTS.md`, `docs/agents/issue-tracker.md`, `docs/agents/triage-labels.md`, `docs/agents/domain.md`)

## Goal

Integrate the email invite flow into the Product Client frontend end to end, then smoke test and verify it completely.

## Durable Artifacts

- CONTEXT updates: existing email invite terms in `CONTEXT.md`
- ADRs: `docs/adr/0009-use-email-invite-bootstraps-for-email-targeted-brain-invitations.md`
- PRD issue: finitecomputer/finite-brain#77
- Slice issues: finitecomputer/finite-brain#84, finitecomputer/finite-brain#85
- Issue sessions: existing local commits through current `main`
- Agent briefs: GitHub issues #84/#85
- Review packets: `docs/feature-dev/2026-07-07-email-invite-frontend-smoke-review-packet.md`
- Local CodeRabbit report: `docs/feature-dev/2026-07-07-local-coderabbit-email-invite-frontend-smoke.md`
- PR CodeRabbit report: `docs/feature-dev/2026-07-07-pr-coderabbit-email-invite-frontend-smoke.md`
- PR URL: https://github.com/finitecomputer/finite-brain/pull/86

## Commands

- Install: not needed; Rust/Node toolchains already available locally
- Typecheck: `node --check crates/finite-brain-server/src/product-client.js`
- Test: `node crates/finite-brain-server/src/product-client.test.js`; `cargo test`
- Build: `cargo build`
- Visual verification: browser smoke against local Product Client with explicit smoke NIP-07 signer and smoke email-proof allowlist

## Implementation Notes

- Added explicit app/server smoke email proof allowlist via `FINITE_BRAIN_SMOKE_EMAIL_PROOFS`, guarded so it cannot be combined with `FINITE_IDENTITY_AUTHORITY` and requires `FINITE_BRAIN_SMOKE_NIP07_SECRET`.
- Extended the opt-in smoke NIP-07 script so browser smoke can use a second local signer through a fragment-only `smokeNip07Secret` override.
- Changed Product Client email invite URLs from API claim paths to `/client#inviteCode=...&inviteEmail=...&inviteSecret=...`, keeping invite data client-side in the fragment.
- Product Client invite landing now hydrates invite code, invited email, and Invite Secret from the fragment, opens the Access invite panel, shows a local connect-signer action, and keeps the panel visible before org metadata is loaded.
- Fixed email proof timestamp handling for browser-created proof timestamps and allowed a small server-side future clock-skew tolerance while preserving the one-day freshness cap and not-before-invitation rule.
- Fixed post-accept/post-claim metadata loading so the newly accepted or claimed Brain remains active instead of being overwritten by stale selector state.

## Slice Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| #84 | AFK | verified locally | current thread | none known | Product Client deterministic tests, static verifier, browser smoke |
| #85 | AFK | verified locally | current thread | none known | Rust E2E encrypted collaboration test, full browser email invite claim smoke |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| Branch target | `staging` is required by Feature Dev loop but absent in this repo's visible branches | final PR target | resolved by targeting the repository default branch, `main` | PR #86 targets `main` |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| #84/#85 | prior local email invite commits | current thread | `4c8fdd3`, `78631b5`, `6a54cd4`, `392509a0795e24de01fb575fbe7959e44ba3a452` | pass: direct standards/spec review plus local CodeRabbit findings `0` | `node --check crates/finite-brain-server/src/product-client.js`; `node crates/finite-brain-server/src/product-client.test.js`; `node scripts/verify-obsidian-product-client.mjs`; `cargo test`; `cargo clippy --all-targets -- -D warnings`; `cargo build`; `git diff --check`; live browser smoke |

## Browser Smoke Evidence

- Local app command: `FINITE_BRAIN_ADDR=127.0.0.1:4029 FINITE_BRAIN_PUBLIC_BASE_URL=http://127.0.0.1:4029 FINITE_BRAIN_DB=/tmp/finite-brain-email-invite-ui-smoke-20260707h.sqlite3 FINITE_BRAIN_SMOKE_NIP07_SECRET=... FINITE_BRAIN_SMOKE_EMAIL_PROOFS=friend@example.com cargo run -p finite-brain-app`
- Admin browser flow created Organization Brain `org-email-invite-smoke-ship-20260707-mraz0n9z`.
- Admin browser flow created email invite for `friend@example.com`; generated invite URL path was `http://127.0.0.1:4029/client` and the fragment contained `inviteCode`, `inviteEmail`, and `inviteSecret`.
- Recipient browser flow opened that URL with a second smoke signer, landed on the invite panel with code/email/secret hydrated, connected signer from the panel, and claimed the invite.
- Claim result: `Email invite claimed`; active Brain became `org-email-invite-smoke-ship-20260707-mraz0n9z`; opened keys count was `4`; Files view showed `getting-started` and `restricted`.

## Open Questions

- None.

## Escalations

- None yet.

## Review Evidence

- Review packet: `docs/feature-dev/2026-07-07-email-invite-frontend-smoke-review-packet.md`
- CodeRabbit local gate: `coderabbit review --agent --type uncommitted` completed with `findings: 0`.
- PR CodeRabbit trigger/fallback: `docs/feature-dev/2026-07-07-pr-coderabbit-email-invite-frontend-smoke.md`
- Sub-agent note: `code-review` normally uses parallel sub-agents, but the current sub-agent tool policy requires explicit user permission for delegation; the standards/spec review was performed directly in this thread and paired with CodeRabbit.

## Final Local Verification

- `node --check crates/finite-brain-server/src/product-client.js`: pass
- `node crates/finite-brain-server/src/product-client.test.js`: pass (`product-client deterministic seams ok`)
- `node scripts/verify-obsidian-product-client.mjs`: pass (`obsidian product client smoke ok`)
- `cargo fmt --check`: pass
- `git diff --check`: pass
- `cargo test`: pass (workspace unit and doc tests)
- `cargo clippy --all-targets -- -D warnings`: pass
- `cargo build`: pass
