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
- 2026-08-24T21:13:52Z: the updated classifier inventoried all 230,854 restored entries with zero blocked state. The 19 generated external symlinks were preserved inertly under `rebuild` or `quarantine`.
- 2026-08-24T21:16:16Z: Sites and integrations inventory completed. Austin had no published legacy Sites and six detected integration classes; all remained inactive and no secret values were emitted.
- 2026-08-24T21:16:42Z: the first session export failed before output because SQLite could not open the frozen WAL database directly from a read-only bind mount. The exporter now stages the frozen database/WAL/shared-memory set in private writable scratch before invoking SQLite's backup API.
- 2026-08-24T21:21:55Z: the repaired v0.14 export completed in 8 seconds with 2,851 sessions, 89,358 messages, and 89 structured-memory facts. The old 2,761-session and 80-fact planning observations were stale and are no longer hard-coded as gates.
- 2026-08-24T21:36:14Z: sealed the 30-GB rehearsal bundle. Its manifest binds all 230,854 source entries, the complete source-home archive, 2,851 sessions, 89,358 messages, 89 facts, 15 scheduled jobs, zero published Sites, and six inactive integration classes.
- 2026-08-24T21:39:00Z: verified every bundle hash inside the exact networkless Hermes v0.20 target image in 84 seconds.
- 2026-08-24T21:41:22Z: created a genuine fresh scratch target with the exact target image and proved its 594-KB rollback archive restores into an empty directory with identical identity and Chat hashes.
- 2026-08-24T21:43:08Z: the first full import failed safely after 90 seconds with SQLite `disk I/O error`. The importer removed its partial transaction and preserved the fresh target's identity, Chat store, zero-session database, and zero-fact memory store. A container probe proved the read-only root also made `/tmp` read-only.
- 2026-08-24T21:45:02Z: restored the pristine scratch target archive and retried with a private, bounded `/tmp` tmpfs. No migration state was repaired by hand.
- 2026-08-24T21:49:11Z: the second import completed in 4 minutes 9 seconds. The receipt records 2,851 sessions, 89,358 messages, 89 facts, 15 review-only scheduled jobs, zero active Sites, and six inactive integration classes. The protected identity and Chat hashes were unchanged.
- 2026-08-24T21:50:12Z: Hermes v0.20 opened the imported databases through its public APIs and reported 2,851 sessions and 89 facts; SQLite integrity was `ok`. The active legacy-skills directory was empty and no active cron file existed.
- 2026-08-24T21:51:02Z: restored the exact installed `source-home.tar` into another empty directory in 21 seconds. All 230,854 entries matched the isolated source restore.
- 2026-08-24T21:57:47Z: classified all 18 session paths that remain unmapped in active state. Six are old Hermes-internal references; twelve are root-level or malformed legacy strings. None is cache-only media, and the complete source home remains in the verified archive. No owner decision is required.

## Gates

- [x] Reviewed migration code is on `main`.
- [x] Full CI is green for the merged code.
- [x] Independent Austin Recovery Set is current and hash-verified.
- [x] Recovery Set restores into an empty isolated target.
- [x] Full real-data rehearsal passes with zero blocked source entries.
- [x] Sites and integrations inventories are complete and secret-free.
- [x] Target image, Hermes version, model, capacity, and Austin-relevant fleet status pass.
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
- A successful inventory printed its complete 18-MB evidence document to the
  terminal. The CLI now prints only status and aggregate counts; exact paths
  and hashes remain in the mode-0600 inventory file.
- Short `nerdctl --rm` proof containers can emit a cleanup-timeout warning
  after their command succeeds. Verification must check the container is gone
  rather than treating that warning alone as a migration failure.
- A frozen WAL database may still require a writable directory when SQLite
  opens it. Mounting the source read-only is correct; the tool must copy the
  frozen database and sidecars to private scratch, then use SQLite's backup API
  from that writable copy.
- Session and memory counts drift while the source is active. The runbook must
  bind verification to counts from the frozen export, not an earlier audit.
- A root-only artifact can appear absent to an unprivileged preflight check.
  Existence and metadata checks must run with the same privilege as staging.
- The manifest and verifier each wrote the complete 26-MB manifest to stdout.
  Operator commands should capture that output and print a concise result.
- A read-only container root also makes `/tmp` read-only. Large SQLite imports
  need a private, bounded tmpfs even when `/data` is the only durable writable
  mount.
- Finite Chat's `app state` command is intentionally read-only and will not
  create a missing client database. A fresh scratch target needs a harmless
  local writer transition such as `app stop`; a real Core-created target does
  this through normal initialization.
- Opening Hermes through its public API requires a writable database directory
  even for verification. Keep the container networkless and the durable
  scratch root writable rather than mounting `/data` read-only.

## Retro notes

Populate this section during rehearsal, cutover, and observation.
