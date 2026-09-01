# Data recovery & repair

Consolidated 2026-08-29 (essentials task 10) from the Postgres backup/restore
drill, the Hosted Web Chat snapshot/empty-target contract, the litestream
replication lane, the catastrophic-custody model, the exact cold-relocation
transaction, and the Identity Authority restore section.

[ADR 0001](../../docs/adr/0001-recoverability-precedes-operator-blindness.md)
governs everything here: **recoverability precedes operator blindness.** Do
not remove a Recovery Authority, couple compute teardown to data purge, or
claim stronger operator-blindness until the same Recovery Set has restored
onto an empty target. A TEE and a provider durable volume are not backups;
a green archive is not a proved restore. Fleet destinations are dated
2026-08-29 (post-ADR 0007): the Recovery Authority host is finite-lat-2;
finite-lat-1's archives are frozen point-in-time records.

## 1. Custody model

Three independent layers:

1. Rebuildable NixOS configuration and public recovery tooling in
   `finite-mono`.
2. The encrypted rsync.net Borg repositories for component Recovery Sets,
   Agent retirement archives, and the secret bootstrap roots
   (lat2's authority repository:
   `fm2890@fm2890.rsync.net:finitecomputer/finite-lat-2`; lat1's
   repositories are preserved untouched and never written again).
3. A separately encrypted, operator-custodied Borg repository on removable
   media or another operator-controlled destination, populated during a
   separately approved writer fence.

Do not use raw device images as the primary catastrophic copy: they copy
unused blocks and stale RAID metadata, are awkward to verify, and do not
establish application consistency. A file-level Borg copy is smaller,
inspectable, deduplicated, and restorable into an empty target. The operator
copy should contain, in full:

```text
/data/finite-saas-runner        (lat3/lat4 durable Kata, retirement, rollback, and upgrade-snapshot state)
/var/lib/finite-saas-runner     (runner service working state; not the durable Kata volumes)
/var/lib/finite-sites
/var/lib/private/finitechat-hosted-device
/var/lib/private/finite-chat
/var/lib/private/finitebrain
/var/lib/private/finite-identity
/var/lib/private/finite-saas-core
/data/recovery-snapshots  and  /data/backups
/etc/finite  and  /etc/finite-saas
/var/lib/finitecomputer/backups/rsync-net
```

The component snapshot/restore tools remain the preferred recovery source
for SQLite and Postgres; raw live database files in the catastrophic copy
are defense in depth only.

**Bootstrap root of trust** — accessing a remote Borg repository requires
all four of: the exact repository endpoint; an independently held SSH
credential and pinned `known_hosts`; the Borg passphrase; and the exported
Borg repokey. Archiving a credential inside the repository it protects is
circular by itself. Store the transport credential, pinned host identity,
and Borg recovery material in an encrypted operator-controlled location
with an independent unlock path. Never commit values, fingerprints, or
password-derived hashes.

**Authorization boundaries** — each of these requires separate explicit
approval: reading and transferring the rsync.net SSH private key; choosing
and writing the operator-custodied destination; stopping writers and Agent
compute; running the coordinated production snapshot and remote archive;
downloading the catastrophic copy; deleting any container metadata,
rollback directory, durable state, or source archive.

## 2. Recovery Sets and their authorities

- **The v3 coordinated snapshot**
  (`finite.hosted-web-chat-recovery-snapshot.v3`). Complete set: the
  Hosted Web Device identity, encrypted client stores, and Agent bindings;
  the complete Finite Chat server SQLite; a custom-format SaaS Core Postgres
  dump; the FiniteBrain SQLite; the Finite Identity SQLite; and the complete
  finite-sites data directory with its registry captured through SQLite's
  backup API. The separately retained Agent Runtime `/data` is NOT in this
  snapshot — a named gap. v1/v2 snapshots are incomplete and the restore
  tool rejects them. The snapshot unit briefly fences every writer in the
  set, uses SQLite's backup API per database, `pg_dump --format=custom` for
  Core, verifies each artifact, and writes relative paths + hashes to the
  integrity manifest; `recovery-set.tsv` binds format version to the six
  component identities. Sites symlinks are preserved without dereferencing
  and bound to `finite-sites-symlinks.bin`; links are allowed nowhere else
  (preserves app environments with dangling build-cache links without
  treating external targets as backed up). Cadence: deploy/manual-triggered
  (the disruptive 15-minute timer was removed); snapshot health allows
  seven days, Borg re-ships the latest daily and offsite health allows 50
  hours. This is not the accepted 15-minute RPO.
- **Postgres dumps.** The `finite-postgres-backup` service+timer writes
  timestamped custom-format dumps every 6 h (at :17) to
  `/data/backups/postgres/finite_core_<UTC-stamp>.dump` with local
  retention; the coordinated snapshot carries a fresh dump off-host.
  Invariant to protect: **87 rows in `finite_private_api_keys`**.
- **Litestream (DR-only).** One `finite-litestream-<name>.service` per
  enrolled database (chat, brain) continuously replicates to the Latitude
  object-storage bucket — seconds-of-writes RPO, NOT a warm standby,
  additive to the snapshot + Borg lanes. lat2 replicates to the
  `finite-lat-2-litestream` bucket; lat1's `finite-lat-1-litestream` is
  frozen (the lat2 import restored FROM it). Each replicator refuses to be
  the first opener of its database (root-created WAL/SHM broke a chat
  deploy once) and is `PartOf=` its owning service only. Health:
  `finite-litestream-health.timer` every 5 min (txid-convergence bound
  900 s); failures turn `scripts/finite-status` recovery red. Remote
  retention enforcement is disabled (the Latitude credential cannot delete;
  `DeleteObjects` 403s), so replica growth is unbounded and point-in-time
  depth is limited only by the replication start date.
- **Frozen legacy archives.** lat1's litestream bucket and Borg
  repositories are point-in-time records of the 2026-08-27 outage; nothing
  ever writes them again.
- **Agent Runtime state.** `/data` per Runtime plus the runner work root
  are outside the v3 snapshot. Coverage today is the banked lat1 runner
  state (imported to lat4 through the exact relocation contract, §5),
  per-retirement restricted Borg recovery sets, and the recovery archives
  named in the runner host's install record.

## 3. Drills — the proof

Backups are only real once restored. **Current honest status (2026-08-29):
the coordinated Hosted Web Chat and Postgres empty-target drills have NOT
passed** — the off-host repositories and a verified first archive exist; do
not regress their health or mistake them for a completed drill. Repeat
every drill after any schema or backup-mechanism change.

### Postgres empty-target restore drill (read-only against production)

Time every step — the timings are the drill's main output.

1. Locate the newest dump on the Recovery Authority host:

   ```sh
   ssh root@finite-lat-2 'ls -1t /data/backups/postgres/finite_core_*.dump | head -1'
   ```

2. Capture live row counts for comparison:

   ```sh
   ssh root@finite-lat-2 "sudo -u postgres psql -d finite_core -c \
     'SELECT relname, n_live_tup FROM pg_stat_user_tables ORDER BY relname;'" \
     | tee live-rowcounts.txt
   ssh root@finite-lat-2 "sudo -u postgres psql -d finite_core -tAc \
     'SELECT count(*) FROM finite_private_api_keys;'"   # expect 87
   ```

3. Copy the dump off-box: `scp finite-lat-2:/data/backups/postgres/<dump> ./`
4. Restore into a scratch postgres:16 (locally or the devfinity stack):

   ```sh
   docker run -d --name pg-restore-drill -e POSTGRES_PASSWORD=drill -p 55432:5432 postgres:16-alpine
   docker cp <dump> pg-restore-drill:/tmp/dump
   docker exec pg-restore-drill createdb -U postgres finite_core
   docker exec pg-restore-drill pg_restore -U postgres --dbname=finite_core \
     --no-owner --role=postgres /tmp/dump
   ```

   (Dump taken as role `finite`; `--no-owner --role=postgres` remaps
   ownership to the scratch superuser; ignorable owner/ACL notices
   expected.)

5. Re-run the step-2 queries against the scratch target. Row counts should
   match live to within one 6 h window of churn; `finite_private_api_keys`
   exactly 87.
6. Record total wall-clock, dump size, discrepancies, and the exact
   accepted flags.

Cleanup: `docker rm -f pg-restore-drill; rm <dump> live-rowcounts.txt` —
the dump contains production data; do not leave it lying around.
Automating this drill as a scheduled job is a road-to-zero item; until
then it is human-run by necessity.

### Hosted Web Chat empty-target drill

1. Use the dedicated synthetic account with multiple Topics and Chats in
   both its canonical and legacy associated Rooms, plus one encrypted
   attachment. Record identifiers in an encrypted evidence file — never in
   logs or this public repository.
2. Provision an empty isolated target. This is the restore boundary: the
   target Recovery Set directory must not exist or must be empty, no target
   service may have initialized a database there, and public ingress,
   email, webhooks, push, billing jobs, and other outbound side effects
   stay disabled. Fence the separately retained Agent Runtime so it cannot
   contact both stacks.
3. Extract one Borg archive into a temporary directory outside the target.
   A missing/wrong passphrase or failed extraction stops here and must
   leave the target untouched.
4. Run the verifier/atomic artifact restore:

   ```sh
   FINITE_RESTORE_ISOLATED=1 \
     infra/scripts/restore-hosted-web-chat-snapshot EXTRACTED_SNAPSHOT EMPTY_TARGET/recovery
   ```

   It rejects missing, partial, corrupt, unsupported, or non-empty-target
   restores before installing artifacts.
5. With the target services stopped, install `recovery/hosted-device` as
   the Hosted Web Device StateDirectory and
   `recovery/finite-chat/server.sqlite3` as the chat database (preserve
   ownership/mode from the target's Nix units). Create an empty
   `finite_core`, then `pg_restore --exit-on-error --single-transaction
   --clean --if-exists`. Install `recovery/finite-brain/
   finite-brain.sqlite3` and `recovery/finite-identity/identity.db` into
   their StateDirectories. Install the complete `recovery/finite-sites`
   directory as the target `/var/lib/finite-sites` without dereferencing
   its symlinks — a restored dangling app-environment link is not evidence
   its target was recovered.
6. Start Postgres, SaaS Core, Finite Identity, FiniteBrain, Finite Chat,
   Hosted Web Device, finite-sites, and dashboard in isolated mode; keep
   public traffic and outbound side effects off.
7. Compare Account, identity binding, Device, Room, Topic, Chat, message,
   attachment, Project, Runtime, Agent, Brain, Folder, and encrypted Brain
   object counts with the encrypted preflight evidence. Sign in as the
   restored hosted human identity, open the restored Brain through the
   normal product path, read retained content, open all retained
   conversations, decrypt history, download the attachment. Compare Sites
   registry rows and published version identities; load the synthetic
   site's HTML and one blob-backed asset through the isolated target.
8. Reconnect only the fenced retained Agent Runtime; verify the durable
   owner claim replays through the canonical Room and one fresh Agent turn
   completes.
9. The owner performs the browser checks. Record date, archive name,
   component versions, count-only results, and pass/fail — no plaintext,
   no live ids.

Do not switch traffic as part of the drill; a production traffic switch
needs its own authorization and rollback plan. Boundaries: backup = the
service-consistent v3 snapshot directory after its atomic staging rename;
restore = the verified isolated staging directory before its atomic rename
into the empty target; rollback = the untouched previous target plus the
selected immutable snapshot/Borg archive (never overwrite a previous
target). To exercise the activation boundary, run once with
`FINITE_RESTORE_FAIL_AFTER_STAGE=1` — the tool must remove its staging
path, leave the target unchanged, and permit an exact retry.

**Negative drill:** before admission, prove that a wrong key, truncated
archive, modified artifact, v1/v2/wrong format, mismatched
`recovery-set.tsv`, missing Chat/Core/Brain/Identity/Sites database,
corrupt Sites registry, unsafe manifest path, non-empty target, and
injected post-staging failure each fail BEFORE target mutation.

**Snapshot checks (source host):**

```sh
systemctl status borgbackup-job-finite-hosted-web-chat-offsite.timer
systemctl status finite-hosted-web-chat-snapshot-health.service
systemctl status finite-hosted-web-chat-offsite-health.service
journalctl -u finite-hosted-web-chat-snapshot -u borgbackup-job-finite-hosted-web-chat-offsite -u finite-hosted-web-chat-offsite-health
latest=/data/recovery-snapshots/hosted-web-chat/latest
age=$(( $(date +%s) - $(stat -Lc %Y "$latest") ))
test "$age" -le 604800      # current health threshold only
test "$age" -le 900         # separate admission/RPO gate; expected to fail today
(cd "$latest" && sha256sum --check manifest.sha256)
test "$(cat "$latest/format")" = finite.hosted-web-chat-recovery-snapshot.v3
test -f "$latest/recovery-set.tsv"
scripts/snapshot-sqlite integrity-check "$latest/finite-sites/registry.db"
```

Never pass a database below `$latest` to plain `sqlite3` — the helper
copies the database and any WAL/SHM sidecars to private scratch space
first. If read-only sealing itself prevents snapshot health, archival, or
rotation, unseal only the resolved latest snapshot
(`sudo chmod -R u+w -- "$(readlink -e
/data/recovery-snapshots/hosted-web-chat/latest)"`) as the mode-bit
rollback; do not use it to inspect SQLite in place, and a subsequent
successful snapshot run must replace the unsealed recovery point.

### Litestream restore-parity drill

Run at activation and after any migration that swaps a database file, on a
host with the secret and litestream 0.5:

```sh
sudo -i
set -a; . /etc/finite/litestream-latitude.env; set +a
out=/data/tmp/litestream-drill-$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p "$out"
litestream restore -config /etc/litestream.yml \
  -o "$out/server.sqlite3" /var/lib/private/finite-chat/data/server.sqlite3
sqlite3 "$out/server.sqlite3" 'PRAGMA integrity_check;'   # must print ok
sqlite3 "$out/server.sqlite3" 'SELECT count(*) FROM http_delivery_ops;'

litestream restore -config /etc/litestream.yml \
  -o "$out/finite-brain.sqlite3" /var/lib/private/finitebrain/finite-brain.sqlite3
sqlite3 "$out/finite-brain.sqlite3" 'PRAGMA integrity_check;'   # must print ok
sqlite3 "$out/finite-brain.sqlite3" 'SELECT count(*) FROM brains;'
rm -rf "$out"
```

Row counts must be >= values observed a moment earlier on the live DB via
`scripts/snapshot-sqlite` (the live database only grows). Add `-timestamp
…Z` for point-in-time recovery (24 h snapshots). Exercised 2026-08-13:
full restore in 35 s, integrity ok, identical MAX(seq) and blob counts,
`txid.replica == txid.db` throughout.

### Brain restore drill

The fbrain-side drill and its automated proof
(`built_fbrain_process_brain_restore_drill`) live with the brain crate:
[`finite-brain/docs/runbooks/brain-restore-drill.md`](../../finite-brain/docs/runbooks/brain-restore-drill.md).

## 4. Restoring into production

AGENTS.md production-repair rules are preconditions, not advice: gather
read-only evidence, reproduce the failure, prove the repair on synthetic
state, name the backup and rollback boundary, and obtain explicit user
authorization before any production mutation. Fence every writer you are
about to replace (single-writer doctrine — [incident.md](incident.md) §4);
never run a restored copy beside its live writer.

- **Postgres.** Bootstrap role/db ownership before the restore (the db
  exists from `services.postgresql`; the role password + ownership come
  from the old secret — it must match `FC_CORE_DATABASE_URL` in
  `/etc/finite/core.env`; values by name only):

  ```sh
  sudo -u postgres psql -c "ALTER ROLE finite WITH PASSWORD '<POSTGRES_PASSWORD>';"
  sudo -u postgres psql -c "ALTER DATABASE finite_core OWNER TO finite;"
  ```

  Restore running as the postgres user from a path postgres can read (NOT
  `/root`):

  ```sh
  sudo cp <dump> /tmp/finite_core.dump && sudo chown postgres /tmp/finite_core.dump
  sudo -u postgres pg_restore -d finite_core --no-owner --role=finite \
    --clean --if-exists /tmp/finite_core.dump
  ```

  Verify the invariant (87 keys:
  `sudo -u postgres psql -d finite_core -tAc 'SELECT count(*) FROM
  finite_private_api_keys;'`), then restart Core and hit `/healthz`.
- **Chat / Brain SQLite.** Stop the owning service first. Restore the file
  into the service's StateDirectory with the ownership the DynamicUser
  assigns, and remove any root-owned `-wal`/`-shm` siblings so the service
  is the first opener. Verify `PRAGMA integrity_check` (on a copy — never
  open a snapshot database directly) and by direct IP before DNS. Only
  after the restored service is the confirmed single writer, re-enable
  litestream replication (it starts a new generation in the bucket).
- **Identity (`identity.db`).** `finite-identity-backup.timer` takes an
  online SQLite backup every six hours (14-day local retention, included
  in the daily off-host Borg job; latest backup < 7 h old with recorded
  SHA-256). The coordinated snapshot also fences the Authority and copies
  the database through SQLite's backup API. Inspect before restoration
  only through the snapshot helper
  (`scripts/snapshot-sqlite integrity-check …/finite-identity/identity.db`;
  expected output exactly `ok`). Authorized production restore order:
  1. stop both Runner workers and their timer, then stop
     `finite-identity.service`;
  2. preserve the current `/var/lib/finite-identity` as the named data
     rollback boundary;
  3. restore `identity.db` as the service StateDirectory owner, mode 0600;
  4. start the Authority, verify loopback health and representative public
     resolution;
  5. start Core, then the Runner workers/timer.

  Restoring the database restores public binding state but cannot restore
  Agent Local Identity Keys — Agent `/data` recovery remains the authority
  for those. The operator token is replaceable configuration, not identity
  data: after host loss, generate a new value and install the same value
  for the Authority and trusted products before starting either. Immutable
  email/key conflicts require investigation, not state rewrite — do not
  replace the database with an older snapshot merely to make a launch
  retry.

### 4a. Chat quarantine repair (frozen room cursor on a rejected entry)

*(Folded from the former `chat-quarantine-repair.md`, 2026-09-01.)*

When a room sync tick hits an entry the device cannot apply (typically an MLS
application ciphertext from another member it cannot decrypt), the failure is
quarantined silently: the room lands in `room_sync_failures`, the durable
cursor freezes, and — before image `2026-08-29.5` — the server's
`RoomAdvanced` hint made the sidecar refetch the same rejected page at
network speed forever (the #776 livelock). The image fix stops the refetch
storm; it does not move the cursor. This runbook is the proven per-agent
state repair (exercised 2026-08-29 on five agents and re-proved against the
current lat4 layout on 2026-09-01 — see
[`docs/runs/2026-08-29-chat-plane-freeze.md`](../../docs/runs/2026-08-29-chat-plane-freeze.md)).

**`finitechat repair skip-entry` is the ONLY production-sanctioned way to
advance a durable room cursor past a rejected entry.** It never accepts an
operator-typed sequence: it derives the skip list from a replay of the
captured log, rehearses against byte copies, and applies to the real store
only when the rehearsal cleanly reaches the capture head. Never hand-edit
the SQLite store. A refusal is a STOP, not a retry-harder.

### Identify (read-only first)

1. **Refetch signature.** On lat2, the chat vhost's Caddy access logs
   (`/var/log/caddy/access-chat.finite.computer*.log`) show a sustained
   high rate of `POST /sync/group` from a runner host's IP — during the
   incident, 13–25 requests/s per affected agent, all returning 200 (every
   individual fetch succeeds; that is why request-bound alarms never fire).
   On images `≥ 2026-08-29.5` the sidecar also emits a single-line stderr
   quarantine report (room id, `rejected_after_seq`, error class) and
   `/readyz` gains `runtime_status` ("ready; N rooms need repair").
2. **Cursor vs head.** Compare the room's durable cursor against the
   server-side `last_seq`. A permanently-behind cursor with a growing server
   head is the quarantine signature.
3. **Gotcha: the cursor is not the events table.** Read the cursor from the
   diagnostic record — `client_app_events` max-seq is NOT the durable cursor
   (they diverged by 10 on one agent during the incident), and an agent
   store's head is not a server-side number. Label every seq you write down
   with its owner.

### PRECONDITIONS

- The agent's room is identified (room id + the frozen cursor seq) and the
  cause is understood as undecryptable application ciphertext, not an outage.
- You are on the runner host that owns the Agent's Kata volume
  (lat3 or lat4), with `sudo` for `nerdctl --namespace finite` and read
  access to `/data/finite-saas-runner/kata/<runtime-id>/`.
- You have resolved the existing container, exact runtime image digest, Kata
  runtime, and `/data` mount source from `nerdctl inspect`. Do not infer a
  host path from the container name: the durable directory is keyed by the
  Core runtime id while the container name is keyed separately.
- `finitechat capture room-log` works read-only against
  `https://chat.finite.computer`; nothing below mutates the server.

### STEPS (per agent; prove each stage before the next)

#### 1. Resolve and stop the existing container

Record the exact live topology before stopping the writer. `KATA` must be the
host source mounted at `/data`, not a guessed path:

```sh
CONTAINER=<existing-container-id-or-name>
IMAGE=$(sudo nerdctl --namespace finite inspect "$CONTAINER" --format '{{.Config.Image}}')
RUNTIME=$(sudo nerdctl --namespace finite inspect "$CONTAINER" --format '{{.HostConfig.Runtime}}')
KATA=$(sudo nerdctl --namespace finite inspect "$CONTAINER" \
  --format '{{range .Mounts}}{{println .Source .Destination}}{{end}}' |
  awk '$2 == "/data" { print $1 }')
test -n "$IMAGE" && test -n "$RUNTIME" && test -d "$KATA/agent"
sudo nerdctl --namespace finite stop --time 30 "$CONTAINER"
test "$(sudo nerdctl --namespace finite inspect "$CONTAINER" --format '{{.State.Status}}')" = exited
```

Kata cleanup may emit a timeout warning while finishing in the background.
Do not proceed until the container is `exited`, its QEMU sandbox process is
gone, and its published contact port is no longer listening.

#### 2. Host-side byte-copy backup

With the writer quiesced, back up the store AND its WAL sibling outside the
writable Kata volume and record checksums. A sequential copy while the writer
is live is not a consistent SQLite rollback boundary.

```sh
TS=$(date -u +%Y%m%dT%H%M%SZ)
BACKUP=/data/finite-saas-runner/repair-backups/<runtime-id>/$TS
sudo install -d -m 0700 "$BACKUP"
sudo cp -p "$KATA/agent/client.sqlite3" "$BACKUP/client.sqlite3"
sudo cp -p "$KATA/agent/client.sqlite3-wal" "$BACKUP/client.sqlite3-wal"
sudo sha256sum "$BACKUP/client.sqlite3" "$BACKUP/client.sqlite3-wal" |
  sudo tee "$BACKUP/SHA256SUMS"
sudo sha256sum -c "$BACKUP/SHA256SUMS"
```

Retain the backups with the audit trail; the rollback boundary is exactly
this checksummed pair. Keep the container stopped through diagnosis, rehearsal,
and apply.

#### 3. Capture + diagnose (read-only; inside a one-shot container)

Run the tooling where the store and the identity live — a one-shot container
from the Agent's exact pinned image and Kata runtime, with the Kata volume at
`/data` and the rollback pair mounted read-only at `/rollback` (CLI at
`/runtime/bin/finitechat`, store at `/data/agent/client.sqlite3`, account
secret inside `/data/agent/identity/identity.json`). Secrets stay in-VM
mode-600 files: extract, use, delete; never typed on the host, never echoed
into logs. Override the image entrypoint for every one-shot; otherwise the
normal Agent Runtime supervisors start alongside the operator command.

From the host, start an interactive operator shell. Keep this shell open
through capture, rehearsal, and apply so that all secret handling remains
inside the Kata VM:

```sh
sudo nerdctl --namespace finite run --rm -it \
  --runtime "$RUNTIME" \
  --network bridge \
  --entrypoint /bin/sh \
  -v "$KATA:/data" \
  -v "$BACKUP:/rollback:ro" \
  "$IMAGE"
```

Run the remaining commands in that shell:

```sh
W=/data/repair-<alias>
mkdir -p "$W"
cd "$W"
# Secret to file (never echoed); call once per stage and delete after use.
write_secret() {
  python3 -c "import json; open('$W/secret.hex','w').write(json.load(open('/data/agent/identity/identity.json'))['secret_hex'].strip())"
  chmod 600 "$W/secret.hex"
}
write_secret
# byte copy of the stopped-store rollback pair for diagnosis
cp /rollback/client.sqlite3 "$W/store-copy.sqlite3"
cp /rollback/client.sqlite3-wal "$W/store-copy.sqlite3-wal"
# page the unapplied range off the server (read-only), starting at the CURSOR
/runtime/bin/finitechat capture room-log \
  --server https://chat.finite.computer \
  --room-id <ROOM-ID> \
  --device-id agent \
  --account-secret-file "$W/secret.hex" \
  --out "$W/room-log.json" \
  --after-seq <CURSOR>
# replay against the COPY; emits the privacy-locked classification record
SECRET=$(cat "$W/secret.hex")
/runtime/bin/finitechat diagnose rejected-entry \
  --store "$W/store-copy.sqlite3" \
  --work-dir "$W/diag-work" \
  --room-log "$W/room-log.json" \
  --device-id agent \
  --account-secret-hex "$SECRET" \
  --incident-alias <alias>
rm -f "$W/secret.hex"
```

**STOP unless every candidate skip classifies as `kind=application` with
`error_class=mls_application_ciphertext`.** Any other kind or error class is
an unexplained failure — capture the record and escalate; do not repair.
The tooling enforces the same rule and will refuse, but read the record
yourself first. `diagnose rejected-entry` reports the first rejection; the
next copy-only rehearsal proves and displays the complete derived skip list.

#### 4. Full copy-only rehearsal

Copy the rollback pair to a disposable trial store and run the repair there
first. This proves that every later rejection has the same allowed class and
that valid entries between rejected entries replay normally. Inspect the
complete `skipped` list and require `rehearsal_outcome=advanced` with
`cursor_after` equal to the capture head. Any refusal or different class is a
STOP.

```sh
cp /rollback/client.sqlite3 "$W/trial-store.sqlite3"
cp /rollback/client.sqlite3-wal "$W/trial-store.sqlite3-wal"
write_secret
SECRET=$(cat "$W/secret.hex")
/runtime/bin/finitechat repair skip-entry \
  --store "$W/trial-store.sqlite3" \
  --work-dir "$W/trial-work" \
  --room-log "$W/room-log.json" \
  --device-id agent \
  --account-secret-hex "$SECRET" \
  --incident-alias <alias>-trial \
  --audit-log "$W/trial-audit.jsonl" \
  --max-skips <reviewed-bound>
rm -f "$W/secret.hex"
```

Before live apply, require the stopped live pair to remain byte-identical to
the checksummed rollback pair:

```sh
cmp /data/agent/client.sqlite3 /rollback/client.sqlite3
cmp /data/agent/client.sqlite3-wal /rollback/client.sqlite3-wal
```

#### 5. Repair (apply phase; container still stopped)

```sh
write_secret
SECRET=$(cat "$W/secret.hex")
/runtime/bin/finitechat repair skip-entry \
  --store /data/agent/client.sqlite3 \
  --work-dir "$W/repair-work" \
  --room-log "$W/room-log.json" \
  --device-id agent \
  --account-secret-hex "$SECRET" \
  --incident-alias <alias> \
  --audit-log "$W/repair-audit.jsonl" \
  --max-skips <reviewed-bound>
rm -f "$W/secret.hex"
```

- Phase 1 rehearses the skip list against byte copies and refuses without
  changing anything if the replay cannot reach the capture head. Phase 2
  applies only the derived skips, ascending, through the sanctioned monotonic
  cursor path; no entries are rewritten or deleted and no other table is
  touched.
- `--max-skips` defaults to 16 with a hard cap of 64. If the diagnosis finds
  more than 64 skips in the captured range, do NOT raise the cap: re-run the
  capture bounded to a nearer window (`--after-seq <CURSOR> --max-pages N`)
  and repair in sequential windows.

#### 6. Restart the existing agent container

Restart the container whose exact image, runtime, volume, network, and port
mapping were recorded in step 1. Do not create a replacement container for a
repair.

```sh
sudo nerdctl --namespace finite start "$CONTAINER"
test "$(sudo nerdctl --namespace finite inspect "$CONTAINER" --format '{{.State.Status}}')" = running
test "$(sudo nerdctl --namespace finite inspect "$CONTAINER" --format '{{.State.Health.Status}}')" = healthy
```

### VERIFY

1. Catch-up: the room's durable cursor reaches the server `last_seq` and
   sync ticks go quiet; held outbound messages drain.
2. If the captured valid tail contains Hermes inbox traffic, require a fresh
   `hermes-inbox.json` mtime inside the agent home as it lands. This is a
   conditional signal: non-Hermes application entries do not rewrite that
   file, so an unchanged mtime is not by itself a failed repair.
3. The Caddy `/sync/group` rate for that runner host falls from the
   refetch-storm rate to the steady hint cadence.
4. On images `≥ 2026-08-29.5`: the sidecar's quarantine stderr line stops
   recurring for the room and `/readyz` no longer names it in
   `runtime_status`.

### ROLLBACK

Restore the step-2 byte copies (store + WAL) into the Kata volume with the
existing container stopped, `sha256sum -c` against the recorded checksums,
and restart that same container with `nerdctl start`. A
quarantined-but-restored cursor is the pre-repair state:
safe, silent only up to the #776 backoff, and re-repairable. Never roll back
only one of the db/WAL pair.

### Records and privacy

- Every repair appends to `--audit-log` (JSONL, mode 0600): one line per
  applied skip plus a final summary (`apply` or `refused`). Keep the audit
  JSONL and the step-1 backups together, keyed by incident alias.
- The tooling is privacy-locked: seqs, kinds, SHA-256 entry bindings, error
  classes, cursor numbers, and counts only — never identifiers, plaintext,
  ciphertext, or secrets. Payloads are never read by anything in this
  procedure, including the operator.
- Secrets live only as in-VM mode-600 files during the procedure; the
  account secret is never a host-side CLI argument, never printed, never
  committed.

## 5. Exact relocation transaction (one stopped Kata Runtime)

Operator-only move between Finite-owned Kata hosts preserving Runtime ID,
Agent Principal, durable state ID, image artifact, and state schema; does
not retire or purge the source. The source compute and state stay stopped
and intact until the target passes its observation window. lat4's Gate F
fleet adoption uses exactly this contract per Runtime — no bulk binding
change at any point.

**Preconditions.** Core and both runner hosts run the
`runtime_relocation.v1` generation. A full quiesced Borg archive completed
and is visible from independently held credentials (or the scoped variant
below). Core shows the exact Runtime bound to the expected source host and
machine. The target advertises the same artifact/schema and persisted
capabilities, has space, and has no compute or durable directory for this
Runtime (a Runtime with `runtime_retirement=true` moves only to a runner
with its dedicated restricted retirement recovery set configured and
tested). No pending/running controls or retirement snapshot. The typed
`stop` request has succeeded (never substitute `nerdctl stop`; Core must
record the Runtime offline). Both runner timers drained while staging.
Abort on any mismatch; never delete/rename/modify source state.

**Capture.** Record PROJECT_ID, RUNTIME_ID, SOURCE_HOST_ID,
SOURCE_MACHINE_ID, DURABLE_STATE_ID, RUNTIME_ARTIFACT_ID,
STATE_SCHEMA_VERSION, EXPECTED_AGENT_NPUB. Verify the canonical source
container exists and is stopped with a succeeded stop receipt for this
exact binding. Locate the deployed runner binary
(`systemctl cat finite-saas-runner.service`) and use that exact binary on
BOTH hosts so the manifest algorithm is identical:

```sh
sudo <runner-bin> state-manifest \
  --path '<source-work-root>/kata/<durable-state-id>'
```

Record the 64-character SOURCE_MANIFEST (the command follows no symlinks,
hashes contents/paths/modes/symlink targets, rejects special files).

**Stage a provider-independent copy** (initiated on the operator Mac; SSH
encrypts both hops; the Mac retains nothing):

```sh
ssh <target-host> "sudo install -d -m 0700 '<target-work-root>/kata'"
ssh <source-host> \
  "sudo tar --acls --xattrs --numeric-owner --sparse -C '<source-work-root>/kata' -cpf - '<durable-state-id>'" \
| ssh <target-host> \
  "sudo tar --acls --xattrs --numeric-owner --sparse -C '<target-work-root>/kata' -xpf -"
```

No `--dereference`; no recursive copy that can cross into another Runtime.
Compute TARGET_MANIFEST on the target; require exact equality with
SOURCE_MANIFEST, and that no container named SOURCE_MACHINE_ID exists
there.

**Enqueue the exact relocation** (root shell; load `/etc/finite/core.env`
without printing it):

```sh
sudo sh -c '
  set -a
  . /etc/finite/core.env
  set +a
  exec /run/current-system/sw/bin/finite-saas-core runtime-cold-relocate-exact \
  --project-id "<project-id>" \
  --expected-agent-runtime-id "<runtime-id>" \
  --expected-source-host-id "<source-host-id>" \
  --expected-source-machine-id "<source-machine-id>" \
  --target-source-host-id "<target-source-host-id>" \
  --expected-agent-npub "<expected-agent-npub>" \
  --durable-state-manifest-sha256 "<source-manifest>" \
  --admin-email "<operator-email>" \
  --admin-workos-user-id "<operator-workos-user-id>"
'
```

Review the returned request (existing Runtime ID, exact target host,
`runtime_relocation.v1` envelope), then re-enable only the target runner
timer. The target runner fails closed unless the request is leased by the
exact target host; RuntimeSpec, Runtime ID, durable state ID, machine
name, and target path all agree; the staged tree still matches the
approved manifest; `agent/identity/identity.json` is a regular file;
target compute is absent before launch; and the launched `/contact`
exposes EXPECTED_AGENT_NPUB. Only then does Core replace the source
binding; durable state is never used as the secret transport.

**Absence variant (`--source-compute-absent`).** For a Runtime whose
source compute no longer exists (container and task both gone — e.g.
cleared by a containerd restart after a poisoned record), the
stopped-container preconditions are unsatisfiable and the flag accepts
exactly those two deviations, recording the attestation in the envelope.
Before using it, run the bounded absence probe on the source host and see
both results exactly:

```sh
timeout 15 nerdctl --namespace finite inspect '<SOURCE_MACHINE_ID>' ; echo "exit: $?"
# required: fatal "no such object <SOURCE_MACHINE_ID>" — NOT a timeout, NOT
# "context deadline exceeded" (that is a poisoned record, a different repair)
timeout 15 ctr -n finite tasks list | grep -c '<SOURCE_MACHINE_ID>'
# required: 0
```

A probe that times out or errors proves nothing and the flag must not be
used. The full-host Borg precondition may be replaced by a SCOPED boundary
for this variant: record the state-manifest hash and take a dedicated
off-host archive of the single durable directory before the transfer.

**VERIFY.** The relocation request is `running`; Core has the same Project,
Runtime ID, artifact, schema, and Principal now bound to the target host
and same machine name; the target container is running and healthy; Finite
Chat receives a round trip from the existing Principal; Sites, Brain,
workspace files, Hermes memory, and installed skills are present; source
compute remains stopped and source durable state still exists; no source
runner work restarted the old binding; the target runner is still drained
after its bounded lease attempt; the target has named recovery coverage
for post-relocation writes. Observe before broad use; record request id,
both manifests, bindings, archive name, timestamps, result — no secret
values.

**ROLLBACK.** Before Core switches the binding, a failed request must
remove target compute and preserves the existing Core link and both durable
trees — verify compute is actually absent rather than assuming cleanup
succeeded; a booted target may have changed the staged manifest even when
Core rejected registration (preserve that tree under a request-specific
non-canonical name, then restage from the stopped source). After Core
switches the binding, never manually start source compute (two writers);
stop the target through Core first. A reverse relocation is a NEW exact
transaction: preserve the stale source canonical copy under an explicitly
approved non-canonical rollback name, stage the stopped target tree into
the absent canonical path, verify its manifest, and run the same contract
with source/target reversed. If the target modified durable state and
cannot be stopped cleanly, fail closed: preserve both sides and restore
the named pre-move archive to an empty recovery target rather than guessing
which tree is canonical.

## 6. Boundaries

- A NixOS rollback is not a data rollback; a generation switch never
  touches database or StateDirectory contents.
- A green archive, a green timer, or a successful `borg check` is not a
  proved restore — only an empty-target drill is.
- Restore never confers authority: a selected row, sort order, identifier
  order, or other navigation state never authorizes choosing or rewriting
  durable user state; ambiguous state fails closed without mutation.
- Every restored writer is fenced until it is the confirmed single writer
  (chat above all — [incident.md](incident.md) §4 single-writer doctrine).
- Snapshot SQLite is inspected only through `scripts/snapshot-sqlite`
  (private scratch copy); sealed snapshots are verified by SHA-256 only.
