# Deploying finite-sites (finitesitesd) to lat1

Since the 2026-07-09 cutover, finitesitesd runs on finite-lat-1
(64.34.82.77), NOT lat2. Config: `infra/nixos/modules/finitesitesd.nix`
(host `finite-lat-1`). It serves `*.finite.chat` / `*.docs.finite.chat` /
`api.finite.chat` as systemd unit `finite-saas-sites.service`
(finitesitesd on 127.0.0.1:8787), fronted by the one host Caddy with the
Cloudflare Origin CA cert. Data `/var/lib/finite-sites` (16 published sites,
npubs intact, restored from lat2 at cutover). Topology:
`infra/nixos/README.md`; box rebuild: [lat1-nixos-reinstall.md](lat1-nixos-reinstall.md).

> **KATA GAP (flagged follow-up):** this module ships `--app-runner none` —
> sites run WITHOUT microVM isolation, so tier-2 tenant apps do not run until
> Kata (or microvm.nix) is ported. lat2 previously ran `--app-runner kata`.
> Tracked as the KATA ISOLATION TODO in `modules/finitesitesd.nix`.

> History: sites previously deployed to lat2 by rsync-source + `cargo build
> --release` on the box + `sudo install`. That box no longer serves sites.
> Do not resurrect the build-on-box flow.

## Deploy flow — nixos-rebuild pinned to a mono rev

`fsite/v*` releases still ship the `fsite` CLI + `finitesitesd` linux binary
([release-cli.md](release-cli.md)), but on lat1 the *daemon* is deployed by
nixos-rebuild (the flake builds `finitesitesd` from the pinned mono rev), not
by copying a release tarball onto the box.

### PRECONDITIONS

- The finitesitesd source change is merged to `main` (you deploy a committed
  rev).
- ssh access to lat1 (`ssh root@64.34.82.77`, key-only) or a nix driver host
  that reaches it as root.
- A fresh Postgres/state safety net exists. FLAG: the Hosted Web Chat recovery
  snapshot does **not** cover `/var/lib/finite-sites`; Sites still needs its own
  service-consistent off-host recovery set and restore proof. Do not treat the
  configured chat Borg repository as Sites protection.

### STEPS

1. Deploy:

   ```sh
   nixos-rebuild switch --target-host root@finite-lat-1 \
     --flake github:finitecomputer/finite-mono/<tag-or-rev>#finite-lat-1
   ```

2. Config-only changes (listen flags, `--app-runner`, sites.env references,
   Caddy vhosts) all live in `infra/nixos/modules/` — never edit units on the
   box. Cert is the Cloudflare Origin CA pair at
   `/etc/finite-saas/certs/finite-chat-origin.{pem,key}` (no ACME; the zone is
   Cloudflare-proxied Full-strict — do not "fix" cert errors by switching to
   ACME).

### VERIFY

1. `ssh root@finite-lat-1 'systemctl status finite-saas-sites'` — active.
2. `curl -fsS https://api.finite.chat/api/v1/healthz`.
3. Load a published site (`https://<something>.finite.chat`) and a
   `*.docs.finite.chat` vhost. (sitesd serves by Host header; there is no
   root `/healthz` on the wildcard vhosts — a 404 at `/` is normal.)
4. TODO: once finitesitesd exposes a `source_commit` health payload
   (finitechat-style contract gate), gate on it here.

### ROLLBACK

```sh
ssh root@finite-lat-1 nixos-rebuild switch --rollback
```

reverts to the previous generation (finitesitesd binary + config together);
or re-run `nixos-rebuild switch --flake ...#finite-lat-1` pinned to the
previous known-good mono rev. Then re-run VERIFY and reconcile git within a
day (break-glass rule).
