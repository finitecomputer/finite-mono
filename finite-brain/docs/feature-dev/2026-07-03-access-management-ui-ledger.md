# Access Management UI Feature Ledger

## Run

- Run ID: 2026-07-03-access-management-ui
- Loop: Feature Dev
- Target repo: finitecomputer/finite-brain
- Base branch: `feature/asset-source-notes` working surface; nominal PR base still needs human choice because this deployed UI is ahead of `staging`
- Feature branch: feature/access-management-ui
- Human owner: Austin
- Started: 2026-07-03
- Current status: stacked PR open against `feature/asset-source-notes`
- Skill setup status: present (`docs/agents/issue-tracker.md`, `triage-labels.md`, `domain.md`)

## Goal

Make the Product Client Access sidebar simple enough to understand at a glance. The current Access UI is overwhelming, visually noisy, poorly segmented, and has overlapping styles. Re-approach it from zero while preserving the Obsidian-like theme and the core truths of Vault membership, Folder access, restricted sharing, invitations, and Folder-key state.

## Durable Artifacts

- CONTEXT updates: none planned unless terminology changes
- ADRs: none planned; this is UI composition around existing access semantics
- PRD issue: not created; single AFK UI cleanup carried directly in this session
- Slice issues: not created; single slice tracked below
- Issue sessions: this implementation session
- Agent briefs: none
- Review packets: this ledger plus local browser captures
- Local CodeRabbit report: attempted; blocked by free CLI rate limit, fallback in-thread review completed
- PR URL: https://github.com/finitecomputer/finite-brain/pull/72

## Commands

- Install: not needed; repo dependencies already present
- JS syntax: `node --check crates/finite-brain-server/src/product-client.js`
- JS seams: `node crates/finite-brain-server/src/product-client.test.js`
- Format: `cargo fmt --check`
- Typecheck: `cargo check --workspace`
- Test: `cargo test -p finite-brain-server`; `cargo test --workspace`
- Build: `cargo build --workspace`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Diff hygiene: `git diff --check`
- Visual verification: local Product Client at `http://127.0.0.1:4021/client`; browser fixture for populated `People` and `Links` Access states at 1280x720 and 390x844
- Local CodeRabbit: `coderabbit review --agent --type committed --base feature/asset-source-notes` returned recoverable `rate_limit` with an 8 minute free CLI wait
- Fallback review: in-thread standards/spec review against `feature/asset-source-notes...HEAD`; found and fixed organization member double-counting in the new People summary

## Slice Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| Access sidebar simplification | AFK | complete locally | local review | none known | yes |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None | | | | |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| Access sidebar simplification | `feature/asset-source-notes` | current Codex thread | local `HEAD` | local checks passed after fallback fix | JS syntax/seams; Rust fmt/check/test/build/clippy; visual fixture |

## Open Questions

- PR target/base decision resolved as a stacked PR. The screenshot/live Smoke surface exists on `feature/asset-source-notes`, so PR #72 targets that branch and should land after/with #71 instead of dragging the whole stack into a UI-only review.
- Recommended design direction implemented: keep the Access panel focused on one selected Folder, expose the three jobs as segmented modes (`Overview`, `People`, `Links`), move Vault invitations into a separate compact block, and make the Folder list a secondary selector rather than competing with the primary task.

## Verification Notes

- Live `/client` check confirmed the Access sidebar opens, uses `role="group"` plus `aria-pressed` buttons instead of a global `role="tablist"`, and the Access panel is `display: block` with `overflow: auto`.
- Populated visual fixture confirmed no section overlap, no measured text overflow, no horizontal document overflow, and a scrollable Access panel in the long narrow `Links` state.
- Fallback review found the `all_members` People summary was double-counting admins because organization admins are already members. Fixed to count `members` only and added a JS seam assertion.
- Temporary local servers were stopped after verification.

## Escalations

- None.
