# FiniteBrain Effective Access and Concurrent Change Ledger

## Run

- Run ID: `2026-07-19-fbrain-effective-access-concurrency`
- Loop: Feature Dev
- Target repo: `finitecomputer/finite-mono`
- Base branch: `main` (existing Brain mega-PR override)
- Feature branch: `codex/brain-personal-agent`
- Human owner: Austin
- Started: 2026-07-19
- Current status: all three slices and whole-branch review remediations are
  implemented; final repository gates and PR publication are in progress
- Skill setup status: present (`finite-brain/AGENTS.md` and
  `finite-brain/docs/agents/`)

## Goal

Make `fbrain` clearly report who already has effective Folder Access, return a
truthful already-has-access result for redundant Folder Key Grants, and guide
the managed FiniteBrain agent to attribute concurrent Brain changes to the
recorded actor instead of guessing that its own failed command caused them.
Keep the work on the existing Brain mega-branch and PR #121.

## Durable Artifacts

- CONTEXT updates: none; existing Folder Access, Brain Admin, Folder Key Grant,
  and Brain Working Tree terms already resolve the behavior
- ADRs: none; this restores existing access and audit semantics
- Prototype source branch, if any: none
- Spec issue: #127 — https://github.com/finitecomputer/finite-mono/issues/127
- Tickets: #128, #129, #130
- Ticket sessions: #128, #129, and #130 recorded
- Agent briefs: recorded in each issue session
- Review packets: #128, #129, and #130 recorded
- Local CodeRabbit reports: three completed rounds, all findings addressed
  (`2026-07-19-coderabbit-round-{1,2,3}.md`)
- PR URL: https://github.com/finitecomputer/finite-mono/pull/121

## Commands

- Install: Nix/direnv-provided development environment
- Typecheck: Rust compiler plus managed-skill static checks
- Test: focused `finite-brain-store`, `finite-brain-server`, and `finite-brain-cli`
  tests; managed-skill static checks; full workspace suite at the final gate
- Build: root workspace build through the existing CI command surface
- Visual verification: synthetic two-principal Organization Brain CLI flow with
  a concurrent access mutation and actor-attributed activity output

## Ticket Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| #128 | AFK | complete | direct worker review passed | none | yes |
| #129 | AFK | complete | direct worker review passed | none | yes |
| #130 | AFK | complete | two-axis re-review passed | none | yes |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| None | | | | |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| #128 | `fc6dad8` | `/root/ticket_128_effective_access` | `b8dcb7ac` | standards pass; spec pass | focused public CLI tests (2); full `finite-brain-cli` tests (95); fmt; clippy; diff check |
| #129 | `5435f62` | `/root/ticket_129_redundant_grant` | `ff6b5261` | standards pass; spec pass | CLI (97), server (56), and store (46) tests; fmt; clippy; diff check |
| #130 | `eedb5d7` | `/root/ticket_130_concurrent_actor` | `ae96d5ad` | standards pass; spec pass after fixes | CLI (97); two-Member-Identity signed sync; static skills (47); byte equality; fmt; clippy; diff check |

## Open Questions

- None. The accepted behavior is: display effective access, treat an existing
  current-version grant as already satisfied, and attribute concurrent changes
  from signed activity rather than temporal proximity.

## Whole-Branch Review

- The first standards/specification review found ten issues spanning canonical
  Personal Brain recipients, atomic Folder grants, bounded rotation fanout,
  truthful access fields, Personal Agent lifecycle preservation, and verified
  user-first setup. Commits `1e7ecfe`, `ae28c20`, `50b1eee`, and `e055bfc`
  addressed them.
- The re-review found two remaining gaps: Personal Brain collaborator removal
  could omit the owner and Personal Agent, and one-Personal-Brain-per-owner was
  not database-enforced across connections. Commit `b1ba347` centralized the
  recipient calculation, added a partial unique index, serialized creation, and
  added repeatable two-connection coverage with rollback assertions.
- Three local CodeRabbit rounds reported nine, two, and one findings. All were
  addressed in `3cf93fc`, `73b97c0`, and the final review checkpoint. The fixes
  include post-commit keyring publication, least-privilege local Brain
  credentials, corrected managed-skill commands and references, stronger
  deletion smoke checks, and consistent superseded/future-state docs.
- The large Product Client module remains non-blocking architecture debt. This
  run did not expand scope into a module-boundary refactor.

## Verification

- Focused Brain suites passed: Product Client JavaScript, store (54), server
  (60), CLI (97), core (33), static skills (47), and repeated competing
  Personal Brain bootstrap tests.
- `cargo fmt --all -- --check`, workspace clippy with warnings denied, managed
  skill/reference byte equality, JavaScript syntax checks, and `git diff
  --check` passed after review remediation.
- Dashboard lint, 208 unit tests, two browser tests, and production build passed
  earlier on this branch. The final `cargo test --workspace --locked --
  --test-threads=1` rerun passed against real isolated PostgreSQL after all code
  and managed-skill changes.

## Escalations

- None.
