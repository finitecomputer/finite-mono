# Hosted Web Chat snapshot and empty-target restore

This runbook covers the Hosted Web Chat Recovery Set only: the Hosted Web
Device identity, encrypted client stores and Agent bindings; the complete
Finite Chat server SQLite database; and a custom-format SaaS Core Postgres
dump. The separately retained Agent Runtime is not in this snapshot.

## Admission gate

Paid admission is blocked unless all of these are true, regardless of Stripe:

- `finite-hosted-web-chat-snapshot.timer` runs every 15 minutes and its latest
  successful snapshot is at most 30 minutes old.
- `finite.recoveryBackup.borgRepository` names the dedicated
  `finitecomputer/finite-lat-1` repository at the same rsync.net destination as
  existing finitecomputer backups. It reuses the established finitecomputer
  SSH key, pinned host key, and passphrase bundle. Append-only protection must
  be verified at the destination; reusing credentials does not by itself prove
  that property. The existing off-host passphrase copy and a Borg key export
  remain available independently of finite-lat-1.
- A current archive has passed the empty-target drill below with the dedicated
  synthetic account. A green timer or successful `borg check` is insufficient.

The host definition selects the destination, Borg 1.2 executable, and the same
credential paths used by `../finitecomputer`. On 2026-07-13 the existing bundle
was copied byte-for-byte to finite-lat-1, revision `3f26292` was deployed, and
the dedicated repository was initialized with a verified first archive. The
reused SSH credential also accepted an arbitrary read-only remote command, so
server-enforced append-only protection is **not** present. Paid admission stays
blocked until rsync.net restricts the archival credential without breaking the
separate administrative retention credential, and until the empty-target drill
passes.

## One-time Borg activation

1. Reuse the existing finitecomputer credential bundle. Its source-of-truth
   host path is `/var/lib/finitecomputer/backups/rsync-net`; the ignored
   off-host passphrase copy is already at
   `../finitecomputer/workspaces/trf/secrets/rsync-net-borg-passphrase`. Do not
   print or commit any value.
2. Copy the bundle unchanged to the same root-owned path on finite-lat-1, with
   each file mode `0600` beneath a mode `0700` directory:

   ```text
   /var/lib/finitecomputer/backups/rsync-net/id_ed25519
   /var/lib/finitecomputer/backups/rsync-net/known_hosts
   /var/lib/finitecomputer/backups/rsync-net/borg-passphrase
   ```

3. Verify how the existing SSH key is restricted at rsync.net. The 2026-07-13
   activation proved that it can run an arbitrary remote command, so the
   archival credential is not append-only. This continuity gate remains unmet;
   do not treat the no-prune host job as destination enforcement and do not
   invent a second local passphrase.
4. Deploy an exact committed revision under the normal Nix deployment
   authority, start `finite-hosted-web-chat-snapshot.service`, then start
   `borgbackup-job-finite-hosted-web-chat-offsite.service`. The job initializes
   the dedicated encrypted repository if necessary and uses remote executable
   `borg12`.
5. Export the Borg key with an administrative recovery environment and retain
   it with the passphrase off-host. Never commit either value or print them in
   evidence.
6. Verify both health units below. A repository configured in Nix is not an
   off-host copy until this succeeds.

The host job intentionally does not prune or compact: its append-only
credential must be unable to erase recovery history. Perform reviewed
retention and compaction from an off-host administrative credential after
restore proof; start from the existing finitecomputer policy (7 daily, 4
weekly, 6 monthly) and retain the last 48 hours of 15-minute archives unless a
later accepted retention decision replaces it.

## Snapshot checks

On the source host:

```sh
systemctl status finite-hosted-web-chat-snapshot.timer
systemctl status borgbackup-job-finite-hosted-web-chat-offsite.timer
systemctl status finite-hosted-web-chat-snapshot-health.service
systemctl status finite-hosted-web-chat-offsite-health.service
journalctl -u finite-hosted-web-chat-snapshot -u borgbackup-job-finite-hosted-web-chat-offsite -u finite-hosted-web-chat-offsite-health
latest=/data/recovery-snapshots/hosted-web-chat/latest
test $(( $(date +%s) - $(stat -Lc %Y "$latest") )) -le 1800
(cd "$latest" && sha256sum --check manifest.sha256)
```

The snapshot unit briefly fences both SQLite writers, copies identity and
encrypted binding files, uses SQLite's backup API for every client database
and the server database, takes a `pg_dump --format=custom`, verifies each
artifact, and writes only relative paths and hashes to the manifest.

## Empty-target drill

1. Use the dedicated synthetic account with multiple Topics and Chats in both
   its canonical and legacy associated Rooms, plus one encrypted attachment.
   Record identifiers in an encrypted evidence file; never put them in logs or
   this public repository.
2. Provision an empty isolated target. Public ingress, email, webhooks, push,
   billing jobs, and other outbound side effects stay disabled. Fence the
   separately retained Agent Runtime so it cannot contact both stacks.
3. Extract one Borg archive into a temporary directory outside the target.
   A missing/wrong passphrase or failed extraction stops here and must leave
   the target untouched.
4. Run the verifier/atomic artifact restore:

   ```sh
   FINITE_RESTORE_ISOLATED=1 \
     infra/scripts/restore-hosted-web-chat-snapshot EXTRACTED_SNAPSHOT EMPTY_TARGET/recovery
   ```

   It rejects missing, partial, corrupt, unsupported, or non-empty-target
   restores before installing artifacts.
5. With the target services stopped, install `recovery/hosted-device` as the
   Hosted Web Device StateDirectory and `recovery/finite-chat/server.sqlite3`
   as the Finite Chat database. Preserve ownership and mode from the target's
   Nix units. Create an empty `finite_core`, then restore with
   `pg_restore --exit-on-error --single-transaction --clean --if-exists`.
6. Start Postgres, SaaS Core, Finite Chat, Hosted Web Device, and dashboard in
   isolated mode. Keep public traffic and outbound side effects off.
7. Compare Account, Device, Room, Topic, Chat, message, attachment, Project,
   Runtime, and Agent identifiers with the encrypted preflight evidence. Open
   all retained conversations, decrypt history, and download the attachment.
8. Reconnect only the fenced retained Agent Runtime. Verify the durable owner
   claim replays through the canonical Room and one fresh Agent turn completes.
9. Paul performs the browser checks. Record date, archive name, component
   versions, count-only results, and pass/fail without plaintext or live ids.

Do not switch traffic as part of the drill. A production traffic switch needs
its own authorization and rollback plan.

Schedule the first drill immediately after the first verified archive and
repeat it before paid admission and after any snapshot-format/schema change.
The operator records the scheduled date in the active run's Acceptance Request;
this public runbook does not invent an appointment.

## Negative drill

Before admission, prove that a wrong key, truncated archive, modified
artifact, missing database, and non-empty target each fail before target
mutation. After any schema or snapshot-format change, repeat both positive and
negative drills.
