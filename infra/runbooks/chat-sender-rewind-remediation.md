# Runbook — remediate sender-rewound chat rooms (fresh mint + re-admission)

Status: DRAFT — awaiting review. Do not execute against production until
reviewed and explicitly authorized. Pilot target: the employee-owned agent (scope below).

## What this fixes

An agent whose durable MLS state was rewound (the 2026-08-29 lat1 disk-loss
class) sends application messages encrypted from ratchet generations below
the receivers' floor. Receivers quarantine every entry and freeze their room
cursor; the owner sees a silent bot; the sender's own sends keep being
accepted by the server (shape/membership checks only). Restarts and
receiver-side skip-repairs cannot fix it: a rewound ratchet cannot be
re-advanced legitimately, and every post-restart boot re-loads the rewound
state (the pilot agent was restarted 2026-08-31 13:44 and re-wedged at its next cron).

MLS remedy: advance the room's epoch. Every epoch transition re-derives the
secret trees at generation 0 for all leaves, so a fresh generation space
heals both directions. The only epoch-advancing action in the product today
is an add-member commit, so the epoch bump rides a **fresh device mint +
re-admission**: mint a fresh chat device for the agent (fresh MLS state, no
restored snapshots), let the owner-side profile chat bootstrap re-admit it
(`prepare_add_member_commit` → `submit_commit`, finitechat-core
lib.rs:4165/:4297), and the room re-keys itself.

Why fresh mint and not a restored backup: restoring any older snapshot of
the device state reproduces the disease (this is the hosted-web 2026-08-28
variant). The fresh device must join via a new welcome, never via snapshot.

## Pilot scope (employee-owned agent)

- Agent runtime: `finite-kata-a8eb082ca6dee9f17245` on finite-lat-4
  (legacy container-named durable root — migrate to the runtime-id root as
  part of this repair, see step 6).
- Chat account: `53064899ff1364ce…c0188c1`, device `agent`.
- Room: `room-5c6e775b3525f2ca` (peer: owner device `hosted-web`).
- Receiver store (owner side, hosted-web user
  `8fca2cda23b71ea3…9d78600a` on finite-lat-2): cursor frozen since
  2026-08-28 14:30:53 (seq 15856); ingests exactly one quarantined entry
  per day at the 09:00 cron.

## Preconditions

1. `scripts/finite-status` run and archived (before-state).
2. Evidence copies of both stores already taken (scratch, 2026-09-01):
   sender `client.sqlite3{,-wal,-shm}`, receiver
   `hosted.sqlite3{,-wal,-shm}`. If older than 24h, retake.
3. Mechanical rehearsal is pinned in CI
   (`rewound_sender_wedges_receiver_and_heals_via_readdmission_epoch_bump`,
   finitechat-client/tests/client_state.rs): the wedge reproduces, the
   epoch-bump heal works, and the receiver's frozen cursor provably cannot
   reach the commit without the step-5 skip. What CI cannot cover is the
   operator surface — before the first production run, walk the owner-side
   re-invite once on the dev stack (`just dev up`) to confirm the web app's
   bootstrap path issues the re-admission commit for a fresh agent device.

## Steps (production — requires explicit authorization)

1. **Quiesce the sender.** Stop the agent container on finite-lat-4
   (`nerdctl --namespace finite stop <pilot-agent-container>`). This releases the
   WriterLease; no sends occur during the repair.
2. **Preserve the rewind boundary (rollback).** On the host:
   `mkdir -p /data/finite-saas-runner/kata/.rewind-remediation/<date>-<agent>`
   and move `agent/client.sqlite3{,-wal,-shm,.writer-lease}` into it. This
   is the rollback boundary AND the evidence copy of the rewound state.
   Note: the agent's conversational memory is not in this store (it lives
   in hermes-home); the local chat transcript for the room is lost to the
   fresh device (MLS forward secrecy) — accepted cost for an employee bot;
   server-side history remains.
3. **Mint fresh, with a NEW device id.** While stopped, edit the agent
   home's `config.json` `device_id` (e.g. `agent` → `agent-r2`); the sidecar
   derives its device identity from it (hermes.rs `home.config.device_id`).
   A fresh device that REUSES the old DeviceRef is served the room's full
   history by the server's membership projection and re-wedges on entries
   it cannot decrypt — the new id is required, and it is also what makes
   the projection serve the fresh device only from its own add-commit
   (proven by `rewound_sender_wedges_receiver_and_heals_via_readdmission_
   epoch_bump` in finitechat-client/tests/client_state.rs). Then start the
   container; the boot path opens an empty store and mints the fresh device
   (`recover_or_create_device_state`, finitechat-core lib.rs:8289). Verify
   the sidecar is healthy and `/readyz` reports `server_stream` connected
   (the #779 surfaces). Note: the fresh device cannot see pre-join room
   history (MLS forward secrecy); the agent's conversational memory is not
   in this store (hermes-home), so only the local room transcript is lost —
   accepted cost for an employee bot; server-side history remains.
4. **Re-admit via the owner.** The owner opens the agent chat in the hosted
   web app; profile chat bootstrap detects the agent's fresh key package,
   prepares and submits the add-member commit. Verify on the server
   (scratch copy of `/var/lib/finite-chat/data/server.sqlite3` via
   `finitechat-server snapshot`) that the room's `current_epoch` advanced
   past 1 and the commit seq landed.
5. **Repair the receiver cursor — MANDATORY, not conditional.** The
   receiver's frozen tick aborts on the first quarantined entry and can
   never reach the heal commit behind it in the log (proven in the same
   test: the receiver stays wedged after the epoch bump until its cursor is
   advanced). Skip the receiver's room cursor past the quarantined
   pre-commit backlog, up to the commit seq (the owner already has the
   epoch-2 group state from the bootstrap merge; entries after the commit
   decrypt normally). This is the established skip-repair surgery
   (recovery.md §4a) with the target seq = the re-admission commit's seq.
6. **Verify the heal.**
   a. Agent side: next cron (09:00) or a manual poke sends; outbox drains;
      the entry appears in the owner receiver's ingest.
   b. Owner side: the owner sees the agent's reply in the app (this is the
      product-level success criterion).
   c. Receiver cursor advances past the freeze point; epoch reads 2 on both
      sides.
7. **Migrate the durable root** (pilot agent only, rides this repair): recreate
   the container under its runtime-id root so the legacy container-named
   root retires. The state is fresh (step 3), so this is the cheapest
   moment to fix the naming. Confirm the runner's lifecycle probe accepts
   the new root before deleting the legacy directory (archive it, don't
   delete).
8. `scripts/finite-status` after-state; record epoch before/after, outbox
   count, receiver cursor in the incident record (#780 lane).

## Rollback

Stop the container, restore the preserved `client.sqlite3*` files from
step 2, start the container. State returns to the rewound-but-known wedge;
no further harm (the room's new epoch is permanent MLS history — the old
device simply cannot decrypt post-bump entries, same silence as today).

## Repeat for the fleet

After the pilot proves the procedure end-to-end (one full cron cycle + owner
confirmation), apply to the evidence-based mint list (Phase 0 survey,
2026-09-01): confirmed treadmill — `runtime_eaaef57a…` (outbox 28),
`runtime_60a635e4…` (12), `runtime_13b1f442…` (11, previously
skip-repaired 2026-08-31), `runtime_c2ec7c3d…` (14), `runtime_531e934f…`
(7), `runtime_b602c67f…` (2), plus the lower-count set
(3b9b17, b1b1df, aa877f, c5e2ee, e28696, efa663). Customer-owned agents
need owner coordination for step 4; batch them per owner. Re-run the
outbox/receiver classifier after each batch — the list is evidence, not
dogma.
