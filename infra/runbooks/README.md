# Runbooks

Operational procedures for everything Finite runs. Consolidated 2026-08-29
(essentials task 10) from 27 per-service runbooks into three survivors plus
the two in-flight cutover records. Current fleet roles live in
[`infra/README.md`](../README.md); executable NixOS configuration is
authority for declared NixOS state; a fresh read-only inventory is authority
for physical state. These runbooks must name which source they rely on
rather than silently promoting an old capture. Source privacy is not a
secret boundary: **no secret values, ever** — env var names and locations
only (`infra/README.md`, secrets policy).

Every runbook states PRECONDITIONS, STEPS, VERIFY, ROLLBACK. Steps that
have not been exercised yet are marked `TODO:` with what must be learned.

Standing rules (one status command, snapshot-SQLite handling,
land-in-a-day, nothing built on a prod box, backup realism) live in
[`infra/README.md`](../README.md). The `finite-status` contract is
documented beside the script (`scripts/finite-status`,
`scripts/finite_status.py`). Finite Private operational tooling is the
executable `scripts/finite-private-ops` wrapper (moved here from
`infra/runbooks/finite-private-ops.sh` 2026-08-29); the model/container
facts live in [`infra/tinfoil/model-inventory.md`](../tinfoil/model-inventory.md).

## Index

| Runbook | Covers |
|---|---|
| [release.md](release.md) | **Production release & rollback** — what ships from where, the wave (runners before Core), closure mechanics + protected-branch conductor, preconditions, the VERIFY ritual, classification/risky paths, rollback levers R1–R4, CLI releases |
| [incident.md](incident.md) | **Incident response** — first five minutes + per-host access, chat availability card, billing incidents, app-plane host failure (ADR 0007 pattern + single-writer doctrine), runner host rebuild, escalation boundaries |
| [recovery.md](recovery.md) | **Data recovery & repair** — custody model, Recovery Sets and authorities, the drills (Postgres/hosted-web-chat/litestream/brain), restoring into production, the exact relocation transaction, boundaries |
| [lat2-replacement-cutover.md](lat2-replacement-cutover.md) | **TIMING GATE — retained until Gate E closes.** The in-flight emergency cutover record; its host-failure pattern then lives in `incident.md` §4 permanently |
| [lat4-nixos-runner-install.md](lat4-nixos-runner-install.md) | **TIMING GATE — retained until Gate F closes.** The in-flight runner install + fleet adoption record; the install pattern then folds into `incident.md` §5 and the relocation remainder into `recovery.md` §5 |

## Release checklist discipline

Two rules apply to **every** release and promotion, no exceptions:

1. **Every release and promotion edits exactly one source of truth** — see
   the table in [`release.md`](release.md) §1. There is no hand-maintained
   ledger to keep in sync.
2. **Rung-ladder: local proof → Docker proof → Kata → Phala/Tinfoil.**
   Nothing is promoted to a confidential-compute lane without a recorded
   proof at the rung below it; the canonical image workflow builds once,
   smokes that exact image, and publishes those exact bytes
   ([`release.md`](release.md) §2(c)).
