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
- The `Lat1 NixOS Closure` workflow can run on a Depot-managed x86_64 Linux
  runner, and the operator can download its artifact with `gh`. The deploy
  machine needs Nix only to copy an already built binary cache to lat1; it must
  not evaluate or build the production closure on the Mac, clawland, lat1, or
  lat2.
- ssh access from the deploy machine to `root@64.34.82.77`.
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

> **Automated path:** build the closure with
> `.github/workflows/lat1-nixos-closure.yml`, download the
> `lat1-nixos-closure-REV` artifact, then run
> `just deploy-lat1-closure <artifact-dir>`. That copies the prebuilt closure
> from the artifact's file binary cache, switches lat1, and verifies the
> running closure by state. The older `just deploy-lat1 REV` path still builds
> on lat2 and exists only as a fallback until this artifact path is live-proven.

To roll a reviewed, healthy existing Runtime cohort after the deployment has
passed its normal verification, use the separate prepare/execute workflow with
an exact artifact id and a real admin identity:

Deploy the reviewed control-plane revision first. Runtime rollout is deliberately
separate: run `scripts/rollout-lat1-runtime-artifact --prepare ...`, review its
concise summary/hash, then run the emitted `--execute-plan-hash ...` command.
The exact commands and evidence path are documented in
[`runtime-image.md`](runtime-image.md).

Preparation verifies every selected canonical container but never enqueues an
upgrade. Execution recomputes the exact reviewed plan, then delegates to Core's
existing Runtime Upgrade operation one Runtime at a time with a just-in-time
provider check and postflight. It stops on the first drift, failure, timeout, or
failed postcondition. Missing compute is a recovery case and fails closed here.
Fleet scope requires both `--roll-all` and an explicit
`--roll-canary-project-id`; that canary must already be healthy on the target.

1. **Core (and any config/module change):** From the reviewed checkout, select
   the full commit, prove it is on `origin/main`, and dispatch the Depot-backed
   closure build:

   ```sh
   set -euo pipefail
   git fetch origin --prune
   REV="$(git rev-parse HEAD)"
   [[ "$REV" =~ ^[0-9a-f]{40}$ ]]
   git merge-base --is-ancestor "$REV" origin/main
   gh workflow run lat1-nixos-closure.yml --ref main -f rev="$REV"
   ```

   `REV` must be exactly 40 lowercase hex characters; do not hand off a tag,
   branch, abbreviation, or dirty working tree. Wait for the workflow to
   complete successfully, then download and inspect the artifact:

   ```sh
   RUN_ID="$(
     gh run list --workflow lat1-nixos-closure.yml --commit "$REV" \
       --json databaseId,conclusion \
       --jq '.[] | select(.conclusion == "success") | .databaseId' \
       | head -1
   )"
   test -n "$RUN_ID"
   ARTIFACT_DIR="target/lat1-nixos-closure-$REV"
   rm -rf "$ARTIFACT_DIR"
   gh run download "$RUN_ID" \
     --name "lat1-nixos-closure-$REV" \
     --dir "$ARTIFACT_DIR"
   python3 -m json.tool "$ARTIFACT_DIR/manifest.json" >/dev/null
   ```

2. Deploy only that artifact. The deploy script validates the manifest, proves
   `REV` is on `origin/main`, takes the pre-deploy recovery snapshot, copies the
   unsigned file binary cache to lat1 with `--no-check-sigs`, installs `SYSTEM`
   as the boot profile, activates it in a transient systemd unit, and asserts
   `/run/current-system` is exactly the artifact's `SYSTEM` path:

   ```sh
   just deploy-lat1-closure "$ARTIFACT_DIR"
   ```

   The script does not evaluate or build Nix derivations. Its local Nix use is
   limited to copying the workflow-produced file binary cache to lat1.

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
4. Core still exposes no build fingerprint in its health payload. The
   authoritative identity check is therefore the exact comparison of
   `/run/current-system` to the prebuilt `SYSTEM` path in step 2; a generation
   number alone is not sufficient.

### ROLLBACK

1. For changes that have **not** activated Finite Private epochs, the fast path
   is `ssh root@64.34.82.77 nixos-rebuild switch --rollback` — boots
   the previous generation (both Core binary and dashboard digest revert
   together). Then reconcile git to match what is running within a day
   (break-glass rule).
2. Deliberate path: build/download the previous known-good full mono rev with
   the same closure-artifact workflow, then copy/switch/verify its exact
   `SYSTEM` path with `just deploy-lat1-closure` (and, for a dashboard-only
   regression, first revert the digest in `modules/dashboard.nix`).
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
