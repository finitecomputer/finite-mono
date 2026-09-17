# Hosted Web Chat snapshot and empty-target restore

Snapshots use `finite.hosted-web-chat-recovery-snapshot.v4`: Hosted Web Device
identities and stores, Chat SQLite, Core Postgres, Brain SQLite and Identity
SQLite. Agent Runtime recovery is separate; Sites has its own
[backup procedure](deploy-sites.md#backups-and-restore).

The verifier accepts complete historical v3 archives, including Sites. Restore
those on isolated scratch targets and retain their Sites material without
starting an old writer. v1/v2 archives are rejected.

Snapshots are deploy/manual-triggered and briefly fence writers. Daily Borg
archival does not create a new recovery point. Current health thresholds are
seven days for a snapshot and 50 hours for offsite archival; those are not a
15-minute RPO. TODO: non-disruptive recovery cadence is tracked in
[FIN-95](https://linear.app/finitecomputer/issue/FIN-95).

The host's `finite.recoveryBackup` settings select the Borg repository and
credential paths. Retain the repokey export, passphrase and independent recovery
access outside that host. No host job prunes or compacts. Do not claim
server-enforced append-only protection without verifying credential restrictions.

## Snapshot checks

On the source host:

```sh
systemctl status borgbackup-job-finite-hosted-web-chat-offsite.timer
systemctl status finite-hosted-web-chat-snapshot-health.service
systemctl status finite-hosted-web-chat-offsite-health.service
journalctl -u finite-hosted-web-chat-snapshot -u borgbackup-job-finite-hosted-web-chat-offsite -u finite-hosted-web-chat-offsite-health
latest=/data/recovery-snapshots/hosted-web-chat/latest
age=$(( $(date +%s) - $(stat -Lc %Y "$latest") ))
test "$age" -le 604800      # current health threshold only
(cd "$latest" && sha256sum --check manifest.sha256)
scripts/verify-hosted-snapshot "$latest"
test -f "$latest/recovery-set.tsv"
scripts/snapshot-sqlite integrity-check "$latest/finite-chat/server.sqlite3"
```

Never pass a database below `$latest` to plain `sqlite3`; the helper copies
the database and any WAL/SHM sidecars to private scratch space first.

The snapshot unit briefly fences every writer in the Recovery Set, copies
identity and encrypted binding files, uses SQLite's backup API for every
Hosted Device, Chat, Brain, and Finite Identity database, takes a
`pg_dump --format=custom`, verifies each artifact, and writes relative paths and
hashes to the integrity manifest. `recovery-set.tsv` binds the version to its
component identities. `scripts/verify-hosted-snapshot` validates the exact
manifest for health, archival and restore. Current snapshots allow no symlinks;
historical v3 snapshots permit only inventoried links inside archived Sites.

## Empty-target drill

1. Use the dedicated synthetic account with multiple Topics and Chats in both
   its canonical and legacy associated Rooms, plus one encrypted attachment.
   Record identifiers in an encrypted evidence file; never put them in logs or
   this public repository.
2. Provision an empty isolated target. This is the restore boundary: the
   target Recovery Set directory must not exist or must be empty, and no target
   service may have initialized a database there. Public ingress, email, webhooks, push,
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
   Install `recovery/finite-brain/finite-brain.sqlite3` and
   `recovery/finite-identity/identity.db` into their target StateDirectories.
   For a historical v3 archive, retain `recovery/finite-sites` separately as
   recovery material, preserving symlinks without dereferencing them. Its
   restoration does not authorize restarting the retired Sites service.
6. Start Postgres, SaaS Core, Finite Identity, FiniteBrain, Finite Chat, Hosted
   Web Device and dashboard in
   isolated mode. Keep public traffic and outbound side effects off.
7. Compare Account, human identity/Nostr identity binding, Device, Room, Topic,
   Chat, message, attachment, Project, Runtime, Agent, Brain, Folder, and
   encrypted Brain object counts with the encrypted preflight evidence. Sign in
   as the restored hosted human identity, open the restored Brain through the
   normal product path, read its retained content, open all retained
   conversations, decrypt history, and download the attachment.
   Qualify current Sites recovery separately through the Sites runbook; a hosted
   snapshot no longer proves Fly data recovery.
8. Reconnect only the fenced retained Agent Runtime. Verify the durable owner
   claim replays through the canonical Room and one fresh Agent turn completes.
9. The operator performs the browser checks. Record date, archive name, component
   versions, count-only results, and pass/fail without plaintext or live ids.

Do not switch traffic as part of the drill. A production traffic switch needs
its own authorization and rollback plan.

The backup boundary is the service-consistent versioned snapshot directory after its
atomic staging rename. The restore boundary is the verified isolated staging
directory before its atomic rename into the empty target. The rollback boundary
is the untouched previous target plus the selected immutable snapshot/Borg
archive: never overwrite a previous target during this drill. To exercise the
activation boundary, run once with `FINITE_RESTORE_FAIL_AFTER_STAGE=1`; the
tool must remove its staging path, leave the target empty/unchanged, and permit
an exact retry after the failure injection is removed.

If read-only sealing itself prevents snapshot health, archival, or rotation,
unseal only the resolved latest snapshot as the mode-bit rollback:

```sh
sudo chmod -R u+w -- "$(readlink -e /data/recovery-snapshots/hosted-web-chat/latest)"
```

Do not use that rollback to inspect SQLite in place; preserve the snapshot and
use `scripts/snapshot-sqlite`. A subsequent successful snapshot run must
replace this temporary unsealed recovery point with a newly sealed one.

## Negative drill

Prove that a wrong key, truncated archive, modified artifact,
v1/v2/wrong format, mismatched `recovery-set.tsv`, missing
Chat/Core/Brain/Identity database (plus Sites for historical v3), corrupt
SQLite, unsafe
manifest path, non-empty target, and injected post-staging failure each fail
before target mutation. After any schema or snapshot-format change, repeat
both positive and negative drills.
