# Deploying finite-saas-core (and dashboard) to lat1

Since the 2026-07-09 consolidation cutover, Core and the dashboard are NixOS
services on finite-lat-1 (64.34.82.77). Config lives in `infra/nixos/`
(host `finite-lat-1`, modules `modules/finite-saas-core.nix` +
`modules/dashboard.nix`); topology and secrets checklist:
`infra/nixos/README.md`. Rebuild/recovery of the box itself:
[lat1-nixos-reinstall.md](lat1-nixos-reinstall.md).

- **Core** = systemd unit `finite-saas-core.service`, binds 127.0.0.1:4200,
  DynamicUser, `EnvironmentFile=/etc/finite/core.env`. Talks to native
  Postgres at 127.0.0.1:5432 via `FC_CORE_DATABASE_URL`.
- **Dashboard** = podman container `finite-saas-dashboard` (host-net, binds
  127.0.0.1:3000), image **digest-pinned** in `modules/dashboard.nix`
  (`ghcr.io/finitecomputer/finite-saas-dashboard@sha256:...`),
  `EnvironmentFile=/etc/finite/dashboard.env`, `FC_CORE_BASE_URL=
  http://127.0.0.1:4200`.
- **Edge** = the single host Caddy: `finite.computer/internal/finite-private/*`
  → core:4200, everything else → dashboard:3000.

> History: this box previously ran Core/dashboard/Postgres as a single-node
> k3s cluster with on-host podman builds and `kubectl set image`. That cluster
> is GONE (wiped at the cutover). Do not resurrect the kubectl flow.

## Deploy flow — prebuilt immutable mono rev

Deploying a release IS pinning the flake: the mono rev you build is the rev
the host runs (binaries + config together). The dashboard is the exception —
it deploys as a digest-pinned GHCR container, so bumping it is an edit to
`modules/dashboard.nix`.

### PRECONDITIONS

- The change (Core source and/or the dashboard digest bump) is merged to
  `main` — you deploy a committed rev, not a working tree.
- Before deploying the RuntimeSpec generation, verify the Core Nix module
  carries the same non-secret `FINITE_SITES_API`,
  `FINITE_BRAIN_SERVER_URL`, and `FINITE_BRAIN_PUBLIC_BASE_URL` values in
  `FC_CORE_RUNTIME_ENV_JSON` that previously lived only in Runner config.
  Runner's `FC_RUNNER_RUNTIME_ENV_JSON` is N-1 fallback only.
- ssh access from the Mac to `ubuntu@finite-lat-2`, with agent forwarding for
  root access from lat2 to `64.34.82.77`. Lat2 is the only production
  build/driver host; never evaluate or build this closure on the Mac,
  clawland, or lat1.
- For a dashboard bump: the new image is CI-built and pushed to GHCR, and you
  have its `name@sha256:...` digest (from the Service Images workflow summary).

### STEPS

1. **Core (and any config/module change):** From the reviewed checkout, select
   the full commit, prove it is on `origin/main`, and prebuild it on lat2. The
   helper's stdout is the exact, GC-rooted system closure path:

   ```sh
   set -euo pipefail
   git fetch origin --prune
   REV="$(git rev-parse HEAD)"
   [[ "$REV" =~ ^[0-9a-f]{40}$ ]]
   git merge-base --is-ancestor "$REV" origin/main
   SYSTEM="$(just nixos-build-lat1 "$REV")"
   printf 'REV=%s\nSYSTEM=%s\n' "$REV" "$SYSTEM"
   ```

   Record both values. `REV` must be exactly 40 lowercase hex characters; do
   not hand off a tag, branch, abbreviation, or dirty working tree.

2. SSH to lat2 and paste the recorded values exactly. Confirm the helper's
   per-revision GC root, copy the closure, switch lat1 directly to that already
   built closure, and assert the running closure is the path handed off:

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
   nix copy --option builders '' --to ssh-ng://root@64.34.82.77 "$SYSTEM"

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
   nix path-info --option builders '' "$system" >/dev/null
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

   This first installs `SYSTEM` as the system profile generation, then runs
   activation in a transient systemd unit that survives SSH loss. Every Nix
   command has builders explicitly disabled, and the direct switch does not
   build on lat1.

3. **Dashboard image bump:** edit `image = "...@sha256:..."` in
   `infra/nixos/modules/dashboard.nix`, commit to `main` — the committed
   digest is the deploy record and the rollback target — then repeat steps 1–2
   for the new rev. podman pulls the pinned digest.

### VERIFY

1. Core health directly on the box:

   ```sh
   ssh root@64.34.82.77 'curl -fsS http://127.0.0.1:4200/healthz'
   ```

2. Through the edge: `curl -fsS https://finite.computer/` (dashboard) and
   `curl -fsS https://finite.computer/internal/finite-private/` → 401
   (core alive + gated — the limiter path).
3. Units are up: `ssh root@64.34.82.77 'systemctl status finite-saas-core
   finite-saas-dashboard'` (`podman-finite-saas-dashboard.service` for the
   container unit name if querying journald).
4. Core still exposes no `source_commit` health payload. The authoritative
   identity check is therefore the exact comparison of `/run/current-system`
   to the prebuilt `SYSTEM` path in step 2; a generation number alone is not
   sufficient.

### ROLLBACK

1. Fast path: `ssh root@64.34.82.77 nixos-rebuild switch --rollback` — boots
   the previous generation (both Core binary and dashboard digest revert
   together). Then reconcile git to match what is running within a day
   (break-glass rule).
2. Deliberate path: prebuild the previous known-good full mono rev using the
   same helper, then copy/switch/verify its exact `SYSTEM` path from lat2 (and,
   for a dashboard-only regression, first revert the digest in
   `modules/dashboard.nix`).
3. Re-run VERIFY.
