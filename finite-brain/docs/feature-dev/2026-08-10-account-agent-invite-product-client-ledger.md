# FiniteBrain Account-Agent Invite Product Client Ledger

## Run

- Run ID: `2026-08-10-account-agent-invite-product-client`
- Loop: Feature Dev
- Target repo: `finitecomputer/finite-mono`
- Base branch: `origin/main` at `0903b4267efbd53c8eafad65ea42958f860ebc0c`
- Feature branch: `codex/finite-brain-invite-prototype-parity`
- Human owner: Austin
- Started: 2026-08-10
- Current status: implementation verified and reviewed; publication in progress
- Skill setup status: present (`AGENTS.md`, `docs/agents/issue-tracker.md`,
  `docs/agents/triage-labels.md`, and `docs/agents/domain.md`)

## Goal

Update and polish the FiniteBrain Product Client so a Brain administrator enters
one Finite VIP mailbox, reviews the human and account agents included or
excluded by authoritative preflight, sees capacity, scope, roster, key-version,
and expiry consequences, and explicitly commits that exact immutable plan.
Keep existing Brain read, sync, and chat behavior available, refuse legacy
human-only mailbox writes when preflight is unavailable, and keep all backend,
Core, Identity, CLI, and persistent-state changes in the separate account-agent
cohort lane.

## Durable Artifacts

- CONTEXT updates: none in this lane; the backend cohort task owns the accepted
  glossary update and ADR integration
- ADRs: ADR 0045 in the read-only backend worktree is authoritative
- Prototype source branch, if any: none; this is a production Product Client
  tracer bullet, not throwaway prototype scaffolding
- Spec issue: #441 — Account-agent access cohorts and multi-agent Personal
  Brains
- Tickets: #442 (preflight), #443 (commit), #446 (explicit reduced-set
  approval), #449 (later mailbox-addressed Folder access), and #450 (revocation
  and reconciliation). These already own frontend parity; no duplicate frontend
  issue is needed.
- Ticket sessions:
  `2026-08-10-account-agent-invite-product-client-issue-session.md`
- Agent briefs: source delegation plus live coordination with backend task
  `019fd8fe-28f0-7023-b954-3ab426582c2f`
- Review packets: final Standards and Spec axes reported no actionable findings
- Local CodeRabbit report: four passes; all findings resolved and final pass clean
- PR URL: pending

## Contract Snapshot

Captured 2026-08-10 from the live backend thread and read-only worktree
`/Users/plebdev/Desktop/Projects/finite-mono-account-agent-cohorts` on branch
`codex/account-agent-access-cohorts` at base commit `0903b426` with substantial
uncommitted implementation changes:

- Preview: `POST /v1/brains/{brainId}/invitations/preflight` with
  `targetEmail`, `folderOnly`, `initialFolderAccess`, and `expiresAt`.
- Preview response: `planId`, `targetEmail`, `scope`, `rosterRevision`,
  `participants`, `excluded`, `keyVersions`, `capacity`, and `expiresAt`.
- Each participant has `relationship`, friendly `name`, readable `nip05`,
  distinct `npub`, and `ready`. Normal Product Client copy must not display the
  raw npub.
- Commit: `POST /v1/brains/{brainId}/invitations` with the mailbox target,
  selected scope and expiry, immutable `planId`, one encrypted
  `participantGrant` per planned participant and Folder key version, and exact
  `approvedExclusions` NIP-05 values.
- Restricted-Folder commit: `POST
  /v1/brains/{brainId}/folders/{folderId}/account-access` with `targetEmail`,
  expiry, immutable `planId`, exact `approvedExclusions`, one participant grant
  per included principal, and the signed access-change event. The response is a
  participant-aware `granted` or `already_applied` receipt with fresh metadata;
  the client must not decompose this atomic write into per-npub calls.
- Restricted-Folder removal: `POST
  /v1/brains/{brainId}/folders/{folderId}/account-access/removal-preflight`
  previews friendly cohort identities plus removed, independently retained, and
  required-recipient machine sets. After explicit confirmation, one `DELETE
  /v1/brains/{brainId}/folders/{folderId}/account-access` removes cohort
  provenance, preserves independent access, and rotates the Folder key once.
- Targeted account-agent Folder removal keeps the existing authoritative
  `DELETE /v1/admin/brains/{brainId}/folders/{folderId}/access/{targetNpub}`
  rotation contract. The backend records `targeted_folder_revocation`, keeps
  the anchoring human and sibling agents, and returns durable
  `accountAccessCohorts` metadata. Each projected participant retains its
  principal relationship and `excludedFolderIds`, so the client can route an
  agent row independently after reload rather than treating its NIP-05 as a
  deliverable human mailbox.
- Authenticated Human Intent is additive on the same exact-npub Folder route
  only when one ready Personal Brain agent restricts or restores a distinct
  ready sibling agent. Owner-human requests omit it. This Product Client does
  not manufacture or manually prompt for that proof: sibling-agent removal is
  unavailable and fails closed until the human-authorized Chat/CLI transport
  supplies the short-lived exact-scope intent.
- Any excluded participant requires explicit reduced-set approval. A missing or
  mismatched exclusion set returns 409 without mutation.
- Stale plan, changed roster/scope/capacity/key version, expired plan, or a
  backend without cohort preflight must not fall back to the legacy
  mailbox-to-single-npub writer.
- Responses retain `planId`, participant and exclusion facts, and
  `deliveryStatus`; overlapping accepted access may reuse an already-installed
  current grant.
- Metadata additively exposes `personalBrainAgents` with separate agent NIP-05,
  display name, status, roster revision, and optional blocker. Ready entries are
  required recipients for new Personal Brain Folder grants. Once this plural
  roster exists it is authoritative; the legacy singular `personalAgent` is a
  read/fallback projection only and cannot retain write authority.
- Metadata additively exposes `humanAnchoredAgentAuthorities` with acting agent,
  authorizing human, `routine_administration` scope, and `active` status. The
  Product Client must re-check the human's current owner/admin role and must not
  interpret this as ownership, recovery authority, or permission for whole-
  Brain deletion.
- Backend/Core/Identity/CLI/store files and the backend worktree are read-only
  to this Product Client lane.

Reinspect this contract before finalizing tests and again before opening the
pull request.

## Commands

- Install: none; use the Nix-managed repository environment
- Syntax check:
  `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
- Test:
  `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
- Build:
  `scripts/with-dev-env cargo build -p finite-brain-app --locked`
- Visual verification: static seeded verifier plus isolated Rust-served
  `/client` at desktop and narrow widths

## Ticket Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| #442 | AFK | implemented and verified | Standards/Spec clear | none open | focused client/server gates pass |
| #443 | AFK | implemented and verified, frontend commit seam only | Standards/Spec clear | none open | focused client/server gates pass |
| #446 | AFK | implemented and verified, explicit reduced-set confirmation only | Standards/Spec clear | none open | focused client/server gates pass |
| #449 | AFK | implemented and verified, atomic Folder account-access and targeted-agent seams | Standards/Spec clear | none open | focused client/server gates pass |
| #450 | AFK | implemented and verified, cohort removal and one key rotation | Standards/Spec clear | none open | focused client/server gates pass |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None | Existing ADR/spec/tickets resolve the frontend behavior | | | |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| #442/#443/#446/#449/#450 frontend tracer | `0903b4267efbd53c8eafad65ea42958f860ebc0c` | current delegated Codex task | pending publication commit | accepted; no actionable Standards or Spec findings | JS client suite, static verifier shell, Rust server suite, clippy, fmt, app build, and desktop/mobile browser checks pass |

## Verification

- Passed Product Client contract suite and JavaScript syntax checks.
- Passed the static Product Client verifier shell. The full seeded verifier is
  environmentally blocked because `/tmp/finite-brain-smoke-test.sqlite3` and
  `/tmp/finite-brain-smoke-brain-keys.json` are not present; no repository or
  production state was mutated to manufacture them.
- Passed `cargo fmt --all --check`, all 81 `finite-brain-server` tests, server
  clippy with warnings denied, the focused served-client asset test, and the
  `finite-brain-app` build.
- Verified the real Rust-served `/client` and the invitation review UI at
  desktop and narrow mobile widths with no horizontal overflow.
- Final formal Standards and Spec review axes are clear. Local CodeRabbit code
  findings were fixed with regression coverage; final-pass ledger consistency
  findings are reflected in this artifact.

## Open Questions

- None. The approved Product Client seam is the existing deterministic client
  contract suite plus the Rust-served `/client` browser flow. The human
  explicitly requested those tests and an explicit preview/confirm UX.

## Escalations

- Asked the backend task for an explicit Organization agent-authority projection
  after confirming membership alone could not safely drive controls. The backend
  added and confirmed `humanAnchoredAgentAuthorities`; no backend files were
  changed in this lane.
- Reported that an agent-row removal was incorrectly shaped like a human-mailbox
  cohort removal. The backend retained the targeted-principal DELETE, made it
  persist Folder-scoped exclusion provenance, and added the durable
  `accountAccessCohorts` projection. The client now routes from that explicit
  relationship instead of inferring from a Managed Agent NIP-05.
