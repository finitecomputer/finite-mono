# Platform reliability checklist — 2026-07-21

This is the short session checklist, not a new runbook. Deployed truth and the
lat1 sequence remain in [`finite-lat-capacity-and-redundancy.md`](finite-lat-capacity-and-redundancy.md).
PR [#110](https://github.com/finitecomputer/finite-mono/pull/110) is the rollout
post-mortem context. [`triage-and-priorities-2026-07-17.md`](../triage-and-priorities-2026-07-17.md)
is historical, not current authority.

## Now

- [x] Prove fresh Finite Chat turns work after an interrupted Hermes turn:
  graceful stop, `SIGKILL`, and stopped-writer empty-target restore.
- [x] Make only fixes required by that test; record the supported recovery
  boundary.
- [x] Run the one-writer AEON Canary 0714 restore drill and require two fresh,
  interactive post-restore chats.

## Before the lat1 RAID maintenance window

- [ ] Prove the complete lat1 recovery set: all Agent `/data`, Hosted Web
  Chat/Core, Sites, Brain, and the secrets bootstrap.
- [ ] Add swap and pin the next lat1 NixOS generation without changing storage.
- [ ] Close the exact lat1 disk inventory and RAID/disko configuration; build
  the reviewed closure on finite-lat-2.
- [ ] Fix the Runner/Core startup ordering found during the lat1 bridge rollout.
- [ ] In an evening window: stop writers, take and verify backups, reinstall
  pinned NixOS with RAID, restore, verify, and keep creation drained until all
  checks pass.

## Next product reliability slices

- [ ] Fail closed when no Agent capacity is available; keep login and existing
  Agents working and tell new users to contact Paul.
- [x] Activate and implement the bounded, default-off repository and synthetic
  [`Runtime Retirement`](runtime-retirement-readiness.md) plan.
- [ ] Separately provision its restricted Borg authority, deploy with all gates
  off, and pass a disposable canary plus independent restore before broad enablement.

## Parked / later

- [ ] Keep finite-lat-2 CI/build-only; reconsider it for Agent capacity later.
- [ ] Revisit Latitude Kubernetes only at the trigger recorded in
  [`parking-lot.md`](parking-lot.md).
- [ ] Preserve provider-neutral Runner boundaries for future Phala/TEE work;
  do not build that integration in this run.
- [ ] Do not let this work block the separately planned multi-device iOS and
  Electron apps, Brain, or Sites improvements.
