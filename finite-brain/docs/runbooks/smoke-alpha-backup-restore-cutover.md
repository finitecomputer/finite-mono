# Smoke Alpha Backup, Restore, And Cutover Runbook

This runbook is for the first internal FiniteBrain smoke alpha. It records the
target Rust service shape, the SQLite backup and restore path, and the
Deployment-loop handoff for replacing the old SilverBullet smoke route.

Feature Dev produces this runbook, verifier, code, and checks. Live smoke route
changes, process replacement, DNS/ingress changes, and data deletion are
Deployment-loop operations.

## Target Service

Run the Rust application server as the single shared FiniteBrain service on the
smoke box. Smoke machines and agents should point at this service rather than
running separate app instances per machine.

Recommended smoke environment:

```sh
FINITE_BRAIN_ADDR=0.0.0.0:3015
FINITE_BRAIN_PUBLIC_BASE_URL=https://brain.smoke.finite.computer
FINITE_BRAIN_SERVER_URL=https://brain.smoke.finite.computer
FINITE_BRAIN_DB=/var/lib/finitebrain/finite-brain.sqlite3
FBRAIN_CONFIG_DIR=/var/lib/finitebrain/fbrain
```

Expected routes:

```sh
curl -fsS https://brain.smoke.finite.computer/health
curl -fsS https://brain.smoke.finite.computer/client
curl -fsS https://brain.smoke.finite.computer/client/config.json
```

For agents, install `fbrain` on every smoke machine or agent runtime and use the
same HTTPS server URL:

```sh
fbrain doctor --server https://brain.smoke.finite.computer
fbrain auth status
fbrain open <vault-id> ./finitebrain-vault --server https://brain.smoke.finite.computer
```

## Current Legacy Route Evidence

The June 27, 2026 pre-cutover investigation found:

- `finite-brain-bot-finite-brain.smoke.finite.computer/client` was still
  OAuth-fronted and backed by the old SilverBullet process on port `3025`.
- `brain.smoke.finite.computer/client` returned `404`.
- No Rust `finite-brain-app` service was verified on the smoke box.
- `fbrain` was not present on the smoke host `PATH`.

Treat the old SilverBullet service as archived legacy state. The Rust Product
Client is the hard-cut route target; there is no runtime compatibility layer.

## Pre-Cutover Checklist

1. Build and stage the Rust app and CLI artifacts:

   ```sh
   cargo build --release -p finite-brain-app -p finite-brain-cli
   ```

2. Install `finite-brain-app` on the smoke box and `fbrain` on every smoke
   machine or agent runtime.

3. Create the service data directories:

   ```sh
   sudo install -d -m 0750 -o finitebrain -g finitebrain /var/lib/finitebrain
   sudo install -d -m 0750 -o finitebrain -g finitebrain /var/backups/finitebrain
   ```

4. Preserve the old SilverBullet data if it is still valuable for reference.
   This is an archive step only, not an input to the Rust service.

5. Quiesce writes before the first production smoke backup. If the service must
   stay online, use SQLite `.backup` rather than copying only the main database
   file.

6. Confirm the intended public and agent base URL is one stable HTTPS origin.
   The Product Client uses `FINITE_BRAIN_PUBLIC_BASE_URL`; `fbrain` should use
   `FINITE_BRAIN_SERVER_URL`.

## SQLite Backup

Use SQLite's backup API and an integrity check. Do not copy only
`finite-brain.sqlite3` while WAL mode may have live changes in `-wal` or `-shm`
files.

```sh
set -euo pipefail

DB="${FINITE_BRAIN_DB:-/var/lib/finitebrain/finite-brain.sqlite3}"
BACKUP_DIR=/var/backups/finitebrain
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
BACKUP="$BACKUP_DIR/finite-brain-$STAMP.sqlite3"

sudo install -d -m 0750 -o finitebrain -g finitebrain "$BACKUP_DIR"

sqlite3 "$DB" "PRAGMA wal_checkpoint(TRUNCATE);"
sqlite3 "$DB" ".backup '$BACKUP'"
sqlite3 "$BACKUP" "PRAGMA integrity_check;"
sqlite3 "$BACKUP" "SELECT name FROM sqlite_schema WHERE type='table' ORDER BY 1;"

sha256sum "$BACKUP" > "$BACKUP.sha256"
```

Expected integrity output:

```text
ok
```

Take one backup immediately before cutover and one backup immediately after the
post-cutover smoke checks pass.

## Reset Smoke Defaults

When default Getting Started or Smoke documentation changes, prefer a
non-destructive reset over deleting Vault data. Back up first, then reseed only
the deterministic Smoke fixture Pages:

```sh
set -euo pipefail

export FINITE_BRAIN_DB=/var/lib/finitebrain/finite-brain.sqlite3
export FINITE_BRAIN_SMOKE_KEYS=/var/lib/finitebrain/finite-brain-smoke-vault-keys.json
export FINITE_BRAIN_SMOKE_VAULT=smoke

node scripts/seed-smoke-doc-pages.mjs
```

The seed script replaces only the known Smoke fixture object ids and preserves
unrelated Vault records. It should be run from the same release checkout that
the Smoke service is serving so seeded Pages match the deployed Product Client
and agent conventions.

## Restore

Restore into an isolated path first when possible. For emergency service
restore, stop the service before replacing the live database file.

```sh
set -euo pipefail

DB="${FINITE_BRAIN_DB:-/var/lib/finitebrain/finite-brain.sqlite3}"
BACKUP=/var/backups/finitebrain/finite-brain-YYYYMMDDTHHMMSSZ.sqlite3
RESTORE_STAMP="$(date -u +%Y%m%dT%H%M%SZ)"

sudo systemctl stop finite-brain-app

if [ -f "$DB" ]; then
  sudo cp "$DB" "$DB.before-restore-$RESTORE_STAMP"
fi

sudo install -m 0640 -o finitebrain -g finitebrain "$BACKUP" "$DB"
sqlite3 "$DB" "PRAGMA integrity_check;"

sudo systemctl start finite-brain-app
```

After the service starts, verify the restored server before exposing it to
users.

```sh
curl -fsS https://brain.smoke.finite.computer/health
curl -fsS https://brain.smoke.finite.computer/client/config.json
fbrain doctor --server https://brain.smoke.finite.computer
```

Then verify at least one real personal Vault and one organization Vault invite
flow:

```sh
fbrain vault metadata --vault <personal-vault-id> --server https://brain.smoke.finite.computer
fbrain open <personal-vault-id> ./restore-check --server https://brain.smoke.finite.computer
(cd ./restore-check && fbrain daemon watch --once --json)
```

If current encrypted projections look wrong after restore, rebuild or verify
from the sync append log before serving traffic.

## Cutover

1. Announce a short smoke maintenance window.
2. Take the pre-cutover SQLite backup.
3. Stop or archive the old SilverBullet service. Keep its data archive if
   desired, but do not route new users to it.
4. Start the Rust `finite-brain-app` service with the target environment.
5. Point `brain.smoke.finite.computer` at the Rust service on port `3015`.
6. Confirm the legacy SilverBullet hostname no longer serves the default smoke
   FiniteBrain client route.
7. Run the post-cutover checks below.
8. Take the post-cutover SQLite backup.

Post-cutover checks:

```sh
curl -fsS https://brain.smoke.finite.computer/health
curl -fsS https://brain.smoke.finite.computer/client | rg 'obsidian-shell|FiniteBrain'
curl -fsS https://brain.smoke.finite.computer/client/app.js | rg 'vaultInvitationPanel|openFolderKeyGrants'
fbrain doctor --server https://brain.smoke.finite.computer
fbrain status --json
```

Browser checks:

- Open `/client`.
- Connect a Nostr/NIP-07 signer.
- Create or open a personal Vault.
- Open Folder Keys from encrypted grants.
- Create an organization Vault invitation by npub.
- Inspect and accept an organization Vault invitation as the invited signer.

Agent checks:

- `fbrain open` materializes a Vault Working Tree.
- Ordinary file create/edit/delete operations are visible in the working tree.
- `fbrain daemon watch --once --json` completes one sync attempt.
- `fbrain sync now --json` reports conflicts clearly or completes cleanly.

## Rollback

Rollback is route/service rollback, not SilverBullet compatibility.

Use this order:

1. Stop the Rust service or remove it from ingress.
2. Route the smoke hostname back to the previous service only if the team needs
   immediate access to the old prototype.
3. If Rust state must roll back, stop the Rust service and restore the
   pre-cutover SQLite backup using the restore section above.
4. Re-run `/health`, `/client/config.json`, `fbrain doctor`, and one real Vault
   metadata check before reopening access.

Any writes made after cutover may be absent after restoring the pre-cutover
backup. Capture a post-incident backup before destructive rollback if you need
forensics.

## Local Verification

Run the local backup verifier before handing this to Deployment:

```sh
scripts/verify-smoke-alpha-backup-restore.sh
```

The script creates a temporary SQLite database, backs it up with `.backup`,
restores it, checks integrity, compares data, and runs the store backup test
unless `SKIP_CARGO_STORE_BACKUP_TEST=1` is set.
