# Brain Restore Drill (Litestream → empty target)

Status: active runbook. This is the #459 restore story folded into the
Litestream replication work (#527): brains are a single SQLite file of
ciphertext plus access facts, clients hold the keys, so disaster recovery is
restore-the-file and prove-the-facts. The automated proof is
`built_fbrain_process_brain_restore_drill`
(`finite-brain/crates/finite-brain-cli/tests/fbrain_process_acceptance.rs`);
the automatic Core-departure leg of the same story was removed with the
auth-kernel cut; revocation is now an explicit admin call, which the drill
covers.

## What survives a restore, by construction

- Memberships and admins with full provenance (`delegated_by_npub`,
  `origin_kind`, `origin_ref`, roster revisions on pre-cut rows).
- Pending and accepted npub invitations (a pending one stays acceptable after
  the restore) and any historical approval requests.
- Revocations: a principal removed before the backup stays removed.
- Folder key grants and pending-wrap markers: key-holding clients converge
  on their next sync.
- Nothing plaintext: the backup file is the same ciphertext-bearing SQLite
  the server held all along; the server never sees keys.

## Drill (manual, against a host running the #527 Litestream replication)

1. **Restore the database onto an empty target.**

   ```sh
   # On the brain host (see infra/runbooks/recovery.md
   # for the replica configuration this reads from):
   sudo systemctl stop finite-brain
   litestream restore -o /var/lib/finite-brain/finite-brain.sqlite3 \
     <replica-url>/finite-brain
   # -o onto a fresh path first if you want to compare before cutover; the
   # target directory must be empty or absent before restore ("empty
   # target" — never merge a replica into existing state).
   ```

2. **Start and verify.**

   ```sh
   sudo systemctl start finite-brain
   scripts/finite-status   # brain health green
   ```

3. **Prove the facts** (read-only; adapt principals to the incident window):

   ```sh
   # Memberships with provenance for a known brain:
   sqlite3 -readonly <db> "SELECT user_id, delegated_by_npub, origin_kind, origin_ref \
     FROM brain_members WHERE brain_id = '<brain-id>' ORDER BY user_id"
   # Pending approvals survived actionable:
   fbrain approvals list
   # A departed principal is still gone; an accepted member still lists the brain:
   fbrain brain list --json   # as each principal's agent
   ```

4. **Converge clients.** Key-holding agents sync on their next tick; pending
   folder-key wraps complete then. No client action is required beyond the
   daemon being online.

## Automated proof

`built_fbrain_process_brain_restore_drill` runs the whole story against a
real `fbrain` + real server: populate (cohort invite → approve → accept),
depart a principal, file a pending approval, **clean shutdown → copy →
destroy → restore onto an empty target**, then assert members + provenance
survived, the departed principal stayed out, the pending approval stayed
deniable, and the accepted principal still sees the brain. The clean
shutdown closes the WAL so the file copy is a complete backup — the same
guarantee a Litestream restore point gives you.

Act 15 of the adr46 slice covers the departure leg against real key
material: remove exactly one agent principal, prove the human membership and
both working trees survive, and unrelated sync continues.
