# Postgres backup + restore drill (lat1 finite_core)

**Highest-priority runbook in this tree. A restore has never been drilled.**
"Backups are only real once restored" — `infra/README.md` deploy principle 5.

## Current reality (post 2026-07-09 cutover)

Postgres 16 is now a **native systemd service** on finite-lat-1
(`services.postgresql`, `infra/nixos/modules/postgres.nix`) — NOT a k3s
StatefulSet/pod anymore. Db `finite_core`, role `finite`, listening
127.0.0.1:5432; Core reaches it via `FC_CORE_DATABASE_URL`. The invariant to
protect: **87 rows in `finite_private_api_keys`** (Finite Private keys,
restored at cutover).

Backups: systemd service+timer `finite-postgres-backup` writes **timestamped**
custom-format dumps to `/data/backups/postgres/finite_core_<UTC-stamp>.dump`
on the `OnCalendar=*-*-* 00/6:17:00` cadence (every 6h at :17), with local
retention (see the module). `/data/backups/postgres` is 0750 postgres:postgres.

> **REDUNDANCY GAP (top follow-up):** the coordinated Hosted Web Chat snapshot
> now includes a fresh custom-format `finite_core` dump and its Nix definition
> selects a dedicated repository at the existing rsync.net destination. The
> 2026-07-18 live inventory observed healthy offsite jobs and a verified first
> archive, but the complete empty-target restore proof is not complete. lat1
> root and `/data` are both **single-disk, no mdadm**
> ([lat1-nixos-reinstall.md](lat1-nixos-reinstall.md)): the live database is on
> root and the six-hourly local dumps are on the separate `/data` NVMe. They do
> not share one disk, but they do share one chassis and provider failure
> domain. The off-host archive is necessary; only an independent restore makes
> it recovery evidence.

## THE RESTORE DRILL

Run this now, then every time the schema or the backup mechanism changes.
The drill is read-only against production.

### PRECONDITIONS

- ssh to lat1 (`ssh root@64.34.82.77`, key-only).
- A scratch postgres:16 target for the restore — Docker (or the devfinity
  local stack) on your machine.
- ~10 minutes and a timer — **time every step**; the timings are the drill's
  main output.

### STEPS

1. **Locate the newest dump on lat1** (they are timestamped, so pick the
   latest):

   ```sh
   ssh root@finite-lat-1 'ls -1t /data/backups/postgres/finite_core_*.dump | head -1'
   ```

2. **Capture live row counts** (for step 5), reading the native db as the
   postgres user:

   ```sh
   ssh root@finite-lat-1 "sudo -u postgres psql -d finite_core -c \
     'SELECT relname, n_live_tup FROM pg_stat_user_tables ORDER BY relname;'" \
     | tee live-rowcounts.txt
   # and the invariant:
   ssh root@finite-lat-1 "sudo -u postgres psql -d finite_core -tAc \
     'SELECT count(*) FROM finite_private_api_keys;'"   # expect 87
   ```

3. **Copy the dump off-box:**

   ```sh
   scp finite-lat-1:/data/backups/postgres/<dump-from-step-1> ./
   ```

4. **Restore into a scratch postgres:16** (locally, or point at the devfinity
   stack instead):

   ```sh
   docker run -d --name pg-restore-drill -e POSTGRES_PASSWORD=drill -p 55432:5432 postgres:16-alpine
   docker cp <dump-from-step-1> pg-restore-drill:/tmp/dump
   docker exec pg-restore-drill createdb -U postgres finite_core
   docker exec pg-restore-drill pg_restore -U postgres --dbname=finite_core \
     --no-owner --role=postgres /tmp/dump
   ```

   (The dump was taken as role `finite`; `--no-owner --role=postgres` remaps
   ownership to the scratch superuser. Expect ignorable owner/ACL notices —
   record the exact accepted flags on the first drill.)

5. **Sanity-query and compare with step 2:**

   ```sh
   docker exec pg-restore-drill psql -U postgres -d finite_core -c \
     "SELECT relname, n_live_tup FROM pg_stat_user_tables ORDER BY relname;"
   docker exec pg-restore-drill psql -U postgres -d finite_core -tAc \
     "SELECT count(*) FROM finite_private_api_keys;"   # expect 87
   ```

   Row counts should match live to within one 6h window of churn
   (`pg_stat_user_tables` is an estimate; `finite_private_api_keys` should be
   exactly 87).

6. **Record it:** total wall-clock (copy, restore, verify), dump size,
   discrepancies, exact commands that worked. Update this file in the same PR.

### VERIFY

The drill passes when: `pg_restore` completes without fatal errors; row
counts are consistent with live; `finite_private_api_keys` = 87; and the
recorded end-to-end time is within the tolerable data-loss + recovery window
(the standalone local dump cadence is up to ~6h; the separately monitored
coordinated off-host Recovery Set has an accepted 15-minute target but is
currently deploy/manual-triggered). The latest verified off-host archive
provides a disk-loss recovery point, but no full recovery claim exists until
the non-disruptive cadence and independent empty-target drill pass.

### ROLLBACK

Nothing to roll back — the drill never touches production (all lat1 steps are
reads). Cleanup:
`docker rm -f pg-restore-drill; rm <dump> live-rowcounts.txt`
(the dump contains production data — do not leave it lying around).

## Restoring INTO production lat1 (real recovery)

For an actual restore onto the native database after a separately authorized
target has been built, or when recovering from a bad migration, use the steps
below. The [2026-07-09 bare-metal transcript](lat1-nixos-reinstall.md) is
historical evidence, not authority to wipe/rebuild the target:

1. Bootstrap the role/db ownership before the restore (the db exists from
   `services.postgresql` but the role password + ownership come from the old
   secret; header in `modules/postgres.nix`):

   ```sh
   sudo -u postgres psql -c "ALTER ROLE finite WITH PASSWORD '<POSTGRES_PASSWORD>';"
   sudo -u postgres psql -c "ALTER DATABASE finite_core OWNER TO finite;"
   ```

   (`<POSTGRES_PASSWORD>` by name only — it must match the one embedded in
   `FC_CORE_DATABASE_URL` in `/etc/finite/core.env`. No secret values in git.)

2. Restore, running as the postgres user from a path postgres can read
   (NOT `/root`):

   ```sh
   sudo cp <dump> /tmp/finite_core.dump && sudo chown postgres /tmp/finite_core.dump
   sudo -u postgres pg_restore -d finite_core --no-owner --role=finite \
     --clean --if-exists /tmp/finite_core.dump
   ```

3. **Verify the invariant:**
   `sudo -u postgres psql -d finite_core -tAc 'SELECT count(*) FROM finite_private_api_keys;'`
   → 87. Then restart Core (`systemctl restart finite-saas-core`) and hit
   `/healthz` on :4200.

## Structural fixes to schedule (in priority order)

1. **Maintain and restore-prove offsite Borg (do this first).** The
   coordinated recovery snapshot uses `pg_dump`, SQLite backup APIs, and a
   brief writer fence; `modules/backups.nix` archives that artifact to the
   configured rsync.net repository. Its first archive and offsite-health jobs
   passed the 2026-07-18 live inventory. Preserve that path and complete the
   independent empty-target drill. Brain, Sites, and Agent `/data` need
   separately declared recovery sets; do not imply they are covered here.
2. **Disk mirror.** The matching lat1 NVMes contain stale metadata from the
   failed 2026-07-09 MD install; they are not free spares. Reuse only the
   storage design proved on lat3, after a fresh serial-stable lat1 inventory,
   exact-geometry synthetic test, recovery proof, and separate destructive
   authorization. Backups remain the safety net until this lands.
