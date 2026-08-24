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
- 2026-08-24T20:21:02Z: ran the canonical fleet-status collector from lat1 using its documented temporary staging path. The overall result was red only because of the pre-existing unrelated Smoke Studio straggler. Austin-relevant readiness was green: Runner drain was off, 29 of 32 lat3 slots were occupied, and the expected Runtime artifact and `deepseek-v4-flash-0731` model were active.
- 2026-08-24T20:22:30Z: selected an Austin-only Recovery Set on the FileVault-encrypted operator Mac, followed by restoration into a new empty scratch directory. The live source is about 23 GiB, up from the 5.9-GiB planning snapshot; available local and box1 storage remains sufficient.
- 2026-08-24T20:23:57Z: froze only `austin-finite` after proving its image, single-PVC mount contract, PV identity, and local path. The no-writer check found zero writable file descriptors or memory maps.
- 2026-08-24T20:26:24Z: whole-volume inventory found 230,854 entries and failed closed on 19 generated external symlinks. All were old Python/Nix environment links or a PulseAudio `/tmp` link; no user file was unreadable or unclassified.
- 2026-08-24T20:37:18Z: sealed the complete 23.16-GB source home into a 10.35-GB compressed Recovery Set on the independent encrypted Mac.
- 2026-08-24T20:37:41Z: restarted Austin on the same image with zero container restarts. The Recovery Set freeze lasted 13 minutes 44 seconds.
- 2026-08-24T21:05:14Z: restored the Recovery Set into a new empty box1 scratch directory. The restored byte count matched the source and both trees produced the same 230,854-entry inventory hash.

## Gates

- [x] Reviewed migration code is on `main`.
- [x] Full CI is green for the merged code.
- [x] Independent Austin Recovery Set is current and hash-verified.
- [x] Recovery Set restores into an empty isolated target.
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
- The Recovery Set requires a short Austin-only freeze so the complete
  `/home/node` tree has a single point-in-time boundary. Austin will be
  restarted and verified immediately after the archive is sealed. The final
  cutover freeze remains a separate later step.
- `docs/runs/` already has a different ACTIVE run. This evidence lives under
  `docs/research/` so it does not create a second work-authorizing run.

## Paper cuts and surprises

- The repository's canonical fleet-status tool is not installed as a command
  on every production host. Preflight must use the documented staging path;
  ad hoc host queries do not replace it.
- The legacy backup wrapper and rsync.net credential problem are separate from
  Austin's data migration. Keeping them separate made the recovery boundary
  clearer.
- Austin's durable home grew from the 5.9-GiB planning estimate to about
  23 GiB before execution. Every migration needs a fresh size check before
  reserving transfer, scratch, and outage time.
- Wildcard expansion fails inside a root-only staging directory because the
  calling shell cannot enumerate it before `sudo`. Tool permissions must name
  each reviewed file explicitly.
- The installed `nerdctl` accepts `--mount ... ,ro`; it rejects
  `options=rbind:ro` in that command. Containerd `ctr` still uses the latter.
- Decompressing the Recovery Set before SSH sent 23.56 GB across the network
  and made the empty restore take about 28 minutes. Future rehearsals should
  copy the 10.35-GB compressed file, verify it, then decompress beside the
  scratch target.
- Real Austin state contains generated symlinks into old Nix store and `/tmp`
  paths. They are safe to preserve as inert metadata but must be automatically
  classified as `rebuild` or `quarantine`, never activated or sent to an owner
  for per-file decisions.

## Retro notes

Populate this section during rehearsal, cutover, and observation.
