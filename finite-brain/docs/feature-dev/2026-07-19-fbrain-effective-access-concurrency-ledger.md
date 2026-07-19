# FiniteBrain Effective Access and Concurrent Change Ledger

## Run

- Run ID: `2026-07-19-fbrain-effective-access-concurrency`
- Loop: Feature Dev
- Target repo: `finitecomputer/finite-mono`
- Base branch: `main` (existing Brain mega-PR override)
- Feature branch: `codex/brain-personal-agent`
- Human owner: Austin
- Started: 2026-07-19
- Current status: all three slices implemented and individually verified; final
  whole-branch review and repository gates in progress
- Skill setup status: present (`finite-brain/AGENTS.md` and
  `finite-brain/docs/agents/`)

## Goal

Make `fbrain` clearly report who already has effective Folder Access, return a
truthful already-has-access result for redundant Folder Key Grants, and guide
the managed FiniteBrain agent to attribute concurrent Vault changes to the
recorded actor instead of guessing that its own failed command caused them.
Keep the work on the existing Brain mega-branch and PR #121.

## Durable Artifacts

- CONTEXT updates: none; existing Folder Access, Vault Admin, Folder Key Grant,
  and Vault Working Tree terms already resolve the behavior
- ADRs: none; this restores existing access and audit semantics
- Prototype source branch, if any: none
- Spec issue: #127 — https://github.com/finitecomputer/finite-mono/issues/127
- Tickets: #128, #129, #130
- Ticket sessions: #128, #129, and #130 recorded
- Agent briefs: recorded in each issue session
- Review packets: #128, #129, and #130 recorded
- Local CodeRabbit report: pending
- PR URL: https://github.com/finitecomputer/finite-mono/pull/121

## Commands

- Install: Nix/direnv-provided development environment
- Typecheck: Rust compiler plus managed-skill static checks
- Test: focused `finite-brain-store`, `finite-brain-server`, and `finite-brain-cli`
  tests; managed-skill static checks; full workspace suite at the final gate
- Build: root workspace build through the existing CI command surface
- Visual verification: synthetic two-principal Organization Vault CLI flow with
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

## Escalations

- None.
