# Improve Context Ledger: fbrain Agent CLI

## Run

- Run ID: `2026-06-24-fbrain-agent-cli-context`
- Loop: Improve Context
- Target repo: `finitecomputer/finite-brain`
- Base branch: `staging`
- Context branch: `feature/fbrain-agent-cli`
- Human owner: Austin
- Started: `2026-06-24T21:30:26Z`
- Current status: context patch applied locally

## Context Frame

- Starting concern: after the `fbrain` feature work, align durable agent-facing
  context with the actual CLI invocation and branch review state.
- Specific area of concern, if any: Agent CLI command surface and follow-up
  loop readiness.
- Out of scope: production daemon hardening, HTTPS transport, keychain storage,
  encrypted writeback implementation, and CLI crate restructuring.
- Known commands: `cargo run -p finite-brain-cli --bin fbrain -- --help`,
  `gh pr view --json number,url,isDraft,baseRefName,headRefName,state,title,statusCheckRollup`,
  `git diff --check`.
- Context surfaces inventoried: `README.md`, `CONTEXT.md`, `AGENTS.md`,
  `docs/agents/domain.md`, `docs/specs/finitebrain-portability-spec.md`,
  `docs/feature-dev/2026-06-24-fbrain-agent-cli-ledger.md`,
  `docs/feature-dev/2026-06-24-issue-38-fbrain-agent-cli-session.md`, and
  `docs/feature-dev/2026-06-24-issue-38-fbrain-agent-cli-review-packet.md`.
- Specs or PRDs inventoried: `docs/specs/finitebrain-portability-spec.md`.
- Source-of-truth notes: `CONTEXT.md` already owns the Agent CLI glossary terms;
  `README.md` owns user-facing local command examples; feature ledgers own run
  evidence and PR state.

## Audit Findings

| Finding | Artifact | Evidence | Decision |
| --- | --- | --- | --- |
| Agent CLI examples used repo-root `cargo run ...` commands even after entering a Brain Working Tree. | `README.md` | `README.md` Agent CLI block; `crates/finite-brain-cli/Cargo.toml` defines binary `fbrain`; `fbrain --help` runs with the command surface. | Use canonical `fbrain` examples and keep repo-development Cargo invocation as a short note. |
| fbrain feature ledger still said PR URL was pending after PR creation. | `docs/feature-dev/2026-06-24-fbrain-agent-cli-ledger.md` | `gh pr view` returned open non-draft PR `#42` targeting `staging`. | Record the PR URL and concrete feature commit SHA. |
| CLI crate module depth is structural friction, not context drift. | Improve Codebase handoff | `crates/finite-brain-cli/src/lib.rs` contains the full command parser, local state, signer, HTTP client, sync, and admin command handling. | Park for the subsequent Improve Codebase round. |

## Routing Decisions

- Accepted findings: README command example correction; feature ledger PR/commit
  evidence correction.
- Dropped findings: no `CONTEXT.md` glossary change; the relevant Agent CLI
  terms are already present and consistent.
- Parked findings: CLI crate module depth and production-hardening work.
- Source-of-truth conflicts: none.
- Grilling sessions: none needed; the command name and terminology were already
  settled by the user and recorded in feature-dev artifacts.
- Human decisions: use `fbrain`; use `Brain Working Tree`, not `Volumes`;
  automatic sync is the normal model.

## Patch Packet

- Packet path: `docs/improve-context/2026-06-24-fbrain-agent-cli-context-patch-packet.md`
- Patch type: documentation-only
- Files changed: `README.md`,
  `docs/feature-dev/2026-06-24-fbrain-agent-cli-ledger.md`,
  `docs/improve-context/2026-06-24-fbrain-agent-cli-context-ledger.md`,
  `docs/improve-context/2026-06-24-fbrain-agent-cli-context-patch-packet.md`
- Evidence summary: `fbrain --help` output matches the command families;
  GitHub reports PR `#42` open and non-draft against `staging`.
- Non-context work parked: resident daemon process, file-watch encrypted object
  writeback, HTTPS transport, platform secret backend, permission-removal
  hardening, and CLI crate modularization.

## Drift Check

| Check | Result | Notes |
| --- | --- | --- |
| Links | pass | Referenced repo files exist. |
| Paths | pass | `crates/finite-brain-cli/Cargo.toml` and referenced run artifacts exist. |
| Commands | pass | `cargo run -p finite-brain-cli --bin fbrain -- --help` and `git diff --check` passed. |
| Contradictions | pass | Old `cargo run` examples remain only in development notes, not as Brain Working Tree commands. |
| Docs-only scope | pass | Patch touches README and docs only. |

## PR And Handoff

- PR URL: `https://github.com/finitecomputer/finite-brain/pull/42`
- Commit SHA: pending until this context patch is committed
- Review notes: drift check passed locally
- Feature Dev handoff: none; the fbrain MVP feature PR already exists.
- Improve Codebase handoff: inspect `crates/finite-brain-cli/src/lib.rs` module
  depth as the likely first structural candidate.
- Deployment handoff: none.
- Human-owned follow-up: select an Improve Codebase candidate after the
  candidate report.

## Open Gates

- Candidate selection gate belongs to the subsequent Improve Codebase loop.
