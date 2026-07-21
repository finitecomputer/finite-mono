# fbrain Agent CLI Feature Ledger

## Run

- Run ID: `2026-06-24-fbrain-agent-cli`
- Loop: Feature Dev
- Target repo: `finitecomputer/finite-brain`
- Base branch: `staging`
- Feature branch: `feature/fbrain-agent-cli`
- Human owner: Austin
- Started: `2026-06-24T20:46:36Z`
- Current status: MVP implementation complete; hardening follow-ups remain
- Skill setup status: present (`AGENTS.md`, GitHub issue tracker, triage labels, single-context domain docs)

## Goal

Build the `fbrain` interface for agents end to end. Agents should work in a
Brain Working Tree as ordinary files while `fbrain` controls identity, daemon
state, local signer setup, automatic sync health, Folder Key opening, blocked
sync states, activity, and access explanations.

## Alignment Decisions

- Invocation name: `fbrain`.
- Discarded term: `Volumes`; use `Brain Working Tree` for the local readable
  agent workspace.
- Sync model: automatic by default through an Agent Sync Daemon. Manual
  push/pull is not the happy path.
- Prototype signer: agents initially use a local Nostr keypair exposed through
  a simple NIP-07-like signer interface.
- First-class blocked states: conflicts, missing auth, missing keys, locked
  Folders, server health, and access explanations must be visible to agents.
- Machine-readable output: status-bearing commands need stable `--json`.

## Durable Artifacts

- CONTEXT updates: Agent CLI, Agent Sync Daemon, Local Agent Signer, Blocked Sync State
- ADRs: none yet; current decisions are prototype-facing and reversible
- PRD issue: `#37` PRD: fbrain agent CLI for Brain Working Trees
- Slice issues: `#38`, `#39`, `#40`, `#41`
- Issue sessions: `docs/feature-dev/2026-06-24-issue-38-fbrain-agent-cli-session.md`
- Agent briefs: pending
- Review packets: `docs/feature-dev/2026-06-24-issue-38-fbrain-agent-cli-review-packet.md`
- Local CodeRabbit report: pending
- PR URL: `https://github.com/finitecomputer/finite-brain/pull/42`

## Commands

- Format: `cargo fmt --check`
- Typecheck: `cargo check --workspace`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- CLI test: `cargo test -p finite-brain-cli`
- Server regression test: `cargo test -p finite-brain-server`
- Full test: `cargo test --workspace`
- Build: `cargo build`
- Diff hygiene: `git diff --check`
- Live smoke: local app on `127.0.0.1:4017` with temp SQLite; verified auth,
  Brain create, duplicate Personal Brain creation for same owner, open auto-sync,
  status JSON, Folder create, and repeated metadata requests.
- Visual verification: not applicable for CLI; use command output and JSON contracts

## Slice Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| `#38` fbrain MVP CLI control surface | AFK | complete | direct review packet | none | yes |
| `#39` fbrain automatic daemon sync loop | AFK | MVP covered | direct review packet | resident background process and file watcher | yes |
| `#40` fbrain secure live signer and encrypted route integration | AFK | MVP covered | direct review packet | encrypted object writeback from file changes | yes |
| `#41` fbrain sharing/invite/admin command coverage | AFK | MVP covered | direct review packet | access-removal rotations and connection member updates | yes |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None | | | | |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| `#38` fbrain MVP CLI control surface | `e2e9759` | current thread | `6869a30` | pass | `fmt`, `check`, `clippy`, `test`, `build`, live smoke |

## Open Questions

- None.

## Escalations

- Existing main worktree is dirty with Product Client UI work, so this run uses a sibling worktree.
