# Runbook — remediate MLS sender-rewind wedged rooms (epoch bump + cursor skip)

Status: DRAFT — awaiting review. Direction corrected 2026-09-01 (see
"Incident shape" below; the earlier draft wrongly minted agent devices —
in this incident the agents are the healthy parties). Do not execute
against production until reviewed and explicitly authorized.

## What this fixes

A device whose durable MLS state was rewound keeps sending application
messages encrypted from ratchet generations below the receivers' floor.
MLS error classes at the receiver: `TooDistantInThePast` (deep), then
`SecretReuse` as the rewound sender slowly climbs back into the consumed
window. The receiver quarantines the entry and freezes its durable room
cursor; its UI shows silence while the sender's sends keep being accepted
by the server (shape/membership checks only — the server cannot see
secret-tree generations).

A rewound ratchet cannot be un-rewound, and receiver-side skip-repairs
are NOT durable while the rewound sender is still below the floor: every
new send from it re-quarantines the room (observed in production — a
skip-repaired room re-wedged on the very next entry). The durable lever
is an **epoch-advancing commit**: every epoch transition re-derives all
leaves' secret trees at generation 0, making the rewound sender's stale
generations irrelevant without replacing any device.

## Incident shape (2026-09-01, direction-corrected)

The rewound senders are the **owners' hosted-web devices on lat2**: their
stores were restored from a banked file pull of lat1's hosted-device tree
— main database files only, no `-wal` sidecars — taken while lat1 was
still serving; each store came back at its last SQLite checkpoint,
silently rewound by its un-checkpointed WAL tail (a quiet room checkpoint
rarely: one owner lost ~13 h of own sends). The relocated agent runtimes
on lat4 were current throughout (their staging shipped WALs); the agents
are the wedged **receivers**.

The rewound-party test in CI covers the mechanism generically
(`rewound_sender_wedges_receiver_and_heals_via_readdmission_epoch_bump`,
finitechat-client/tests/client_state.rs) regardless of which device class
is rewound.

## Remediation, per room

1. **Epoch-advancing commit from the owner side.** The only commit
   vehicle in the product today is add-member: the owner admits a fresh
   device key package (e.g. links a second own device) through the
   normal hosted-web bootstrap, whose add-member commit bumps the room's
   epoch and re-derives everyone's secret trees at generation 0. The
   commit is not secret-tree generation bound, so a still-rewound owner
   device can drive it. Do NOT mint/replace the healthy party's device.
2. **Skip-repair the wedged receiver's cursor — MANDATORY.** The frozen
   tick aborts on the first quarantined entry and can never reach the
   commit behind it (proven in the CI test: the receiver stays wedged
   after the epoch bump until its cursor is advanced). Advance the
   receiver's room cursor past the quarantined backlog, up to the commit
   seq (recovery.md §4a surgery), then let normal sync apply the commit
   and everything after it.
3. **Verify both directions:** owner decrypts the agent's next send;
   agent decrypts the owner's next send (the epoch-2 trees make the
   rewound sender's position moot); receiver cursor tracks the room head;
   no new quarantine entries.

## Room states at time of writing (2026-09-01 survey)

- **Wedged now** (agent cursor frozen; owner still below floor): four
  rooms, including the pilot employee room (owner-side epoch bump + skip).
- **Latent** (owner rewound but has not sent since cutover — the next
  owner message poisons): four rooms. Prefer the epoch bump BEFORE the
  owner speaks; otherwise run steps 1–2 when it happens.
- **Healed by skip-repair** (owner already climbed past the floor): three
  rooms — reading again; no further action unless a re-wedge appears.
- **Fine / owner idle**: the remainder.

Concrete room/runtime identifiers live in the incident handoff notes
(2026-09-01 root-cause correction), not in this repo.

## Preconditions

1. `scripts/finite-status` run and archived (before-state).
2. Evidence copies of both stores (sender and receiver) taken within the
   last 24 h; retake if older.
3. The receiver-side skip tooling is the already-proven recovery.md §4a
   procedure; rehearse on the dev stack once for the owner-driven
   re-admission flow before the first production room.

## Rollback

The commit is ordinary MLS history — permanent and safe; there is nothing
to roll back. The cursor skip is forward-only; if a skip lands the cursor
past unreadable-but-decryptable entries, the transcript shows a gap but
the room stays live (accepted cost, already paid in the healed rooms).

## Prevention (follow-up work, not this runbook)

- Placement gate on the **hosted-web restore lane**: a store must not
  enter service older than the server's accepted sends from that device
  (would have caught every rewind in this incident; the kata relocation
  lane was not the defect).
- Cutover runbook rule: never restore a live WAL-mode SQLite tree by
  copying main files — use the sqlite backup API / `VACUUM INTO` /
  litestream or stop the writer first, and verify `-wal` handling
  explicitly.
- finite-status probe: per room, compare the owner store's max own-send
  seq against the server's accepted sends from that device (cross-host or
  server-side per #804's conclusion). Stuck outboxes and cron-only
  patterns are not reliable signals (outbox tails predate this incident).
