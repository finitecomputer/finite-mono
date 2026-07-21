# Deploying finite-saas-core (and dashboard) to lat1

Since the 2026-07-09 consolidation cutover, Core and the dashboard are NixOS
services on finite-lat-1 (64.34.82.77). Config lives in `infra/nixos/`
(host `finite-lat-1`, modules `modules/finite-saas-core.nix` +
`modules/dashboard.nix`); topology and secrets checklist:
`infra/nixos/README.md`. The
[2026-07-09 bare-metal transcript](lat1-nixos-reinstall.md) supplies historical
facts only; no current destructive rebuild/recovery authority exists.

- **Core** = systemd unit `finite-saas-core.service`, binds 127.0.0.1:4200,
  DynamicUser, `EnvironmentFile=/etc/finite/core.env`. Talks to native
  Postgres at 127.0.0.1:5432 via `FC_CORE_DATABASE_URL`.
- **Dashboard** = podman container `finite-saas-dashboard` (host-net, binds
  127.0.0.1:3000), image **digest-pinned** in `modules/dashboard.nix`
  (`ghcr.io/finitecomputer/finite-saas-dashboard@sha256:...`),
  `EnvironmentFile=/etc/finite/dashboard.env`, `FC_CORE_BASE_URL=
  http://127.0.0.1:4200`.
- **Edge** = the single host Caddy: `finite.computer/internal/finite-private/*`
  plus the exact API-key usage and reset paths → core:4200, everything else →
  dashboard:3000. No general `/api/core/*` route is public.

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
- For a Core schema change: capture the pre-deploy Postgres backup named in
  [postgres-backup-restore.md](postgres-backup-restore.md), record its path and
  checksum, and keep the previous exact system closure as the binary rollback
  target. A binary rollback does not reverse additive schema changes.
- Before enabling Finite Private reset epochs, query production read-only for
  total reservation cardinality, active `reserved` rows (including age), and
  `EXPLAIN` the grant/epoch/status/window usage sum. Record recent reservation
  counts separately from historical rows left in `reserved`; never rewrite
  either as part of deployment. Stop if a recent reservation is still plausibly
  in flight or the added per-turn status query lacks measured headroom; do not
  discover either condition after activation. Migration 0014 adds the reviewed
  `(grant_id,status,burst_window_epoch,created_at)` index; include its brief DDL
  lock in the activation window and verify it exists afterward.
- Finite Private epoch/reset history is a one-way Core binary boundary. Once
  this generation accepts traffic, do not use the ordinary N-1 binary rollback:
  N-1 ignores epochs and can charge a freshly reset window from a late
  settlement, and N-1 rows can be undercounted after re-upgrade. Prefer a
  forward fix on the epoch-aware generation.

### STEPS

> **Automated path:** `just deploy-lat1 <exact-40-hex-rev>` performs steps 1-2
> end-to-end (lat2 prebuild, closure copy, switch, state verification) and is
> the preferred way to run them. It stages the switch script on lat2 as a file
> — running it over ssh stdin fails silently with exit 0 because the inner ssh
> calls consume the remaining script. The manual steps below remain the
> reference for what it does and for break-glass situations.

To roll a reviewed, healthy existing Runtime cohort after the deployment has
passed its normal verification, append an exact artifact id, a real admin
identity, and one or more explicit project ids:

```sh
just deploy-lat1 "$REV" \
  --roll-runtime-artifact finite-agent-runtime-YYYY-MM-DD.N \
  --roll-admin-email operator@example.com \
  --roll-admin-workos-user-id user_operator \
  --roll-project-id project_example
```

The helper plans the cohort first, verifies every selected canonical container
on lat1 before enqueueing anything, and then delegates to Core's existing
Runtime Upgrade operation one Runtime at a time. It stops on the first failure,
timeout, or failed postcondition. Missing compute is a recovery case and fails
closed here. Fleet scope is available only with both `--roll-all` and an
explicit `--roll-canary-project-id`; the canary must finish healthy before the
remaining planned projects are attempted.

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
   `curl -s https://finite.computer/internal/finite-private/v1/health` → 401
   with an invalid-token error (core alive + gated — the limiter path; the
   bare `/internal/finite-private/` prefix 404s, only `/v1/*` routes exist).
   When the Finite Private self-service routes are changed, also verify the
   narrow edge contract: an invalid bearer returns 401 from both
   `/api/core/v1/finite-private/usage` and
   `/api/core/v1/finite-private/usage/reset`, while a neighboring path such as
   `/api/core/v1/admin/runtimes` still reaches the dashboard/404 rather than
   public Core. Then use a canary Finite Private key from a mode-0600 env file
   to GET status and POST reset; never put the raw key in argv or logs.
3. Units are up: `ssh root@64.34.82.77 'systemctl status finite-saas-core
   finite-saas-dashboard'` (`podman-finite-saas-dashboard.service` for the
   container unit name if querying journald).
4. Core still exposes no `source_commit` health payload. The authoritative
   identity check is therefore the exact comparison of `/run/current-system`
   to the prebuilt `SYSTEM` path in step 2; a generation number alone is not
   sufficient.

### ROLLBACK

1. For changes that have **not** activated Finite Private epochs, the fast path
   is `ssh root@64.34.82.77 nixos-rebuild switch --rollback` — boots
   the previous generation (both Core binary and dashboard digest revert
   together). Then reconcile git to match what is running within a day
   (break-glass rule).
2. Deliberate path: prebuild the previous known-good full mono rev using the
   same helper, then copy/switch/verify its exact `SYSTEM` path from lat2 (and,
   for a dashboard-only regression, first revert the digest in
   `modules/dashboard.nix`).
3. Re-run VERIFY.

After the epoch-aware Core has accepted traffic, the previous N-1 closure is
not a safe live binary rollback target. Keep serving or deploy a forward fix on
an epoch-aware closure. If an emergency nevertheless requires N-1, first enter
an explicitly approved Finite Private maintenance window: disable all reserve,
settle, status, and reset callers; prove every `reserved` row has been resolved;
capture a database backup; and document how epoch>0 grants plus any rows written
by N-1 will be reconciled before re-upgrade. Do not re-enable the limiter or
re-upgrade until that reconciliation has been tested on synthetic restored
state. A profile replay check alone is not proof of this boundary.
