# Litestream SQLite replication (chat + Brain → Latitude object storage)

## Scope

`finite-litestream.service` on finite-lat-1 continuously replicates enrolled
SQLite files to the Latitude.sh S3-compatible bucket `finite-lat-1-litestream`
(`https://objects.chi.storage.sh`, path-style; chi measured nearest to lat1
at 29 ms vs nyc's 48 ms, 2026-08-12):

| Replica name | Live path | Owning service |
| --- | --- | --- |
| `finite-chat-server` | `/var/lib/private/finite-chat/data/server.sqlite3` | `finitechat-server.service` |
| `finite-brain` | `/var/lib/private/finitebrain/finite-brain.sqlite3` | `finite-brain-app.service` |

This is a **DR-only restore lane** with seconds-of-writes RPO — NOT a warm
standby. It is additive to the deploy-triggered Recovery Snapshot and the
nightly Borg offsite job; neither is replaced.

Both databases must be in WAL mode with the owning service as the first
opener. Brain WAL is set in `BrainStore::open`; chat already set WAL on its
store. The replicator refuses to start unless every enrolled `$db-wal` exists
and is owned by the same uid as the database (root-created WAL/SHM broke the
2026-08-11 chat deploy).

Stopping either owning service stops the shared replicator (`PartOf=`), so a
Brain restart briefly pauses chat replication and vice versa. The health bound
(900 s) absorbs that gap.

**Single-writer doctrine applies** (see `deploy-finitechat-server.md` and
`deploy-brain.md`): a restored copy must never run against production traffic
while another writer holds the same role. Restores go to a scratch path
(drill) or into a fresh StateDirectory on a replacement host (real DR) —
never next to a live server.

Config: `infra/nixos/modules/finite-litestream.nix` (module),
`infra/nixos/hosts/finite-lat-1/default.nix` (`finite.litestream.*`),
rendered to `/etc/litestream.yml` on the host. Credentials:
`/etc/finite/litestream-latitude.env` (declared in
`secret-bootstrap-contract.json`; values never in the repo).

## PRECONDITIONS

- Bucket `finite-lat-1-litestream` exists at Latitude.sh with a scoped
  credential (this bucket only). Custody: regenerate at the provider; copy in
  the team password manager.
- `/etc/finite/litestream-latitude.env` installed on lat1, root:root 0600,
  containing `LITESTREAM_ACCESS_KEY_ID` and `LITESTREAM_SECRET_ACCESS_KEY`.
  `sudo scripts/check-lat1-secret-bootstrap` passes.
- `finitechat-server.service` and `finite-brain-app.service` are running (the
  replicator refuses to be the first opener of either database — root-created
  WAL/SHM broke the 2026-08-11 chat deploy).
- The synthetic drill passes in CI: `just litestream-recovery-contract`.

## ACTIVATION (one-time; record evidence here when exercised)

1. Smoke-test the credential and endpoint with litestream 0.5 before the
   enabling closure is deployed: replicate a scratch SQLite into the bucket
   and `litestream restore` it back. This proves Latitude↔litestream-0.5
   compatibility independent of production.
   **EXERCISED 2026-08-12** from lat1 itself: scratch WAL db replicated to
   `s3://finite-lat-1-litestream/smoke-test` via
   `https://objects.chi.storage.sh` (path-style) with litestream 0.5.11 from
   the platform pin, restored, `PRAGMA integrity_check` = ok, content
   intact. Secret file verified root:root 0600 with both names present. The
   tiny `smoke-test/` prefix left in the bucket may be deleted from the
   dashboard at any time.
2. Deploy the enabling closure via the normal chain
   (`just nixos-build-lat1 REV` → `scripts/deploy-lat1 REV`).
3. Watch the initial snapshot upload:
   `journalctl -fu finite-litestream`. Chat (`curl -s http://127.0.0.1:8788/health`)
   and Brain (`curl -s http://127.0.0.1:3015/health`) must stay uninterrupted.
4. Run the restore-parity VERIFY below; record its date and output here.
5. `systemctl start finite-litestream-health` and confirm success plus the
   stamp at `/var/lib/finite-litestream/health-last-success`;
   `scripts/finite-status` shows the recovery boundary green with a
   `litestream` block.

## ROUTINE HEALTH

- `finite-litestream-health.timer` runs every 5 minutes: secret names
  present, daemon active, metrics served on `127.0.0.1:9351`, and the newest
  replicated LTX entry younger than 900 s for every enrolled database. Any
  failure lands in `journalctl -u finite-litestream-health` and turns
  `scripts/finite-status` recovery red via the stale stamp.
- The pre-deploy snapshot fence stops `finitechat-server`, which stops the
  replicator (`partOf`); the chat server's start pulls it back. A deploy
  therefore shows a short replication gap — the health bound (900 s) absorbs
  it.

## VERIFY (restore-parity drill — run at activation and after any migration that swaps the database file)

On lat1 (or any host with the secret and litestream 0.5):

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

For point-in-time recovery add `-timestamp 2026-08-12T00:00:00Z` (retention:
snapshots every 24 h, 168 h kept).

**EXERCISED 2026-08-13** (post-#490-deploy verification): full restore from
the chi bucket to `/data/tmp` in **35 s**, `PRAGMA integrity_check` = ok,
`MAX(seq)` in `http_delivery_ops` identical to the live database at drill
time (147575 == 147575), blob counts identical (764 == 764). Replicator had
been running since 2026-08-12 22:25 UTC with `txid.replica == txid.db`
throughout. One defect found and fixed in the same pass: the health unit's
freshness bound false-alarmed on a quiet database (write recency ≠
replication lag) — corrected to a txid-convergence check in this branch.

## REAL DR RESTORE (lat1 lost)

1. Provision the replacement host with the lat1 closure (see
   `lat1-nixos-reinstall.md` caveats) but do NOT start `finitechat-server` or
   `finite-brain-app`.
2. Install `/etc/finite/litestream-latitude.env` from the password manager.
3. Restore each enrolled database to a scratch path, then into the matching
   StateDirectory **while that service is stopped**:
   - chat: `litestream restore -o /root/server.sqlite3 /var/lib/private/finite-chat/data/server.sqlite3`
     (or `-replica-url s3://finite-lat-1-litestream/finite-chat-server`)
   - Brain: `litestream restore -o /root/finite-brain.sqlite3 /var/lib/private/finitebrain/finite-brain.sqlite3`
     (or `-replica-url s3://finite-lat-1-litestream/finite-brain`)
4. `PRAGMA integrity_check` = ok on each file, then place it into the
   service's fresh StateDirectory and remove any root-owned `-wal`/`-shm`
   siblings so the DynamicUser service is the first opener (PR #477 lesson).
   Start the service; verify `/health` by direct IP before DNS per the
   matching deploy runbook.
5. Only after each restored service is the confirmed single writer, re-enable
   replication (it will start a new generation in the bucket).

## ROLLBACK

Remove `finite.litestream.enable` (or the module import) from
`hosts/finite-lat-1/default.nix` and redeploy. Replica data stays in the
bucket under its retention; the secret file may remain installed. Chat and
Brain keep serving in either direction — the replicator is read-only toward
the databases apart from litestream's own checkpointing.
