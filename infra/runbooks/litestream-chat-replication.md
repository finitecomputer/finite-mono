# Litestream chat replication (chat.finite.computer SQLite → Latitude object storage)

## Scope

`finite-litestream.service` on finite-lat-1 continuously replicates the chat
server's SQLite (`/var/lib/private/finite-chat/data/server.sqlite3`) to the
Latitude.sh S3-compatible bucket `finite-lat-1-litestream`
(`https://objects.nyc.storage.sh`, path-style). This is a **DR-only restore
lane** with seconds-of-writes RPO — NOT a warm standby. It is additive to the
deploy-triggered Recovery Snapshot and the nightly Borg offsite job; neither
is replaced.

**Single-writer doctrine applies** (see `deploy-finitechat-server.md`): a
restored copy must never run against production traffic while another
finitechat-server holds the same role. Restores go to a scratch path (drill)
or into a fresh StateDirectory on a replacement host (real DR) — never next
to a live server.

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
- `finitechat-server.service` is running (the replicator refuses to be the
  database's first opener — root-created WAL/SHM broke the 2026-08-11 deploy).
- The synthetic drill passes in CI: `just litestream-recovery-contract`.

## ACTIVATION (one-time; record evidence here when exercised)

1. TODO: Before the enabling closure is deployed, smoke-test the credential
   and endpoint from any machine with litestream 0.5: replicate a scratch
   SQLite into the bucket and `litestream restore` it back. This proves
   Latitude↔litestream-0.5 compatibility independent of production.
2. Deploy the enabling closure via the normal chain
   (`just nixos-build-lat1 REV` → `scripts/deploy-lat1 REV`).
3. Watch the initial snapshot upload of the ~4 GB database:
   `journalctl -fu finite-litestream`. Chat must stay uninterrupted
   (`curl -s http://127.0.0.1:8788/health`).
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
rm -rf "$out"
```

For point-in-time recovery add `-timestamp 2026-08-12T00:00:00Z` (retention:
snapshots every 24 h, 168 h kept).

TODO: exercised-on-prod evidence (date, output) goes here after activation.

## REAL DR RESTORE (lat1 lost)

1. Provision the replacement host with the lat1 closure (see
   `lat1-nixos-reinstall.md` caveats) but do NOT start `finitechat-server`.
2. Install `/etc/finite/litestream-latitude.env` from the password manager.
3. `litestream restore -o /root/server.sqlite3 <original db path>` using
   `/etc/litestream.yml` (or `-replica-url s3://finite-lat-1-litestream/finite-chat-server`
   with the endpoint env vars if the config is absent).
4. `PRAGMA integrity_check` = ok, then place the file into the service's
   fresh StateDirectory **while the service is stopped** and remove any
   root-owned `-wal`/`-shm` siblings so the DynamicUser service is the first
   opener (PR #477 lesson). Start the service; verify `/health` by direct IP
   before DNS per the deploy runbook.
5. Only after the restored server is the confirmed single writer, re-enable
   replication (it will start a new generation in the bucket).

## ROLLBACK

Remove `finite.litestream.enable` (or the module import) from
`hosts/finite-lat-1/default.nix` and redeploy. Replica data stays in the
bucket under its retention; the secret file may remain installed. Chat is
unaffected in either direction — the replicator is read-only toward the
database apart from litestream's own checkpointing.
