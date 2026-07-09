# Deploying finite-saas-core (and dashboard) to lat1

Host map, manifests, and drift notes: `infra/hosts/lat1/README.md` and
`infra/hosts/lat1/k8s/`. Namespace `finite-system`, single-node k3s.

## Current flow — DEPRECATED

On-box podman build + containerd import + imperative `kubectl set image`.
Recorded in full in `infra/hosts/lat1/deploy.md` ("Current flow"); keep only
for emergencies, do not extend. Everything below is the target flow.

## Target flow — CI image by digest

### PRECONDITIONS

- The change is on `main` (the workflow builds from the dispatched ref's
  checkout SHA).
- ssh access to lat1 (`ssh finite-lat-1`; kubeconfig on the box is
  world-readable — any local user is cluster-admin).
- TODO: verify before the first GHCR deploy that lat1's k3s can pull
  `ghcr.io/finitecomputer/finite-saas-core` / `finite-saas-dashboard`
  (package visibility public, or an imagePullSecret exists). Today's live
  images are `localhost/*` — GHCR pull has never been exercised on this box.

### STEPS

1. Dispatch **Service Images** (`.github/workflows/service-images.yml`):
   `image=core` (or `dashboard`), `version=<date-based, e.g. 2026-07-08.1>`.
2. Copy the pinned ref from the workflow summary:
   `ghcr.io/finitecomputer/finite-saas-core:<version>@sha256:...`
   (the summary also records the source commit).
3. Edit `infra/hosts/lat1/k8s/core.yaml` (or `dashboard.yaml`): set the
   container `image:` to the full pinned `name:tag@digest`. Commit — the
   committed digest is the deploy record and the rollback target.
   (This replaces the `:dev` placeholder tags noted in the lat1 README
   appendix item 1.)
4. On lat1:

   ```sh
   kubectl apply -k infra/hosts/lat1/k8s/
   # or, minimal-blast-radius equivalent:
   kubectl -n finite-system set image deployment/finite-saas-core \
     core=ghcr.io/finitecomputer/finite-saas-core:<version>@sha256:...
   ```

### VERIFY

1. `kubectl -n finite-system rollout status deployment/finite-saas-core`
   (readiness probe is `GET /healthz` on :4200 — the rollout only completes
   healthy).
2. Core `/healthz` directly (from the lat1 host, which can reach the service
   CIDR):

   ```sh
   curl -fsS http://10.43.237.180:4200/healthz
   ```

   Note: `10.43.237.180` is the hardcoded ClusterIP also baked into the host
   Caddyfile — see lat1 README "How finite.computer routes". If this curl
   fails after a Service recreation, Caddy's `/internal/finite-private/*`
   route is broken too.
3. Through Caddy: `curl -fsS https://finite.computer/` — this exercises the
   edge → dashboard NodePort path (per the Caddyfile, only
   `/internal/finite-private/*` reaches core through Caddy; everything else,
   including `/healthz`, hits the dashboard).
4. For the dashboard: `kubectl -n finite-system rollout status
   deployment/finite-dashboard` + load the site.
5. TODO: core does not yet expose a `source_commit`-reporting health payload
   (the finitechat-style contract gate); until it does, confirm the running
   image digest matches the commit:
   `kubectl -n finite-system get deploy finite-saas-core -o jsonpath='{.spec.template.spec.containers[0].image}'`.

### ROLLBACK

1. Preferred: re-apply the previous committed digest (git revert the
   manifest bump, `kubectl apply -k infra/hosts/lat1/k8s/`).
2. Fast path: `kubectl -n finite-system rollout undo deployment/finite-saas-core`
   (revision history exists — live revisions were core 38 / dashboard 67 at
   capture). Then reconcile the manifest in git to match what is running,
   within a day (break-glass rule).
3. Re-run VERIFY.
