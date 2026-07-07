#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="$(mktemp -d "${TMPDIR:-/tmp}/finitebrain-backup-restore.XXXXXX")"

cleanup() {
  if [[ "${KEEP_SMOKE_ALPHA_BACKUP_VERIFY:-0}" != "1" ]]; then
    rm -rf "$TMPDIR"
  else
    printf 'kept backup verifier temp dir: %s\n' "$TMPDIR"
  fi
}
trap cleanup EXIT

SOURCE_DB="$TMPDIR/source.sqlite3"
BACKUP_DB="$TMPDIR/backup.sqlite3"
RESTORED_DB="$TMPDIR/restored.sqlite3"

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "sqlite3 is required for smoke alpha backup verification" >&2
  exit 1
fi

sqlite3 "$SOURCE_DB" >/dev/null <<'SQL'
PRAGMA journal_mode=WAL;
CREATE TABLE smoke_backup_probe (
  id INTEGER PRIMARY KEY,
  label TEXT NOT NULL,
  created_at TEXT NOT NULL
);
INSERT INTO smoke_backup_probe (label, created_at)
VALUES ('vaults-grants-sync-invitations', '2026-06-27T00:00:00Z'),
       ('cutover-rollback-check', '2026-06-27T00:00:01Z');
PRAGMA wal_checkpoint(TRUNCATE);
SQL

sqlite3 "$SOURCE_DB" ".backup '$BACKUP_DB'"

backup_integrity="$(sqlite3 "$BACKUP_DB" 'PRAGMA integrity_check;')"
if [[ "$backup_integrity" != "ok" ]]; then
  echo "backup integrity check failed: $backup_integrity" >&2
  exit 1
fi

cp "$BACKUP_DB" "$RESTORED_DB"

restored_integrity="$(sqlite3 "$RESTORED_DB" 'PRAGMA integrity_check;')"
if [[ "$restored_integrity" != "ok" ]]; then
  echo "restored integrity check failed: $restored_integrity" >&2
  exit 1
fi

source_rows="$(sqlite3 "$SOURCE_DB" 'SELECT count(*) FROM smoke_backup_probe;')"
restored_rows="$(sqlite3 "$RESTORED_DB" 'SELECT count(*) FROM smoke_backup_probe;')"
if [[ "$source_rows" != "$restored_rows" ]]; then
  echo "restored row count mismatch: source=$source_rows restored=$restored_rows" >&2
  exit 1
fi

expected_labels="cutover-rollback-check|vaults-grants-sync-invitations"
restored_labels="$(
  sqlite3 "$RESTORED_DB" \
    "SELECT group_concat(label, '|') FROM (SELECT label FROM smoke_backup_probe ORDER BY label);"
)"
if [[ "$restored_labels" != "$expected_labels" ]]; then
  echo "restored labels mismatch: expected=$expected_labels restored=$restored_labels" >&2
  exit 1
fi

if [[ "${SKIP_CARGO_STORE_BACKUP_TEST:-0}" != "1" ]]; then
  (
    cd "$ROOT"
    cargo test -p finite-brain-store sqlite_backup_copy_restores_append_log_and_can_rebuild_projection -- --nocapture
  )
fi

printf 'smoke alpha backup restore verifier ok\n'
