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

1. **Epoch-advancing commit.** Preferred: `finitechat hermes rekey
   --room <room> --json` from the wedged receiver's agent home — an
   ordinary self-update Commit that bumps the room's epoch and
   re-derives everyone's secret trees at generation 0. It does not need
   the receiver's cursor at the server head (it replays the backlog
   first, step 2, and refuses unless the local epoch then matches the
   server's).
   Alternative when the receiver cannot commit: an owner-side add-member
   commit (the owner links a second own device through the normal
   hosted-web bootstrap); commits are not secret-tree generation bound,
   so a still-rewound owner device can drive it. Do NOT mint/replace the
   healthy party's device.
2. **Backlog: the rekey replays it first, on evidence.** Before it
   commits, `rekey` replays every entry above the receiver's cursor at
   the current epoch through the real apply path. Healthy messages are
   delivered and stored (`applied` in the report); only application
   entries that fail with the MLS application-ciphertext class — the
   rewound sender's poison — are skipped, listed under `skipped` (seq,
   sender account/device, message id, error class), and appended to
   `<agent-home>/rekey-audit.jsonl` (`record: "rekey"`; `--audit-log`
   overrides the path). The same skip rule drives `repair skip-entry`.
   The cursor then lands on `commit_seq` through the normal merge. The
   rewound sender must resend the skipped messages. If anything else
   fails to replay (a Commit or Proposal, any other error class, one of
   the receiver's own entries), the rekey refuses with a typed error and
   changes nothing: sync or repair the room first, then run it again. If
   a previous attempt already committed and left the cursor frozen (a
   run on the old image, or a crash after acceptance), the backlog can no
   longer be classified honestly and the rekey refuses naming the path:
   use `finitechat repair skip-entry` (recovery.md §4a; canonical
   container stopped) — its rehearsal replay crosses the merged own
   commit as a no-op advance, derives the poison-only skip list, and the
   restarted device converges on its next sync. A cursor that was already
   current reports `applied: 0`, `skipped: []`.
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
3. `repair skip-entry` is needed only when the rekey refuses (a
   previous attempt already committed above a frozen cursor); it is the
   already-proven recovery.md §4a procedure. Rehearse the rekey on the
   dev stack once before the first production room.

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
