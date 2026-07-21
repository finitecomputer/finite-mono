# Goal Ledger: fbrain Transport And Working Tree Sync

## Run

- Run ID: `2026-06-26-fbrain-transport-working-tree-sync`
- Loop: Feature Dev
- Target repo: sibling `finite-brain` worktree
- Base branch: `staging`
- Feature branch: `feature/fbrain-transport-working-tree-sync`
- Human owner: Austin
- Started: `2026-06-26T23:28:22Z`
- Current status: open non-draft staging PR with green CI; PR CodeRabbit was silent past the wait cap and fallback review is recorded
- Skill setup status: present (`AGENTS.md`, `docs/agents/issue-tracker.md`, `docs/agents/triage-labels.md`, `docs/agents/domain.md`)

## Goal

Do missing rollout items 3 and 6 end to end: harden `fbrain` server transport configuration and wire the agent Brain Working Tree sync loop so agents can use normal files while `fbrain` pulls, decrypts, encrypts, signs, and pushes FiniteBrain object changes.

## Alignment

- Product intent: preserve the trusted-client plaintext boundary. The Agent Runtime may decrypt accessible Pages locally; the server remains encrypted-object only.
- Base branch note: `main` is currently ahead of `staging` in Product Client files, but CLI/core files are identical for this feature scope.
- Human gate: none. The requested items map directly to existing `CONTEXT.md` terms: Agent CLI, Agent Sync Daemon, Brain Working Tree, Local Agent Signer, and Blocked Sync State.

## Durable Artifacts

- CONTEXT updates: none planned unless implementation resolves new terminology
- ADRs: none planned
- PRD issue: `finitecomputer/finite-brain#43`
- Slice issues:
  - `finitecomputer/finite-brain#45` fbrain transport config and HTTPS
  - `finitecomputer/finite-brain#44` Brain Working Tree materialize/writeback sync
- Issue sessions:
  - `docs/feature-dev/2026-06-26-issue-45-fbrain-transport-session.md`
  - `docs/feature-dev/2026-06-26-issue-44-working-tree-sync-session.md`
- Agent briefs: this ledger
- Review packets:
  - `docs/feature-dev/2026-06-26-issue-45-fbrain-transport-review-packet.md`
  - `docs/feature-dev/2026-06-26-issue-44-working-tree-sync-review-packet.md`
- Local CodeRabbit report: `docs/feature-dev/2026-06-27-local-coderabbit-fbrain-transport-working-tree-sync.md`
- PR CodeRabbit report: `docs/feature-dev/2026-06-27-pr-coderabbit-fbrain-transport-working-tree-sync.md`
- PR URL: `https://github.com/finitecomputer/finite-brain/pull/46`

## Commands

- Install: existing Cargo workspace
- Typecheck: `cargo check -p finite-brain-cli`; `cargo check -p finite-brain-server`; `cargo check --workspace`
- Test: `cargo test -p finite-brain-cli`; targeted server tests; `cargo test --workspace`
- Build: `cargo build -p finite-brain-app -p finite-brain-cli`; `cargo build`
- Visual verification: not applicable for CLI-only slices
- Live smoke:
  - Temp DB: `/tmp/fbrain-sync-smoke.oC4srw/finite-brain.sqlite3`
  - Server: `http://127.0.0.1:4016`
  - Commands proved: `auth login`, `brain create personal-beta`, `open`, readable `home`, create/update/delete `home/smoke.md` through `sync now`, empty conflicts, final latest sequence `3`
  - Rerun after local CodeRabbit fixes used temp DB `/tmp/fbrain-sync-smoke.WWDQFD/finite-brain.sqlite3`, brain `personal-gamma`, and final latest sequence `3`
  - Rerun after local CodeRabbit round-two fixes used temp DB `/tmp/fbrain-sync-smoke-round2.BzarH1/finite-brain.sqlite3`, server `http://127.0.0.1:4018`, brain `personal-round2`, and final latest sequence `3` with `conflicts=[]`
  - Final rerun after local CodeRabbit round-three fixes used temp DB `/tmp/fbrain-sync-smoke-final.LlHQ5M/finite-brain.sqlite3`, server `http://127.0.0.1:4019`, brain `personal-final`, and final latest sequence `3` with `conflicts=[]`

## Slice Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| `#45` | AFK | implemented | direct review packet | none | yes |
| `#44` | AFK | implemented | direct review packet | none | yes |

## Parked HITL Slices

None.

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| `#45` | `df69b01521d9e126f430d926a7730f4f4c641d05` | current thread | `8bf422f969aa689c7c0214d70f98df85f1eca7b7` | pass | `fmt`, `check`, `test`, `clippy`, `build`, live smoke |
| `#44` | `df69b01521d9e126f430d926a7730f4f4c641d05` | current thread | `8bf422f969aa689c7c0214d70f98df85f1eca7b7` | pass | `fmt`, `check`, `test`, `clippy`, `build`, live smoke |

## Review Notes

- Fixed point: `df69b01521d9e126f430d926a7730f4f4c641d05`
- Review skill: direct two-axis review used. Sub-agent review was skipped because available multi-agent tooling requires explicit user delegation.
- Standards sources: `AGENTS.md`, `CONTEXT.md`, `docs/agents/domain.md`, `docs/specs/finitebrain-portability-spec.md`, and relevant ADRs.
- Standards result: pass, no findings.
- Spec sources: `finitecomputer/finite-brain#43`, `#44`, and `#45`.
- Spec result: pass, no findings.

## Local CodeRabbit

- Command: `coderabbit review --agent --type all --base staging`
- Availability: completed through the free CLI allowance
- Findings: 8 addressed, 0 ignored
- Fix commit: `f17e40b03b90897e1a929088457b8d0e696c0639`
- Fix evidence:
  - Partial-success sync now rematerializes accepted writes and restores conflicted markdown edits.
  - `timestamp_from_unix` guards oversized values.
  - Folder readability requires the current Folder Key version.
  - Stale moved object paths are removed.
  - Bootstrap grant requests are validated against required recipients before conversion.
  - Plaintext HTTP is restricted to localhost/loopback.
  - `fbrain open` validates the server URL before persistence.
  - Bootstrap grant generation reuses one Folder Key per folder/key version across recipients.
  - Product Client asset test body cap was raised after full-suite verification found the checked-in HTML exceeded the stale 16 KiB test cap.
- Round 2 command: `coderabbit review --agent --type all --base staging`
- Round 2 findings: 4 addressed, 0 ignored
- Round 2 fix commits:
  - `a7ebbb8` Address fbrain CodeRabbit round 2 findings
  - `b73fb40` Fix fbrain HTTP port validation clippy
- Round 2 fix evidence:
  - HTTP redirects are disabled after URL validation.
  - Loopback `http://` host parsing rejects malformed bracketed hosts and ports.
  - Conflict sync fake server reads complete request bodies.
  - fbrain-created Folder Object plaintext now carries encrypted page path metadata while preserving legacy raw markdown fallback.
- Branch hygiene:
  - `331ccd0` restored Product Client asset parity because `f17e40b` had already added asset assertions for Product Client controls without the matching asset updates.
  - Verification passed: `cargo test -p finite-brain-server product_client_serves_spine_assets_and_config`; `node crates/finite-brain-server/src/product-client.test.js`; `node scripts/verify-obsidian-product-client.mjs`.
- Clean verification after round 2:
  - Passed from `/tmp/fbrain-verify-worktree.KosIBG`: `cargo fmt --check && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo build && git diff --check`.
- Round 3 command: `coderabbit review --agent --type all --base staging`
- Round 3 findings: 8 addressed, 1 false positive recorded
- Round 3 fix commit: `f17784c` Address final fbrain CodeRabbit findings
- Round 3 fix evidence:
  - Test HTTP request reads are bounded.
  - Product Client opened-key and onboarding state is key-version/readability aware.
  - README and URL selection now align with loopback-only `http://`.
  - Local sync refuses current-key-missing writes as conflicts instead of using historical Folder Keys.
  - Opened-grant counts and unlocked-folder metadata now reflect newly persisted grants.
  - Encrypted page paths must be `.md`.
- Round 3 finding not addressed:
  - Missing `nostr::JsonUtil` import was a false positive; clean workspace check/test/clippy passed without it.
- Clean verification after round 3:
  - Passed from `/tmp/fbrain-verify-worktree.8TW46X`: `cargo fmt --check && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo build && git diff --check`.

## PR CodeRabbit

- PR: `https://github.com/finitecomputer/finite-brain/pull/46`
- Trigger: `@coderabbit full review`
- Posted: `2026-06-27T15:09:29Z`
- Wait result: no CodeRabbit comments or reviews through `2026-06-27T15:30:43Z`
- Fallback: current-thread direct review, because PR CodeRabbit stayed silent past the Feature Dev loop cap
- CI at fallback: `Rust workspace` passed; `Product Client JavaScript` passed

## Open Questions

- None.

## Escalations

- None.
