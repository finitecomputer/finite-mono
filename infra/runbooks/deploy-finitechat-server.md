# Deploying the finitechat server (chat.finite.computer)

## Where it runs

**finite-lat-2 (64.34.80.19), NixOS.** Since the 2026-08-29 ADR 0007 cutover
`chat.finite.computer` DNS points at lat2; the server is systemd unit
`finitechat-server.service` binding **`127.0.0.1:8788`** (moved off 8787,
which finitesitesd owns on this consolidated app-plane host — the public URL is
unchanged), DynamicUser with SQLite at the real path
`/var/lib/private/finite-chat/data/server.sqlite3`, fronted by the one host
Caddy (`chat.finite.computer` → 127.0.0.1:8788, Let's Encrypt cert via ACME
HTTP-01). Config: `infra/nixos/modules/finitechat-server.nix`; topology:
`infra/nixos/README.md`. The
[2026-07-09 bare-metal transcript](lat1-nixos-reinstall.md) supplies historical
facts only and is not current rebuild/recovery authority.

The migration from clawland is **DONE**: `finitechat-server` on clawland is
`systemctl disable`d (single-writer doctrine below), and the SQLite was
carried onto the app-plane host per that discipline. Deploys now use a
prebuilt immutable mono rev. Production evaluation/build happens in the
`Lat2 NixOS Closure` workflow
on a Depot-managed x86_64 Linux runner; the deploy script copies that exact
artifact to lat2 and switches it only after the explicit activation step. The flake builds
`finitechat-server` from that pinned rev.

## The contract gate (applies to EVERY server deploy)

Per `finitechat/docs/server-deployment-gate.md`: production `GET /health`
must report `server_contract_version`, `server_version`, the Nix-derived
`source_fingerprint` matching the selected Chat package, and
`source_dirty: false`.

### PRECONDITIONS

- The server commit to deploy is on mono `main`; local
  `cargo test -p finitechat-server` suites pass.
- You know the expected post-deploy `/health` payload (contract version,
  automatically derived Chat source fingerprint).
- The reviewed revision has a successful `Lat2 NixOS Closure` workflow artifact
  and the deploy operator can SSH to `root@64.34.80.19`. Do not evaluate or
  build the production closure on the Mac, clawland, lat1, or lat2.

### STEPS

1. Build and download the reviewed revision's `lat2-nixos-closure-REV`
   artifact with the shared procedure in
   [deploy-core.md](deploy-core.md#steps). `REV` must be the exact lowercase
   40-hex commit on `origin/main`, not a tag, branch, short hash, or dirty
   tree.

2. Deploy that artifact with:

   ```sh
   just deploy-lat2-closure "$ARTIFACT_DIR" --prepare
   scripts/finite-status
   just deploy-lat2-closure "$ARTIFACT_DIR" --activate
   ```

   This is a routine in-place server update, not a host move — no data
   migration. A host MOVE follows the single-writer doctrine below. The deploy
   script validates the manifest, copies the prebuilt file binary cache to
   lat2, dry-activates during `--prepare`, and proves `/run/current-system`
   equals the artifact's exact `SYSTEM` path after `--activate`. It does not
   evaluate or build on lat1 or lat2. Activation holds the monitoring timers
   across the switch, warns instead of failing on a monitoring-only unit
   failure, and never rolls back automatically: a failed activation leaves the
   new generation current and prints the operator revert recipe, legal only
   under the `rollback-check` condition in ROLLBACK below.

3. After the deploy, run the gate from a mono checkout at the release commit.
   This evaluation reads package metadata and does not rebuild the closure:

   ```sh
   set -euo pipefail
   export FINITECHAT_SOURCE_FINGERPRINT="$(
     nix eval --option builders '' --raw \
       .#packages.x86_64-linux.finitechat-server.sourceFingerprint
   )"
   finitechat/scripts/server-contract-gate.py \
     --server https://chat.finite.computer \
     --expected-fingerprint "$FINITECHAT_SOURCE_FINGERPRINT"
   ```

### VERIFY

1. Gate passes (exact `source_fingerprint`, `source_dirty: false`).
2. Post-deploy smoke from the gate doc:

   ```sh
   set -euo pipefail
   cargo run -q -p finitechat-cli -- http --server https://chat.finite.computer health
   cargo test -p finitechat-server --test http_routes
   cargo test -p finitechat-server --test http_persistence
   ```

3. No app/TestFlight build ships while the gate fails — that is the point.
4. systemd now restarts hosted-device with the server; curl Chat liveness at
   `http://127.0.0.1:8788/health`, semantic serving readiness at
   `http://127.0.0.1:8788/readyz`, and hosted-device health at
   `http://127.0.0.1:38918/healthz`.

### ROLLBACK

No deploy script reverts automatically. Reverting the server generation is
legal ONLY if `finitechat-server rollback-check --sqlite
/var/lib/finite-chat/data/server.sqlite3` exits 0 AND the pre-fold backup is
restored first (stop the server, restore, then switch generation — the
single-writer order below); otherwise roll forward with a newer closure. An
older binary must never serve the folded database. Under that condition,
select the previous generation (`nixos-rebuild switch --rollback`, or
build/download/deploy the previous known-good rev's exact closure artifact),
then verify `/run/current-system` against the selected path and re-run the
gate. If the selected server predates
`/readyz`, roll the dedicated monitoring receiver back to its `/health` target
as well so monitoring-version skew does not masquerade as a Chat outage.
Data rollback (SQLite) comes from the coordinated Hosted Web Chat recovery
set. FLAG: the rsync.net repository has a verified first archive and its
offsite-health jobs passed the 2026-07-18 live inventory, but the complete
empty-target proof is still outstanding. Snapshot creation is deploy/manual-
triggered after the disruptive 15-minute timer was removed, so the accepted
RPO is also unproved. Agent Runtime `/data` is outside this Recovery Set. Follow
[hosted-web-chat-recovery.md](hosted-web-chat-recovery.md); a green archive
alone is not a proved restore.

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
   — contract version + source fingerprint + `source_dirty:false`.
4. Only then flip DNS. During the TTL window, clients cached on the old IP
   get connection refused — a clean outage, by design.
5. Rollback inverts the same discipline: stop+disable the NEW server BEFORE
   re-enabling the old one, and carry the database back (any writes the new
   server accepted must move with it or be consciously discarded).

## Host MOVES — SEPARATELY SCHEDULED, NOT ROUTINE

The clawland → lat1 and lat1 → lat2 moves are DONE. Any FUTURE host move (for
example splitting chat back onto dedicated hardware) is a
deliberate cutover, NOT a routine deploy, and follows the single-writer
doctrine above exactly: disable the old writer FIRST, WAL-checkpoint, carry
the quiesced SQLite, start + verify the new writer via direct IP, and only
then flip `chat.finite.computer` DNS (keep the TTL low ahead of the move).
Chat had no users at the 2026-07-09 move, so the outage window was free —
treat that as rehearsal, not license to skip the discipline when it is not.

The historical `lat1-nixos-reinstall.md` “Data restore → Chat” note records the
2026-07-09 path mapping only; it is not a current rebuild checklist. A future
host move follows the finite-lat recovery plan and gets its own accepted,
rehearsed cutover record before any writer moves.
