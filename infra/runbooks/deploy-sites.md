# Deploying finite-sites (finitesitesd) to lat1

Since the 2026-07-09 cutover, finitesitesd runs on finite-lat-1
(64.34.82.77), NOT lat2. Config: `infra/nixos/modules/finitesitesd.nix`
(host `finite-lat-1`). It serves `*.finite.chat` / `*.docs.finite.chat` /
`api.finite.chat` as systemd unit `finite-saas-sites.service`
(finitesitesd on 127.0.0.1:8787), fronted by the one host Caddy with the
Cloudflare Origin CA cert. Data `/var/lib/finite-sites` (16 published sites,
npubs intact, restored from lat2 at cutover). Topology:
`infra/nixos/README.md`. The historical
[2026-07-09 bare-metal transcript](lat1-nixos-reinstall.md) is not current box-
rebuild authority.

> **KATA GAP (flagged follow-up):** this module ships `--app-runner none` —
> sites run WITHOUT microVM isolation, so tier-2 tenant apps do not run until
> Kata (or microvm.nix) is ported. lat2 previously ran `--app-runner kata`.
> Tracked as the KATA ISOLATION TODO in `modules/finitesitesd.nix`.

> History: sites previously deployed to lat2 by rsync-source + `cargo build
> --release` on the box + `sudo install`. That box no longer serves sites.
> Do not resurrect the build-on-box flow.

## Deploy flow — prebuilt immutable mono rev

`fsite/v*` releases still ship the `fsite` CLI + `finitesitesd` linux binary
([release-cli.md](release-cli.md)), but on lat1 the *daemon* is deployed by
nixos-rebuild (the flake builds `finitesitesd` from the pinned mono rev), not
by copying a release tarball onto the box.

### PRECONDITIONS

- The finitesitesd source change is merged to `main` (you deploy a committed
  rev).
- The reviewed revision has a successful `Lat1 NixOS Closure` workflow artifact
  and the deploy operator can SSH to `root@64.34.82.77`. Do not evaluate or
  build the production closure on the Mac, clawland, lat1, or lat2.
- A fresh v3 coordinated recovery snapshot exists and its Borg archive has
  passed the empty-target drill in
  [hosted-web-chat-recovery.md](hosted-web-chat-recovery.md). v1 and v2
  snapshots do **not** cover `/var/lib/finite-sites`; do not treat an older
  configured archive as Sites protection.

### STEPS

1. Build and download the reviewed revision's `lat1-nixos-closure-REV`
   artifact with the shared procedure in
   [deploy-core.md](deploy-core.md#steps). `REV` must be the exact lowercase
   40-hex commit on `origin/main`, not a tag, branch, short hash, or dirty
   tree.

2. Deploy that artifact with:

   ```sh
   just deploy-lat1-closure "$ARTIFACT_DIR"
   ```

   The script validates the manifest, copies the prebuilt file binary cache to
   lat1, activates it in a transient systemd unit, and proves
   `/run/current-system` equals the artifact's exact `SYSTEM` path. It does not
   evaluate or build on lat1 or lat2.

3. Config-only changes (listen flags, `--app-runner`, sites.env references,
   Caddy vhosts) all live in `infra/nixos/modules/` — never edit units on the
   box. Cert is the Cloudflare Origin CA pair at
   `/etc/finite-saas/certs/finite-chat-origin.{pem,key}` (no ACME; the zone is
   Cloudflare-proxied Full-strict — do not "fix" cert errors by switching to
   ACME).

### VERIFY

1. `ssh root@64.34.82.77 'systemctl status finite-saas-sites'` — active.
2. `curl -fsS https://api.finite.chat/api/v1/healthz`.
3. Load a published site (`https://<something>.finite.chat`) and a
   `*.docs.finite.chat` vhost. (sitesd serves by Host header; there is no
   root `/healthz` on the wildcard vhosts — a 404 at `/` is normal.)
4. Probe one real HTML URL and one real asset URL through Cloudflare. Both
   must return `Cache-Control: no-store`; reject any positive `max-age`:

   ```sh
   for url in \
     'https://<published-site>.finite.chat/' \
     'https://<published-site>.finite.chat/<real-asset>.js'
   do
     headers="$(curl -fsSI "$url")"
     grep -Eiq '^cache-control:[[:space:]]*no-store([[:space:]]|$)' <<<"$headers"
     ! grep -Eiq '^cache-control:.*max-age=[1-9]' <<<"$headers"
   done
   ```

   On 2026-07-23 the zone's default four-hour Browser Cache TTL rewrote the
   origin's `public, max-age=0, must-revalidate` asset responses to
   `public, max-age=14400, must-revalidate`. That let an ordinary reload mix
   current HTML with stale JavaScript or CSS. `no-store` is the application
   correctness boundary while URLs remain mutable. Separately set Cloudflare
   **Caching → Configuration → Browser Cache TTL** to **Respect Existing
   Headers** and ensure no Cache Rule overrides browser TTL. Only restore
   validator-based public caching after an edge probe proves those headers are
   preserved; correctness must not depend on that external setting. Browsers
   that already received the old four-hour header can retain that old entry
   until it expires (an edge purge cannot clear a browser cache), so allow that
   one-time rollout window or hard-reload the validation browser once.
5. TODO: once finitesitesd exposes an automatically derived source fingerprint
   (finitechat-style contract gate), gate on it here.

### ROLLBACK

```sh
ssh root@64.34.82.77 nixos-rebuild switch --rollback
```

reverts to the previous generation (finitesitesd binary + config together);
or build/download/deploy the previous known-good rev's exact closure artifact.
Verify `/run/current-system` against the selected rollback path, then re-run
VERIFY and reconcile git within a day (break-glass rule).
