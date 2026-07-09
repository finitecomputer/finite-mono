# Deploying the finitechat server (chat.finite.computer)

## Where it runs today

**clawland-ovh (15.204.108.57), not lat1.** `chat.finite.computer` DNS points
at clawland; the server is systemd unit `finitechat-server.service` binding
`10.42.0.1:8787` with SQLite at `/var/lib/finite-chat/data/server.sqlite3`,
fronted by the legacy fleet's Traefik/oauth stack, **borg-backed**
(`fc-offsite-backup.service` + timer). Full map:
`infra/hosts/clawland/finitechat-server.md` and
`infra/hosts/clawland/README.md`.

Server deploys to clawland are performed **from the legacy `finitecomputer`
repo** ("box1") — mono owns the server source and the release gate, not the
clawland deploy mechanics. TODO: the exact legacy-side deploy command for
clawland is not captured in mono; record it here (or supersede it) at the
next production server deploy.

## The contract gate (applies to EVERY server deploy)

Per `finitechat/docs/server-deployment-gate.md`: production `GET /health`
must report `server_contract_version`, `server_version`, `source_commit`
matching the expected finite-chat commit, and `source_dirty: false`.

### PRECONDITIONS

- The server commit to deploy is on mono `main`; local
  `cargo test -p finitechat-server` suites pass.
- You know the expected post-deploy `/health` payload (contract version,
  12-char source commit).

### STEPS

1. Hand off to the deploy lane per the gate doc (branch + full SHA, whether
   companion services like `push-drain` are needed, rollback notes).
2. After the deploy, run the gate from a mono checkout at the release
   commit:

   ```sh
   export FINITECHAT_RELEASE_COMMIT="$(git rev-parse --short=12 HEAD)"
   finitechat/scripts/server-contract-gate.py \
     --server https://chat.finite.computer \
     --expected-source "$FINITECHAT_RELEASE_COMMIT"
   ```

### VERIFY

1. Gate passes (exact `source_commit`, `source_dirty: false`).
2. Post-deploy smoke from the gate doc:

   ```sh
   cargo run -q -p finitechat-cli -- http --server https://chat.finite.computer health
   cargo test -p finitechat-server --test http_routes
   cargo test -p finitechat-server --test http_persistence
   ```

3. No app/TestFlight build ships while the gate fails — that is the point.

### ROLLBACK

Redeploy the previous known-good commit through the same lane and re-run the
gate against it. Data rollback (SQLite) comes from clawland's borg backups —
TODO: the borg restore procedure for `/var/lib/finite-chat/data/` has not
been drilled from mono's side; drill and document it (same discipline as
[postgres-backup-restore.md](postgres-backup-restore.md)).

## Single-writer doctrine (Paul, 2026-07-09 — applies to every chat move, forever)

The chat protocol depends on the server being **one ordered log**. There must
never be two servers able to accept writes for the same database, and the
server must never "half-accept" traffic during a move. **Fail closed: if chat
has to go down, it goes DOWN** — connection refused is correct; split state is
unrecoverable. Concretely, any migration follows this exact order:

1. `systemctl disable --now finitechat-server` on the OLD host — disable, not
   just stop, so nothing (reboot, reconcile loop) can resurrect a second
   writer. Verify the port no longer answers.
2. Checkpoint the WAL (`sqlite3 server.sqlite3 "PRAGMA wal_checkpoint(TRUNCATE);"`)
   and copy the database ONLY after step 1.
3. Start the NEW server; verify via direct IP
   (`curl --resolve chat.finite.computer:443:<new-ip> https://chat.finite.computer/health`)
   — contract version + source_commit + `source_dirty:false`.
4. Only then flip DNS. During the TTL window, clients cached on the old IP
   get connection refused — a clean outage, by design.
5. Rollback inverts the same discipline: stop+disable the NEW server BEFORE
   re-enabling the old one, and carry the database back (any writes the new
   server accepted must move with it or be consciously discarded).

## Future lat1 cutover — SEPARATELY SCHEDULED, NOT ROUTINE

Moving the server to lat1 is a deliberate cutover with a data migration
following the single-writer doctrine above: quiesced SQLite copy of
`/var/lib/finite-chat/data/server.sqlite3` plus a DNS flip of
`chat.finite.computer` from 15.204.108.57 to 64.34.82.77 (TTL already
lowered to 300). Chat has no users as of 2026-07-09, so the outage window
is free — the discipline is rehearsal for when it will not be. It is
explicitly NOT part of routine deploys
(`infra/hosts/clawland/finitechat-server.md`, "Migration story").

The deploy script for that future home is
`infra/hosts/lat1/scripts/deploy-finitechat-server.sh` — **do not run it
as-is**. Its header documents known host mismatches (it was written for a
NixOS/Traefik host; lat1 is Ubuntu with `--disable=traefik` and host-Caddy
edge): `nix shell` build, Traefik IngressRoute CRDs, dangling NixOS nologin
shell. TODO: reconcile the script (cargo or CI-built binary; Caddy vhost
instead of IngressRoutes) before scheduling the cutover, and write the
cutover checklist as its own runbook at that time.

## Small follow-up: clean the rolled-back Jul 7 leftovers on lat1

The 2026-07-07 lat1 deploy attempt ran ~2 minutes and was rolled back
(lat1 README appendix item 7). Still on lat1:

1. `finite-chat` user (UID 986, dangling NixOS nologin shell) + group.
2. `/var/lib/finite-chat/` — release binary, src clone at the pinned SHA,
   and a small SQLite DB with a **197KB WAL that was never checkpointed**.

Cleanup steps: if that 2-minute window's data could matter, checkpoint
before archiving (`sqlite3 <db> 'PRAGMA wal_checkpoint(TRUNCATE);'`), tar
`/var/lib/finite-chat` aside, then remove the directory and
`userdel finite-chat` / `groupdel finite-chat`. Do it before the real
cutover so the migration starts from a clean slate.
