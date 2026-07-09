# Deploying finite-sites (finitesitesd + fsite) to lat2

Host map: `infra/hosts/lat2/README.md`. Deploy detail (current and target):
`infra/hosts/lat2/deploy.md`. Serves `*.finite.chat` via
`finite-saas-sites.service` (finitesitesd on 127.0.0.1:8787, Kata/cloud-
hypervisor microVMs for tier-2 apps) behind Caddy.

## Current flow — DEPRECATED

rsync source to the box, `cargo build --release` on the box, `sudo install`
to `/usr/local/bin`. Summarized in `infra/hosts/lat2/deploy.md`, full detail
in `finite-sites/docs/deploy-finite-lat-2.md` §3/§5a. No commit provenance —
do not extend.

## Target flow — binaries from an `fsite/v*` release

> `fsite/v*` releases publish `finitesitesd-linux-x86_64.tar.gz` (+ sha256)
> alongside the `fsite` CLI assets specifically for this flow (added
> 2026-07-08, same day this gap was found — the asset first exists on the
> first mono-cut `fsite/v*` release; earlier legacy releases do not have it).

### PRECONDITIONS

- An `fsite/vX.Y.Z` release exists with linux-x86_64 assets for both
  binaries (see gap above) and `compat/matrix.toml` `[field.fsite-cli]` was
  updated at release time ([release-cli.md](release-cli.md)).
- ssh access: `ssh finite-lat-2`.
- A fresh durable backup of `/var/lib/finite-sites` exists — see
  `infra/hosts/lat2/backups.md` (as of capture the newest durable backup was
  2026-06-17; take one first).

### STEPS

1. Download the tagged linux assets and verify their `.sha256`s locally.
2. Copy **binaries only** (never source) to the box, then:

   ```sh
   # on lat2 — keep the .prev rollback copies exactly as today
   ts=$(date -u +%Y%m%dT%H%M%SZ)
   sudo cp /usr/local/bin/finitesitesd /usr/local/bin/finitesitesd.prev-$ts
   sudo cp /usr/local/bin/fsite        /usr/local/bin/fsite.prev-$ts
   sudo install -m 0755 ./finitesitesd ./fsite /usr/local/bin/
   sudo systemctl restart finite-saas-sites
   ```

3. Config changes (units, drop-ins, polkit, sudoers, Caddyfile) deploy from
   `infra/hosts/lat2/systemd/` and `infra/hosts/lat2/caddy/Caddyfile` via
   `sudo install` + `systemctl daemon-reload` / `systemctl reload caddy` —
   never from the sites source checkout.

### Cautions

- **Caddy:** prefer `systemctl reload caddy` over restart. TLS is a
  Cloudflare Origin CA cert pair at `/etc/finite-saas/certs/` (no ACME, no
  API token on the box) — do not "fix" cert errors by switching to ACME; the
  zone is Cloudflare-proxied Full (strict).
- **Kata:** tier-2 apps run as Kata microVMs driven via `sudo nerdctl` +
  containerd, launched by finitesitesd. TODO: verify during the first
  target-flow deploy what restarting `finite-saas-sites` does to running
  Kata app microVMs (orphaned vs. restarted) and record it here.
- `registry.db` is WAL-mode SQLite under `/var/lib/finite-sites` — for
  anything destructive, take the stop-the-world backup first
  (`infra/hosts/lat2/backups.md`).

### VERIFY

1. `finitesitesd --version` reports the released version (0.2.16 was live at
   capture).
2. `curl -fsS https://api.finite.chat/api/v1/healthz`
3. Load a published site (`https://<something>.finite.chat`) and a docs
   vhost.
4. TODO: once finitesitesd exposes a `source_commit` health payload
   (finitechat-style contract gate — the stated bar in
   `infra/hosts/lat2/deploy.md`), gate on it here.

### ROLLBACK

```sh
# on lat2 — .prev copies are the rollback path (pattern in use since
# finitesitesd.prev-20260619T155747Z)
sudo install -m 0755 /usr/local/bin/finitesitesd.prev-<stamp> /usr/local/bin/finitesitesd
sudo install -m 0755 /usr/local/bin/fsite.prev-<stamp>        /usr/local/bin/fsite
sudo systemctl restart finite-saas-sites
```

Then re-run VERIFY, and record the rollback (and why) against the release.
