# Deploying finite-brain (smoke)

> **DEFERRED from the 2026-07-09 lat1 cutover.** finite-brain was intentionally
> NOT migrated; it still runs on ovh-vps-smoke, deployed from the legacy
> `finitecomputer` repo. Its move to lat1 (a `modules/finite-brain.nix` +
> oauth2-proxy already stubbed in `infra/nixos/`) is bundled with the
> auth-integration follow-up and will happen then. Until then everything below
> — smoke host, legacy `just host-deploy` flow — is CURRENT. See
> [lat1-nixos-reinstall.md](lat1-nixos-reinstall.md) "Post-install follow-ups".

finite-brain runs on ovh-vps-smoke (15.204.56.61) as the NixOS-generated
unit `finite-brain-app.service`, listening on :3015, SQLite at
`/var/lib/private/finitebrain/finite-brain.sqlite3`. Host map:
`infra/hosts/smoke/README.md`. Deploy detail (current and target):
`infra/hosts/smoke/deploy.md` — that doc is the flow of record; this runbook
is the operational wrapper.

## Current flow — legacy `just host-deploy` (pointer)

Deploys come from the LEGACY `finitecomputer` repo (deliberately outside
mono): `workspaces/ovh-vps-smoke && just host-deploy` → rsync to
`/etc/nixos` → `nixos-rebuild switch --flake .#ovh-vps-smoke`, with
**clawland (15.204.108.57) as the nix build host**. finite-brain is built by
nix from vendored source tarballs in the legacy repo's `nix/sources/`
(0.1.2-6466fcc live at capture). Full steps: `infra/hosts/smoke/deploy.md`.

### PRECONDITIONS

- Operator checkout of the legacy `finitecomputer` repo with its
  `secrets/workspace.env` (values never in any repo).
- Know the current NixOS generation (`nixos-rebuild list-generations` /
  system-237 at capture) — it is your rollback target.

### VERIFY (after any switch)

```sh
curl -fsS https://brain.smoke.finite.computer/health
curl -fsS https://brain.smoke.finite.computer/client/config.json
ssh root@15.204.56.61 systemctl status finite-brain-app   # no ssh alias yet
```

Route expectations and the fbrain client checks:
`finite-brain/docs/runbooks/smoke-alpha-backup-restore-cutover.md`.

### ROLLBACK

NixOS generations: `nixos-rebuild switch --rollback` (or boot a previous
generation). Data rollback is separate — SQLite procedure in the
smoke-alpha runbook above. **Caution: there is currently no automated backup
of that SQLite on this host** (fix 1 below).

## The two proposed fixes (do these before feature work)

From `infra/hosts/smoke/deploy.md` (both are changes in the legacy repo):

1. **Enable borg backups via `backup.json`.** The offsite-borg module
   (`nix/modules/host-offsite-backup.nix`) already runs on clawland; it is
   gated on `workspaces/<ws>/host/backup.json` `{"enabled": true}`, which
   does not exist for ovh-vps-smoke. Add it, and add
   `/var/lib/private/finitebrain` to the backup paths (defaults do not cover
   the brain state dir). Until then the brain's SQLite — the only copy —
   has no protected copy. TODO: after enabling, drill a borg restore of the
   SQLite before calling this fixed.
2. **Close the :3015 bind/firewall gap.** `FINITE_BRAIN_ADDR` is
   `0.0.0.0:3015` on the public IP; if OVH's network firewall does not block
   external :3015, direct access skips oauth entirely. Either bind
   `127.0.0.1:3015` (and repoint the manual Endpoints /
   cluster.json `endpoint_ip` accordingly) or verify the firewall blocks
   :3015 (and k3s :6443/:10250). TODO: unverified either way at capture —
   test from outside OVH and record the result in
   `infra/hosts/smoke/README.md`.

## The bridge: `fbrain/v*` release consumption

Target (not a replatform): mono releases under the `fbrain/v*` tag and the
legacy nix packaging consumes a **pinned release URL + hash** instead of
hand-vendored tarballs in `nix/sources/` — killing tarball copying, the
`nix/sources/` disk growth (smoke's disk was 82% full at capture), and
making the deployed rev traceable to a public mono tag.

Status and gap:

- `compat/matrix.toml` `[field.fbrain-cli]`: **no public fbrain release
  yet** (champagne test used a PR-branch build). The first
  `fbrain/vX.Y.Z` tag follows [release-cli.md](release-cli.md), including
  the matrix update.
- `fbrain/v*` releases publish `finite-brain-linux-x86_64.tar.gz` (+ sha256)
  — the prebuilt `finite-brain` server binary from `finite-brain-app` — for
  the nix bridge (added 2026-07-08). TODO: adapt the legacy nix packaging to
  consume this prebuilt binary (or keep source-tarball builds if NixOS purity
  is preferred — decide when the bridge lands; the release also carries the
  git tag, so a source build can pin against it either way).
