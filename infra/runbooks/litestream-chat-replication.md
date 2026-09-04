# Litestream SQLite replication (chat + Brain -> Latitude object storage)

## Scope

One `finite-litestream-<name>.service` instance per enrolled database on the
live app-plane host, finite-lat-2, continuously replicates its SQLite file to
the Latitude.sh S3-compatible bucket `finite-lat-2-litestream`
(`https://objects.chi.storage.sh`, path-style; chi measured nearest to lat1
at 29 ms vs nyc's 48 ms, 2026-08-12):

| Replica name / unit suffix | Live path | Owning service | Metrics |
| --- | --- | --- | --- |
| `finite-chat-server` | `/var/lib/private/finite-chat/data/server.sqlite3` | `finitechat-server.service` | `127.0.0.1:9351` |
| `finite-brain` | `/var/lib/private/finitebrain/finite-brain.sqlite3` | `finite-brain-app.service` | `127.0.0.1:9352` |

This is a **DR-only restore lane** with seconds-of-writes RPO — NOT a warm
standby. It is additive to the deploy-triggered Recovery Snapshot and the
nightly Borg offsite job; neither is replaced.

Both databases must be in WAL mode with the owning service as the first
opener. Brain WAL is set in `BrainStore::open`; chat already set WAL on its
store. Each replicator instance refuses to start unless its `$db-wal` exists
and is owned by the same uid as the database (root-created WAL/SHM broke the
2026-08-11 chat deploy).

Each instance is `PartOf=` its own owning service only: restarting Brain
pauses Brain replication (the health bound, 900 s, absorbs it) and leaves
chat replication running, and vice versa.

**Single-writer doctrine applies** (see `deploy-finitechat-server.md` and
`deploy-brain.md`): a restored copy must never run against production traffic
while another writer holds the same role. Restores go to a scratch path
(drill) or into a fresh StateDirectory on a replacement host (real DR) —
never next to a live server.

Config: `infra/nixos/modules/finite-litestream.nix` (module),
`infra/nixos/hosts/finite-lat-2/default.nix` (`finite.litestream.*`). Each
instance runs from its own single-db config in the Nix store;
`/etc/litestream.yml` renders the combined view of all enrolled databases
for on-host CLI work (`litestream restore` / `litestream ltx`) — never run
`litestream replicate` against it by hand, that would fight the per-db
services. Credentials:
`/etc/finite/litestream-latitude.env` (declared in
`secret-bootstrap-contract.json`; values never in the repo).

## PRECONDITIONS

- Bucket `finite-lat-2-litestream` exists at Latitude.sh with a scoped
  credential (this bucket only). Custody: regenerate at the provider; copy in
  the team password manager.
- `/etc/finite/litestream-latitude.env` installed on lat2, root:root 0600,
  containing `LITESTREAM_ACCESS_KEY_ID` and `LITESTREAM_SECRET_ACCESS_KEY`.
  `sudo scripts/check-lat1-secret-bootstrap` passes.
- `finitechat-server.service` and `finite-brain-app.service` are running
  (each replicator instance refuses to be the first opener of its database —
  root-created WAL/SHM broke the 2026-08-11 chat deploy).
- The synthetic drill passes in CI: `just litestream-recovery-contract`.

## ACTIVATION (one-time; record evidence here when exercised)

1. Smoke-test the credential and endpoint with litestream 0.5 before the
   enabling closure is deployed: replicate a scratch SQLite into the bucket
   and `litestream restore` it back. This proves Latitude↔litestream-0.5
   compatibility independent of production.
   **HISTORICAL LAT1 EXERCISE 2026-08-12**: scratch WAL db replicated to
   `s3://finite-lat-1-litestream/smoke-test` via
   `https://objects.chi.storage.sh` (path-style) with litestream 0.5.11 from
   the platform pin, restored, `PRAGMA integrity_check` = ok, content
   intact. Secret file verified root:root 0600 with both names present. The
   tiny `smoke-test/` prefix left in the bucket may be deleted from the
   dashboard at any time.
2. Deploy the enabling closure via the normal CI-built artifact chain:
   build or download `lat2-nixos-closure-REV`, then run
   `just deploy-lat2-closure ARTIFACT_DIR --prepare`, review fresh
   `scripts/finite-status` evidence, and run
   `just deploy-lat2-closure ARTIFACT_DIR --activate`.
3. Watch the initial snapshot uploads:
   `journalctl -fu finite-litestream-finite-chat-server -fu finite-litestream-finite-brain`.
   Chat (`curl -s http://127.0.0.1:8788/health`)
   and Brain (`curl -s http://127.0.0.1:3015/health`) must stay uninterrupted.
4. Run the restore-parity VERIFY below; record its date and output here.
5. `systemctl start finite-litestream-health` and confirm success plus the
   stamp at `/var/lib/finite-litestream/health-last-success`;
   `scripts/finite-status` shows the recovery boundary green with a
   `litestream` block.

## ROUTINE HEALTH

- `finite-litestream-health.timer` runs every 5 minutes: secret names
  present, every per-db daemon active, metrics served on each instance's
  loopback address (see the Scope table), and the newest replicated LTX
  entry younger than 900 s for every enrolled database. Any failure lands in
  `journalctl -u finite-litestream-health` and turns `scripts/finite-status`
  recovery red via the stale stamp.
- The pre-deploy snapshot fence stops `finitechat-server`, which stops the
  chat replicator instance (`partOf`); the chat server's start pulls it
  back. Brain replication keeps running through a chat-only fence. A deploy
  therefore shows a short per-db replication gap — the health bound (900 s)
  absorbs it.

## VERIFY (restore-parity drill — run at activation and after any migration that swaps the database file)

On lat2 (or any host with the secret and litestream 0.5):

```sh
sudo -i
set -a; . /etc/finite/litestream-latitude.env; set +a
out=/data/tmp/litestream-drill-$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p "$out"
litestream restore -config /etc/litestream.yml \
  -o "$out/server.sqlite3" /var/lib/private/finite-chat/data/server.sqlite3
sqlite3 "$out/server.sqlite3" 'PRAGMA integrity_check;'   # must print ok
# Sanity: row presence in a core table (count must be >= the value observed
# a moment earlier on the live DB via scripts/snapshot-sqlite, since the
# live database only grows):
sqlite3 "$out/server.sqlite3" 'SELECT count(*) FROM http_delivery_ops;'

litestream restore -config /etc/litestream.yml \
  -o "$out/finite-brain.sqlite3" /var/lib/private/finitebrain/finite-brain.sqlite3
sqlite3 "$out/finite-brain.sqlite3" 'PRAGMA integrity_check;'   # must print ok
sqlite3 "$out/finite-brain.sqlite3" 'SELECT count(*) FROM brains;'
rm -rf "$out"
```

For point-in-time recovery add `-timestamp 2026-08-12T00:00:00Z` (snapshots
every 24 h). NOTE 2026-08-18: remote retention enforcement is DISABLED
(`retention.enabled = false` in the per-db replicate configs) because the
Latitude credential cannot delete — its S3-compatible layer 403s
`DeleteObjects`, which made pruning permanent AccessDenied noise after the
2026-08-12 wave. Replica growth is unbounded by litestream and accepted for
now on this DR-only lane; all historical LTX files remain in the bucket, so
point-in-time restore depth is currently limited only by the 2026-08-12
replication start, not by the documented-but-inert 168 h window. Revisit if
Latitude ever offers a delete-capable credential or a safe bucket lifecycle
rule.

**HISTORICAL LAT1 EXERCISE 2026-08-13** (post-#490-deploy verification):
full restore from the chi bucket to `/data/tmp` in **35 s**,
`PRAGMA integrity_check` = ok,
`MAX(seq)` in `http_delivery_ops` identical to the live database at drill
time (147575 == 147575), blob counts identical (764 == 764). Replicator had
been running since 2026-08-12 22:25 UTC with `txid.replica == txid.db`
throughout. One defect found and fixed in the same pass: the health unit's
freshness bound false-alarmed on a quiet database (write recency ≠
replication lag) — corrected to a txid-convergence check in this branch.

## REAL DR RESTORE (app-plane host lost)

1. Provision the replacement host from an accepted replacement-host runbook,
   but do NOT start `finitechat-server` or `finite-brain-app`.
2. Install `/etc/finite/litestream-latitude.env` from the password manager.
3. Restore each enrolled database to a scratch path, then into the matching
   StateDirectory **while that service is stopped**:
   - chat: `litestream restore -o /root/server.sqlite3 /var/lib/private/finite-chat/data/server.sqlite3`
     (or `-replica-url s3://finite-lat-2-litestream/finite-chat-server`)
   - Brain: `litestream restore -o /root/finite-brain.sqlite3 /var/lib/private/finitebrain/finite-brain.sqlite3`
     (or `-replica-url s3://finite-lat-2-litestream/finite-brain`)
4. `PRAGMA integrity_check` = ok on each file, then place it into the
   service's fresh StateDirectory and remove any root-owned `-wal`/`-shm`
   siblings so the DynamicUser service is the first opener (PR #477 lesson).
   Start the service; verify `/health` by direct IP before DNS per the
   matching deploy runbook.
5. Only after each restored service is the confirmed single writer, re-enable
   replication (it will start a new generation in the bucket).

## ROLLBACK

Remove `finite.litestream.enable` (or the module import) from
`hosts/finite-lat-2/default.nix` and redeploy. Replica data stays in the
bucket under its retention; the secret file may remain installed. Chat and
Brain keep serving in either direction — the replicator is read-only toward
the databases apart from litestream's own checkpointing.
