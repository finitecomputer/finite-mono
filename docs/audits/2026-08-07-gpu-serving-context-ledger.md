# GPU serving context repair ledger — 2026-08-07

## Run

- Run ID: `gpu-serving-handoff-20260807`
- Loop: Improve Context
- Target repo: `finitecomputer/finite-mono`
- Base branch: `ops/inkling-small-h200-20260807`
- Context branch: `context/gpu-lab-20260807-handoff`
- Human owner: Austin
- Started: 2026-08-07
- Current status: complete

## Context frame

- Starting concern: cement and back up the temporary eight-H200 campaign,
  especially the likely DeepSeek production update tonight.
- Out of scope: release creation, Tinfoil relaunch, secrets, traffic, or any
  production mutation.
- Context surfaces: three model research reports, Tinfoil candidate configs,
  Finite Private runbooks and helper, satellite tags/releases, live read-only
  Tinfoil status, root agent rules, runbook index, and repo context map.
- Source-of-truth note: model reports own lab measurements; candidate YAML owns
  desired executable config; measured satellite releases own deployed bytes;
  live Tinfoil status owns observed control-plane state.

## Audit findings

| Finding | Artifact | Evidence | Decision |
| --- | --- | --- | --- |
| Campaign evidence was split across three branches | Campaign handoff | Remote branch/commit verification | Add one pointer map; keep detailed results in model reports |
| Retry-2 runbook still described GLM as production | Retry-2 runbook | Tinfoil reported DeepSeek retry-2-3 ready | Mark old status historical and route tonight to a new scheduler runbook |
| Tonight's change could be mistaken for a model cutover | Scheduler runbook | Current release versus candidate structural diff | State that only 64/512 changes to 128/2048 |
| Immediate rollback was ambiguous | Scheduler runbook | Current production exact tag is known and ready | Use current DeepSeek baseline as immediate rollback; GLM is escalation only |
| Inkling could be mistaken for deployable | Campaign handoff | All vLLM raw-token gates failed; SGLang test incomplete | Preserve a hard not-deployable boundary |

## Routing decisions

- Accepted: campaign map, scheduler-only runbook, runbook index entry, stale
  retry-2 status correction.
- Dropped: duplicating every benchmark table into the campaign map.
- Parked: satellite release publication and production promotion; both require
  separate operator authority.
- Source-of-truth conflicts: resolved from current Tinfoil state plus exact
  released and candidate configs.
- Grilling: not needed; the requested goal and executable diff are explicit.
- Human decisions: likely production work tonight, but no exact release tag or
  relaunch authorization was supplied.

## Patch packet

- Packet: [2026-08-07-gpu-serving-context-patch-packet.md](2026-08-07-gpu-serving-context-patch-packet.md)
- Patch type: documentation-only
- Evidence: pinned commits, config hashes, measured results, release artifacts,
  and read-only Tinfoil status.
- Non-context work parked: release creation and rollout.

## Drift check

| Check | Result | Notes |
| --- | --- | --- |
| Links | passed | All relative Markdown targets exist |
| Paths | passed | Model reports, candidate, helper, and status command verified |
| Commands | passed | Helper syntax and both candidate contracts passed |
| Contradictions | passed | Retry-2 and Tinfoil index now distinguish historical GLM cutover from current DeepSeek promotion |
| Docs-only scope | passed | Only Markdown files changed |

## PR and handoff

- PR URL: https://github.com/finitecomputer/finite-mono/pull/461
- Patch commits:
  - `d100f053ec021a677f77e15b71633b766c1db399`
  - `925e34a6882947581fa467237cfa838d4ec808a6`
- Review notes: fixed stale GLM rollback/index language, compatibility-matrix
  release discipline, objective performance bounds, and full rollback gates.
- Production/release handoff: follow the scheduler-promotion runbook only after
  an exact measured tag and fresh explicit rollout approval exist.
- Human-owned follow-up: approve or decline the exact measured release and
  maintenance window.

## Open gates

- None. Production/release work remains separately authorized and parked.
