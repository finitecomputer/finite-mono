# Postgres backup + restore drill (lat1 finite_core)

**Highest-priority runbook in this tree. A restore has never been drilled.**
"Backups are only real once restored" — `infra/README.md` principle 4.

## Current reality (captured 2026-07-08)

- CronJob `finite-core-postgres-backup` (ns `finite-system`, schedule
  `17 */6 * * *`, manifest `infra/hosts/lat1/k8s/postgres.yaml`) runs
  `pg_dump --format=custom --file=/backups/finite_core_latest.dump` against
  `finite-core-postgres` (db `finite_core`, user `finite`).
- It **overwrites the same file every run**: retention is exactly one
  snapshot, at most 6h old, ~1MB at capture.
- The dump lands on PVC `finite-core-postgres-backups` — a **local-path PV
  on the same `md0` filesystem as the Postgres data PVC and the OS**
  (lat1 README appendix item 6). No timestamps, no rotation, no off-host
  copy. This is barely a backup: it survives a bad `DELETE`, not a disk.
- One ad-hoc manual dump also exists on the host:
  `/opt/finite/backups/finitecomputer-v2/pre-cleanup-20260703T0029Z.dump`.

## THE RESTORE DRILL

Run this now, then every time the schema or the backup mechanism changes.
The drill is read-only against production.

### PRECONDITIONS

- ssh to lat1 (`ssh finite-lat-1`); kubectl works on the box.
- Docker (or the devfinity local stack) on your machine for the scratch
  restore target.
- ~10 minutes and a timer — **time every step**; the timings are the drill's
  main output.

### STEPS

1. **Locate the dump on lat1.** The backups PVC is local-path, so the file
   is on the node filesystem:

   ```sh
   # on lat1
   sudo find /var/lib/rancher/k3s/storage -name finite_core_latest.dump -ls
   ```

   TODO: verify during the first drill that this is the local-path root on
   this box (k3s default) and record the exact
   `pvc-<uid>_finite-system_finite-core-postgres-backups` path here.

2. **Copy it off-box.** Either straight from the node path:

   ```sh
   # from your machine
   scp finite-lat-1:<path-from-step-1>/finite_core_latest.dump ./
   ```

   or via `kubectl cp` from a short-lived pod mounting the PVC (the CronJob
   pods are Completed, so you cannot `kubectl cp` from them directly):

   ```sh
   # on lat1
   kubectl -n finite-system apply -f - <<'EOF'
   apiVersion: v1
   kind: Pod
   metadata: {name: backup-reader, namespace: finite-system}
   spec:
     restartPolicy: Never
     containers:
       - name: reader
         image: postgres:16-alpine
         command: ["sleep", "3600"]
         volumeMounts: [{name: backups, mountPath: /backups, readOnly: true}]
     volumes:
       - name: backups
         persistentVolumeClaim: {claimName: finite-core-postgres-backups}
   EOF
   kubectl -n finite-system cp backup-reader:/backups/finite_core_latest.dump ./finite_core_latest.dump
   kubectl -n finite-system delete pod backup-reader
   ```

   TODO: validate this pod spec on the first drill (single node, RWO PVC —
   it should schedule fine; confirm).

3. **Capture live row counts** (for step 6):

   ```sh
   # on lat1
   kubectl -n finite-system exec finite-core-postgres-0 -- \
     psql -U finite -d finite_core -c \
     "SELECT relname, n_live_tup FROM pg_stat_user_tables ORDER BY relname;" \
     | tee live-rowcounts.txt
   ```

4. **Restore into a scratch postgres:16** (locally, or point at the
   devfinity stack's postgres instead):

   ```sh
   docker run -d --name pg-restore-drill -e POSTGRES_PASSWORD=drill -p 55432:5432 postgres:16-alpine
   docker cp finite_core_latest.dump pg-restore-drill:/tmp/
   docker exec pg-restore-drill createdb -U postgres finite_core
   docker exec pg-restore-drill pg_restore -U postgres --dbname=finite_core \
     --no-owner --role=postgres /tmp/finite_core_latest.dump
   ```

   TODO: first drill must confirm whether `--no-owner --role=postgres` is
   sufficient (dump was taken as role `finite`; expect owner/ACL noise —
   record the exact accepted flags and any ignorable errors here).

5. **Sanity-query the restore:**

   ```sh
   docker exec pg-restore-drill psql -U postgres -d finite_core -c \
     "SELECT relname, n_live_tup FROM pg_stat_user_tables ORDER BY relname;"
   ```

6. **Compare with step 3.** Row counts should match live to within one
   6h-window of churn (`pg_stat_user_tables` is an estimate; for exact
   comparison use `SELECT count(*)` on the 2–3 most important tables —
   TODO: name those tables here after the first drill).

7. **Record it:** total wall-clock time (copy, restore, verify), dump size,
   discrepancies, exact commands that worked. Update this file in the same
   PR.

### VERIFY

The drill passes when: `pg_restore` completes without fatal errors; row
counts are consistent with live; and the recorded end-to-end time is under
whatever we can tolerate as data-loss + recovery window (currently: up to 6h
of writes are unprotected by design — the fixes below shrink that story).

### ROLLBACK

Nothing to roll back — the drill never touches production (all lat1 steps
are reads; delete `backup-reader` if you created it). Cleanup:
`docker rm -f pg-restore-drill; rm finite_core_latest.dump live-rowcounts.txt`
(the dump contains production data — do not leave it lying around).

## Structural fixes to schedule (in priority order)

Both are edits to `infra/hosts/lat1/k8s/postgres.yaml` + `kubectl apply`;
land each as its own PR, and re-run the drill after each.

1. **Timestamped dumps + retention.** Replace the bare `pg_dump` command
   with a shell wrapper that writes
   `/backups/finite_core_<UTC-stamp>.dump`, then prunes to the newest N
   (e.g. 28 = 7 days at 6h cadence; 20Gi PVC vs ~1MB dumps leaves huge
   headroom). Sketch — TODO: validate exact command against the
   postgres:16-alpine shell during the first fix PR:

   ```yaml
   command: ["sh", "-c"]
   args:
     - pg_dump --host=finite-core-postgres --username=finite
       --dbname=finite_core --format=custom
       --file="/backups/finite_core_$(date -u +%Y%m%dT%H%M%SZ).dump"
       && ls -1t /backups/finite_core_*.dump | tail -n +29 | xargs -r rm
   ```

   Keep writing (or symlinking) `finite_core_latest.dump` only if something
   consumes that name — TODO: confirm nothing does before dropping it.

2. **Off-host copy.** The dumps must leave lat1's md0. Candidate targets:
   lat2 `/data` (1.8T, empty, earmarked for backups —
   `infra/hosts/lat2/backups.md`) via scp/rsync from a host systemd timer,
   or Latitude object storage (the restic/S3 machinery in
   `hermes-runtime-smoke.yml` shows the org already has it). TODO: decide
   target and mechanism at the first-fix review; the dump contains
   production data — encrypt or restrict perms in transit and at rest
   (same caution as lat2's backup tarballs).
