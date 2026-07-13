# Deploying finite-brain on lat1

Finite Brain runs as `finite-brain-app.service` on finite-lat-1, bound only to
`127.0.0.1:3015`. The dashboard proxies `/health`, `/client`, and `/_admin/*`
to that loopback service. WorkOS protects the browser Product Client at
`/client`; `/_admin/*` bypasses WorkOS so Brain can enforce its own route-level
Nostr, invitation-proof, or other narrowly specified authorization. There is
no second Brain vhost and no oauth2-proxy.

The SQLite database is `/var/lib/private/finitebrain/finite-brain.sqlite3`.
Compute deployment and data migration are separate operations. Never replace
the database without a byte-for-byte rollback copy.

## Preconditions

- The exact mono commit is pushed and its production NixOS configuration
  evaluates successfully.
- You can SSH from the Mac to `ubuntu@finite-lat-2` with agent forwarding for
  root access from lat2 to `64.34.82.77`. Lat2 is the only production
  build/driver host; do not evaluate or build on the Mac, clawland, or lat1.
- `finite.computer` dashboard auth is healthy.
- A consistent SQLite backup has been copied from the current source and its
  size plus SHA-256 recorded outside the database contents.
- The previous NixOS generation and source Brain service remain available for
  rollback until the lat1 Product Client and `fbrain` proofs pass.

## First migration from smoke

1. On smoke, make a SQLite online backup (or briefly stop the service if the
   installed SQLite lacks `.backup`), then restart it immediately. Do not move
   the live file while the service is writing.
2. Copy the backup to a root-only staging path on lat1 and record its SHA-256.
3. Deploy the pinned mono revision. Let systemd create the DynamicUser state
   directory and an empty database if necessary.
4. Stop `finite-brain-app` on lat1, keep a rollback copy of any destination
   database, replace it with the staged backup, match the destination file's
   owner/mode, and start the service.
5. Leave smoke unchanged until verification completes.

## Normal deploy

From the reviewed checkout, prebuild the full pushed commit on lat2. Record
both lines; `REV` must be exactly 40 lowercase hex characters, not a tag,
branch, abbreviation, or dirty tree:

```sh
set -euo pipefail
git fetch origin --prune
REV="$(git rev-parse HEAD)"
[[ "$REV" =~ ^[0-9a-f]{40}$ ]]
git merge-base --is-ancestor "$REV" origin/main
SYSTEM="$(just nixos-build-lat1 "$REV")"
printf 'REV=%s\nSYSTEM=%s\n' "$REV" "$SYSTEM"
```

The helper prints the exact system output and roots it on lat2. SSH to lat2
with temporary agent forwarding, paste the recorded values without recomputing
them, and deploy only that prebuilt closure:

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

All Nix commands use empty builders, so evaluation/building stays local to
lat2. Installing the system profile first preserves boot/generation rollback;
transient-unit activation survives SSH loss and cannot build on lat1. Brain is
built with the rest of the monorepo from that revision; no source tarball or
legacy-repo deploy is part of the path.

## Verify

```sh
set -euo pipefail
ssh root@64.34.82.77 systemctl is-active finite-brain-app
ssh root@64.34.82.77 curl -fsS http://127.0.0.1:3015/health
curl -fsS https://finite.computer/health
curl -fsS -o /dev/null -w '%{http_code}\n' https://finite.computer/client
```

The public `/health` route must report the Brain service healthy and `/client`
must require a WorkOS session. A signed `fbrain` request to `/_admin/*` must
reach Brain without a WorkOS session. In an authenticated browser, verify the
Product Client loads and completes a real `/_admin/*` request through the
dashboard. Then run `fbrain doctor` and a write/read proof from an authorized
Nostr identity against `https://finite.computer`.

## Rollback

1. Switch lat1 to the previous NixOS generation and record the resulting
   `/run/current-system`; for a deliberate rollback, prebuild and deploy the
   previous known-good rev's exact closure from lat2 and verify that path.
2. If Brain data was written on lat1, preserve that database before restoring
   the pre-migration rollback copy; do not discard either side.
3. Keep or restore the smoke service as the temporary endpoint while deciding
   how to reconcile post-cutover writes.

A NixOS rollback is not a data rollback. Offsite Recovery Snapshot and
empty-target restore remain TODO; do not claim them until a restore drill has
passed.
