# Organization Brain sharing feature ledger

## Run

- Run ID: `organization-brain-sharing-2026-07-23`
- Loop: `feature-dev`
- Target repo: `finitecomputer/finite-mono`
- Base branch: `codex/brain-personal-agent`
- Feature branch: `codex/hybrid-wiki-search-slices-1-3`
- Human owner: Austin Kelsay
- Started: 2026-07-23
- Current status: ticket 217 independently approved; ticket 218 is next
- Skill setup status: configured for GitHub, canonical triage labels, and
  multi-context domain documentation

## Goal

Make sharing an Organization Brain with another Agent a complete, observable
workflow: resolve the Agent identity, assign the requested Brain role, grant
access to the intended existing restricted folders without weakening the
encryption boundary, clearly report partial completion, and teach both humans
and Agents how to inspect and repair access.

## Queue

- Publish the Organization Brain Collaboration domain model, specification, and
  ticket graph.
- Implement and review #217: convergent server and CLI collaboration.
- Implement and review #218: Product Client access truth and repair.
- Implement and review #219: managed skill behavior and cross-surface smoke.
- Complete repository verification, CodeRabbit review, PR update, and the fresh
  two-Agent acceptance run.

## Durable artifacts

- CONTEXT updates: `finite-brain/CONTEXT.md`
- ADRs: `finite-brain/docs/adr/0034-make-organization-brain-collaboration-a-desired-state-operation.md`
- Prototype source branch, if any: none
- Spec issue: https://github.com/finitecomputer/finite-mono/issues/216
- Tickets: #217, #218, #219
- Ticket sessions: pending
- Agent briefs: pending
- Review packets: pending
- Local CodeRabbit report: pending
- PR URL: https://github.com/finitecomputer/finite-mono/pull/172

## Commands

- Install: pending repo command discovery
- Typecheck: pending repo command discovery
- Test: pending repo command discovery
- Build: pending repo command discovery
- Visual verification: two-Agent Organization Brain sharing smoke test

## Ticket ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| #217 | server + CLI tracer | approved | round 4 approved | none | store/server/CLI suites + clippy + two-Finite-Home acceptance green |
| #218 | Product Client tracer | ready | pending | pending | no |
| #219 | managed skill + smoke tracer | blocked by #217 and #218 | pending | pending | no |

## Parked HITL slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| none | — | — | — | — |

## Issue session ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| #217 | `894d1ba` | `2026-07-23-issue-217-session.md` | `d13914f` | standards + spec approved in round 4 | store/server/CLI + clippy + built-process acceptance green |

## Open questions

- None. The human owner delegated product decisions; the recommended
  desired-state collaboration interface and highest practical testing seam
  were adopted.

## Escalations

- None.
