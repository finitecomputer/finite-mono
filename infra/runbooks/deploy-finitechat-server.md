# Deploying the finitechat server (chat.finite.computer)

## Where it runs

**finite-lat-1 (64.34.82.77), NixOS.** Since the 2026-07-09 cutover
`chat.finite.computer` DNS points at lat1; the server is systemd unit
`finitechat-server.service` binding **`127.0.0.1:8788`** (moved off 8787,
which finitesitesd owns on this consolidated box — the public URL is
unchanged), DynamicUser with SQLite at the real path
`/var/lib/private/finite-chat/data/server.sqlite3`, fronted by the one host
Caddy (`chat.finite.computer` → 127.0.0.1:8788, Let's Encrypt cert via ACME
HTTP-01). Config: `infra/nixos/modules/finitechat-server.nix`; topology:
`infra/nixos/README.md`; box rebuild:
[lat1-nixos-reinstall.md](lat1-nixos-reinstall.md).

The migration from clawland is **DONE**: `finitechat-server` on clawland is
`systemctl disable`d (single-writer doctrine below), and the SQLite was
carried to lat1 per that discipline. Deploys are now nixos-rebuild pinned to
a mono rev (`nixos-rebuild switch --target-host root@finite-lat-1
--flake github:finitecomputer/finite-mono/<tag-or-rev>#finite-lat-1`) — the
flake builds `finitechat-server` from the pinned rev.

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

1. Deploy the release rev to lat1:

   ```sh
   nixos-rebuild switch --target-host root@finite-lat-1 \
     --flake github:finitecomputer/finite-mono/<tag-or-rev>#finite-lat-1
   ```

   (This is a routine in-place server update, not a host move — no data
   migration. A host MOVE follows the single-writer doctrine below.)
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

`ssh root@finite-lat-1 nixos-rebuild switch --rollback` (or re-deploy the
previous known-good mono rev), then re-run the gate against it. Data rollback
(SQLite) comes from the coordinated Hosted Web Chat recovery set. FLAG: its
rsync.net repository is configured in Nix, but credentials, deployment, first
archive, and empty-target proof are still outstanding on single-disk lat1.
Follow [hosted-web-chat-recovery.md](hosted-web-chat-recovery.md); configuration
alone is not a backup.

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

## Host MOVES — SEPARATELY SCHEDULED, NOT ROUTINE

The clawland → lat1 move is DONE. Any FUTURE host move (e.g. lat1 → a
successor box, or splitting chat back onto dedicated hardware) is a
deliberate cutover, NOT a routine deploy, and follows the single-writer
doctrine above exactly: disable the old writer FIRST, WAL-checkpoint, carry
the quiesced SQLite, start + verify the new writer via direct IP, and only
then flip `chat.finite.computer` DNS (keep the TTL low ahead of the move).
Chat had no users at the 2026-07-09 move, so the outage window was free —
treat that as rehearsal, not license to skip the discipline when it is not.

The `lat1-nixos-reinstall.md` "Data restore → Chat" note is the compact
checklist for standing chat up on a freshly built lat1; a full host-to-host
move should get its own dated runbook at the time it is scheduled.
