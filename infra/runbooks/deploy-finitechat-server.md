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
carried to lat1 per that discipline. Deploys now use a prebuilt immutable mono
rev. Production evaluation/build happens only on `ubuntu@finite-lat-2` with
builders disabled; lat2 copies the exact closure to lat1 and switches it
directly. The flake builds `finitechat-server` from that pinned rev.

## The contract gate (applies to EVERY server deploy)

Per `finitechat/docs/server-deployment-gate.md`: production `GET /health`
must report `server_contract_version`, `server_version`, `source_commit`
matching the expected finite-chat commit, and `source_dirty: false`.

### PRECONDITIONS

- The server commit to deploy is on mono `main`; local
  `cargo test -p finitechat-server` suites pass.
- You know the expected post-deploy `/health` payload (contract version,
  12-char source commit).
- You can SSH from the Mac to `ubuntu@finite-lat-2` with agent forwarding for
  root access from lat2 to `64.34.82.77`. Do not build on the Mac, clawland,
  or lat1.

### STEPS

1. From the reviewed checkout, prebuild the full pushed commit on lat2 and
   record both immutable handoff values:

   ```sh
   set -euo pipefail
   git fetch origin --prune
   REV="$(git rev-parse HEAD)"
   [[ "$REV" =~ ^[0-9a-f]{40}$ ]]
   git merge-base --is-ancestor "$REV" origin/main
   SYSTEM="$(just nixos-build-lat1 "$REV")"
   printf 'REV=%s\nSYSTEM=%s\n' "$REV" "$SYSTEM"
   ```

   `REV` must be the exact lowercase 40-hex commit, not a tag, branch, short
   hash, or dirty tree. The printed `/nix/store/...` path is GC-rooted on lat2.

2. SSH to lat2, paste those exact values, and deploy only that prebuilt path:

   ```sh
   ssh -A ubuntu@finite-lat-2
   ```

   On lat2, run:

   ```sh
   set -euo pipefail
   REV='<exact-40-hex-rev-from-prebuild>'
   SYSTEM='<exact-/nix/store-path-from-prebuild>'
   [[ "$REV" =~ ^[0-9a-f]{40}$ ]] || exit 64
   [[ "$SYSTEM" =~ ^/nix/store/[0-9a-z]{32}-nixos-system-finite-lat-1-[^/[:space:]]+$ ]] || exit 64
   ROOT="$HOME/.local/state/finite-mono/lat1-closures/$REV"
   test -L "$ROOT"
   test "$(readlink -f "$ROOT")" = "$SYSTEM"
   nix path-info --option builders '' "$SYSTEM" >/dev/null
   ssh -o BatchMode=yes root@64.34.82.77 true
   # The exact lat2-built closure is unsigned; authenticated root SSH is the
   # trust boundary for this reviewed handoff.
   nix copy --no-check-sigs --option builders '' \
     --to ssh-ng://root@64.34.82.77 "$SYSTEM"

   UNIT="finite-nixos-activate-${REV}.service"
   ssh -o BatchMode=yes root@64.34.82.77 \
     bash -s -- "$REV" "$SYSTEM" "$UNIT" <<'LAT1'
   set -euo pipefail
   rev="$1"
   system="$2"
   unit="$3"
   [[ "$rev" =~ ^[0-9a-f]{40}$ ]] || exit 64
   [[ "$system" =~ ^/nix/store/[0-9a-z]{32}-nixos-system-finite-lat-1-[^/[:space:]]+$ ]] || exit 64
   [[ "$unit" == "finite-nixos-activate-${rev}.service" ]] || exit 64
   test "$(readlink -f "$system")" = "$system"
   test -x "$system/bin/switch-to-configuration"
   nix-store --check-validity "$system" >/dev/null
   load_state="$(systemctl show --property=LoadState --value "$unit" 2>/dev/null || true)"
   [[ "$load_state" == not-found ]] || {
     echo "refusing to replace existing transient unit $unit ($load_state)" >&2
     exit 73
   }
   nix-env --option builders '' --profile /nix/var/nix/profiles/system \
     --set "$system"
   test "$(readlink -f /nix/var/nix/profiles/system)" = "$system"
   systemd-run --quiet --unit="$unit" --property=Type=oneshot \
     --property=RemainAfterExit=yes --no-block \
     "$system/bin/switch-to-configuration" switch
   LAT1

   deadline=$((SECONDS + 600))
   while true; do
     if ! state="$(ssh -o BatchMode=yes -o ConnectTimeout=5 root@64.34.82.77 \
       systemctl show --property=ActiveState --value "$UNIT" 2>/dev/null)"; then
       state=unreachable
     fi
     case "$state" in
       active) break ;;
       activating|inactive|unreachable) ;;
       failed)
         ssh -o BatchMode=yes root@64.34.82.77 \
           journalctl --no-pager -n 100 -u "$UNIT" >&2 || true
         exit 1
         ;;
       *) echo "unexpected activation state: $state" >&2; exit 1 ;;
     esac
     (( SECONDS < deadline )) || { echo "activation timed out" >&2; exit 1; }
     sleep 2
   done
   PROFILE="$(ssh -o BatchMode=yes root@64.34.82.77 \
     readlink -f /nix/var/nix/profiles/system)"
   ACTUAL="$(ssh -o BatchMode=yes root@64.34.82.77 \
     readlink -f /run/current-system)"
   test "$PROFILE" = "$SYSTEM"
   test "$ACTUAL" = "$SYSTEM"
   ssh -o BatchMode=yes root@64.34.82.77 systemctl stop "$UNIT"
   ```

   This is a routine in-place server update, not a host move — no data
   migration. A host MOVE follows the single-writer doctrine below. Empty
   builders keep all evaluation/building local to lat2. Installing the system
   profile first preserves boot/generation rollback, while transient-unit
   activation survives SSH loss and cannot build on lat1.

3. After the deploy, run the gate from a mono checkout at the release
   commit:

   ```sh
   set -euo pipefail
   export FINITECHAT_RELEASE_COMMIT="$(git rev-parse --short=12 HEAD)"
   finitechat/scripts/server-contract-gate.py \
     --server https://chat.finite.computer \
     --expected-source "$FINITECHAT_RELEASE_COMMIT"
   ```

### VERIFY

1. Gate passes (exact `source_commit`, `source_dirty: false`).
2. Post-deploy smoke from the gate doc:

   ```sh
   set -euo pipefail
   cargo run -q -p finitechat-cli -- http --server https://chat.finite.computer health
   cargo test -p finitechat-server --test http_routes
   cargo test -p finitechat-server --test http_persistence
   ```

3. No app/TestFlight build ships while the gate fails — that is the point.

### ROLLBACK

`ssh root@64.34.82.77 nixos-rebuild switch --rollback` (or prebuild and
deploy the previous known-good rev's exact closure from lat2), then verify
`/run/current-system` against the selected rollback path and re-run the gate.
Data rollback (SQLite) comes from the coordinated Hosted Web Chat recovery
set. FLAG: its
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
