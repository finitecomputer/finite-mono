# Deploying finite-brain to ovh-vps-smoke

Status: the deploy mechanism lives in the LEGACY `finitecomputer` repo
(deliberately outside finite-mono). This doc is the bridge: it records the
flow exactly as evidenced on the host (2026-07-08 capture), then the target
state where mono owns the artifact and the legacy repo only consumes it.

## Current: legacy `just host-deploy`

Run from an operator machine with a checkout of the legacy `finitecomputer`
repo (a `secrets/workspace.env` there defines `HOST_SSH_HOST` etc. — values
never in any repo):

1. `cd workspaces/ovh-vps-smoke && just host-deploy`, which runs:
2. `scripts/host_sync_up.sh` — rsync the repo to `root@15.204.56.61:/etc/nixos`,
   `git commit` a "sync" commit on the host (the on-host repo is local-only,
   no remote), and write the real operator-side rev to
   `/etc/nixos/.fc-source-rev` (was `d954500eca4af6e197c1254058631dce4944b67f`
   at capture).
3. `scripts/host_deploy.sh` — ssh to the host:
   `cd /etc/nixos && nixos-rebuild switch --flake .#ovh-vps-smoke`, optionally
   with `--build-host $HOST_BUILD_SSH_HOST`. **clawland-ovh (15.204.108.57) is
   the build host** — it holds a freshly built `nixos-system-ovh-vps-smoke`
   closure at `/root/result` (Jul 7 18:55 at capture).
4. `host_import_images.sh`, restart of cluster deployments,
   `host_runtime_health.sh`.

Each switch creates a NixOS generation (system-237 current at capture; four
switches on Jul 8 alone: 01:02, 14:19, 15:57, 16:31) and restarts
`finite-brain-app.service` when its store path changes.

What the switch actually installs, for finite-brain:

- `nix/finite-brain.nix` — `rustPlatform.buildRustPackage`, pname
  `finite-brain`, version `0.1.2-6466fcc`,
  `finiteBrainRev = 6466fcca389b1897771fa0a7c1cc5c6516e1d467`,
  `finiteNostrRev = fefd22b3f3c39481225a28000bba0b2b9354d1ce`,
  `src = ./sources/finitebrain-6466fcc.tar.gz` +
  `cargoLock = ./sources/finitebrain-6466fcc-Cargo.lock` — i.e. **vendored
  source tarballs checked into the legacy repo's `nix/sources/`** (~27 prior
  tarballs there, Jun 30–Jul 8 cadence). Builds
  `-p finite-brain-app -p finite-brain-cli`; installs `bin/finite-brain` and
  `bin/fbrain`.
- `nix/modules/host-agent-cluster.nix` — generates
  `finite-brain-app.service` (see the captured copy in this directory) and
  renders the k3s Addon manifests (`fc-finite-brain`: IngressRoute +
  selector-less Service + manual Endpoints → 15.204.56.61:3015), gated by
  `workspaces/ovh-vps-smoke/agent-cluster/cluster.json` `.finite_brain`.

Rollback: NixOS generations (`nixos-rebuild switch --rollback` or boot a
previous generation). Data rollback is separate — see
`finite-brain/docs/runbooks/smoke-alpha-backup-restore-cutover.md` in mono for
the SQLite backup/restore procedure.

## Target (bridge, not replatform)

Same box, same NixOS mechanism. The change is where the artifact comes from:

1. **mono releases fbrain / finite-brain-app binaries** under the `fbrain/v*`
   component tag (see `infra/README.md` deploy principles — binaries ship from
   release tags).
2. **The legacy nix packaging consumes a released tarball** (pinned URL +
   hash in `nix/finite-brain.nix`) instead of hand-vendored source tarballs in
   `nix/sources/`. This kills the tarball-copying step, stops `nix/sources/`
   growth, and makes the deployed rev traceable to a public mono tag.
3. **Full ownership moves to mono only when the legacy fleet is retired.**
   Until then the legacy repo remains the deploy mechanism of record for this
   host, and this directory is documentation, not executable config.

## Two immediate fixes to propose (in the legacy repo)

1. **Enable backups for this host.** The offsite-borg module
   (`nix/modules/host-offsite-backup.nix`: `fc-offsite-backup` oneshot +
   daily timer → `scripts/host_borg_backup.sh`, borg over ssh to an
   rsync.net-style repo, 7d/4w/6m retention) already exists and runs on
   clawland. It is gated on `workspaces/<ws>/host/backup.json`
   `{"enabled": true}` — add `backup.json` to
   `workspaces/ovh-vps-smoke/host/` and add `/var/lib/private/finitebrain` to
   the backup paths (defaults cover k3s storage + control-plane, not the brain
   state dir). Today the brain's SQLite has NO protected copy.
2. **Close the oauth-bypass window on :3015.** Either set
   `FINITE_BRAIN_ADDR=127.0.0.1:3015` (and point the manual Endpoints object
   at a reachable host address for the cluster — note the current path routes
   via the public IP, so this needs the Endpoints/cluster.json `endpoint_ip`
   adjusted accordingly, e.g. the cni0 host address) or verify the OVH
   network firewall blocks external :3015 (and :6443/:10250) and record the
   result here.
