# Deploying to lat1

## Current flow — DEPRECATED (do not extend)

This is what actually produced the workloads running on 2026-07-08. It is
recorded so it can be reproduced in an emergency, and retired.

1. Get the source onto the box. Today that means the rsync'd, non-git working
   tree at `/opt/finite/finitecomputer-v2` (owned by uid 501 — see the README
   appendix; no SHA provenance).
2. Build on the host with podman, tagged with a short SHA or ad-hoc label:
   `podman build -f <core.Dockerfile> -t localhost/finite-saas-core:<tag> .`
   (same pattern for the dashboard with `dashboard.Dockerfile`; the image
   definitions now live in `infra/images/`, adapted for the mono root build
   context — the copies this flow used came from the pre-mono v2 checkout).
3. Import the image into k3s containerd (podman and containerd are separate
   stores; the capture shows the same `localhost/...` tags in both, so images
   are exported from podman and imported into containerd, e.g. `podman save
   ... | k3s ctr images import -`).
4. Point the Deployment at the new tag (`kubectl set image ...` or edit +
   `kubectl apply -f k8s/`). Manifests commit `:dev` tags; production tags
   were always set imperatively — hence the repo/live image drift in the
   README appendix. Live revision counters (dashboard: 67, core: 38) show how
   many times this loop has run.
5. Secrets: created/updated imperatively (`kubectl create secret ... ` seeded
   from `/etc/finite-computer/deploy.env`, then patched directly over time —
   deploy.env is now 5 keys behind the live secret).
6. Runner: build `finite-saas-runner` with cargo in
   `/opt/finite/finitecomputer-v2`, edit `/etc/finite-computer/runner.env`,
   `systemctl daemon-reload` if units changed. The timer re-invokes the new
   binary within 20s; no restart needed.

Why deprecated: nothing above is reproducible from a git ref — the binary and
images come from whatever was on the box.

## Target flow

Minimal change to the same box and mechanisms; no new tooling:

1. CI (GitHub Actions in finite-mono) builds the core and dashboard images at
   a git SHA from the Dockerfiles defined in `infra/images/`, tags them with
   the SHA, and pushes to GHCR.
2. Deploys reference the image **by digest**
   (`ghcr.io/finitecomputer/<image>@sha256:...`) in the manifests under
   `k8s/`, committed to this repo.
3. `kubectl apply -k k8s/` on lat1; k3s pulls from GHCR. No podman, no
   on-host build, no import step.
4. Rollback = re-apply the previous committed digest.

Runner binary follows the same principle via component release tags (see
`infra/README.md`): built by CI, fetched to the host at a pinned version,
replacing the rsync'd working tree.
