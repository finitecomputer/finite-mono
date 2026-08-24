# Austin Hermes migration exercise

Status: IN PROGRESS

Started: 2026-08-24T20:19:33Z

Source: Austin's Hermes v0.14 bot on box1

Target: a fresh Hermes v0.20 Agent on lat3

Operator procedure: [Migrate one box1 Hermes bot to lat3](../../infra/runbooks/legacy-hermes-box1-to-lat3.md)

## Purpose

Run the first complete box1-to-lat3 export/import with Austin as the canary.
Keep enough evidence to improve the tool and runbook before another person's
bot is migrated. Record timing, unexpected prerequisites, paper cuts, useful
shortcuts, and anything that was easier or harder than expected.

This file contains no credentials, private message content, or private runtime
identifiers. Exact identifiers and hashes are retained in the operator's
mode-0700 evidence directory.

## Approved scope

- Merge the reviewed migration tooling and runbook.
- Create and prove an independent Austin Recovery Set.
- Rehearse the exact migration against an isolated restore.
- Create a fresh Austin target on lat3.
- Freeze only Austin on box1, import into the stopped target, and verify it.
- Keep box1 and both Recovery Sets through the observation window.
- Restore Austin's external behavior only through the supported target flows.
- Do not decommission box1 during the canary.

No other bot may be stopped, restarted, migrated, or changed.

## Timeline

- 2026-08-24T20:19:33Z: PR #658 merged into `main` as `cd5acd22` after the full CI matrix passed.
- 2026-08-24T20:19:58Z: created the root-only local evidence directory. Austin remained live and unchanged.

## Gates

- [x] Reviewed migration code is on `main`.
- [x] Full CI is green for the merged code.
- [ ] Independent Austin Recovery Set is current and hash-verified.
- [ ] Recovery Set restores into an empty isolated target.
- [ ] Full real-data rehearsal passes with zero blocked source entries.
- [ ] Sites and integrations inventories are complete and secret-free.
- [ ] Target image, Hermes version, model, capacity, and fleet status pass.
- [ ] Fresh Austin target is created and identified.
- [ ] Source freeze and no-writer proof pass.
- [ ] Offline import and post-restart verification pass.
- [ ] Minimum 24-hour observation window passes.
- [ ] Retro is complete and accepted changes are folded into the migration contract.

## Deviations and decisions

- The legacy fleet-wide rsync.net job is not the Austin Recovery Set for this
  exercise because it has been failing since 2026-07-22 and repairing it would
  broaden the canary's blast radius. The accepted replacement is an encrypted,
  Austin-scoped copy on an independent machine, followed by an empty-target
  restore. The recoverability requirement is unchanged.
- `docs/runs/` already has a different ACTIVE run. This evidence lives under
  `docs/research/` so it does not create a second work-authorizing run.

## Paper cuts and surprises

- The repository's canonical fleet-status tool is not installed as a command
  on every production host. Preflight must use the documented staging path;
  ad hoc host queries do not replace it.
- The legacy backup wrapper and rsync.net credential problem are separate from
  Austin's data migration. Keeping them separate made the recovery boundary
  clearer.

## Retro notes

Populate this section during rehearsal, cutover, and observation.

