# infra/nixos — finite-lat-1 as code

The NixOS definition of the single app server (finite-lat-1, 64.34.82.77).
The root flake's `nixosConfigurations.finite-lat-1` composes the modules here;
`packages.nix` builds every server binary from this workspace.

**LIVE since 2026-07-09.** The cutover is done — lat1 was reinstalled as
NixOS and now runs the whole coupled cluster (Core, dashboard, native
Postgres, chat, sites, search, one Caddy edge). This tree IS lat1's current
config; `nixos-rebuild switch --flake ...#finite-lat-1` is the deploy.
Rebuild/recover procedure + the hard-won gotchas (single-disk/no-mdadm, disks
by-id, WAN-by-MAC) are in `infra/runbooks/lat1-nixos-reinstall.md`. Brain is
served under the WorkOS-protected dashboard origin; a disk mirror and proven
offsite backups remain deferred.

## Deploy story

### Rebuild from bare metal (wipes the box)

Full procedure — install, rescue-mode recovery, secrets, data restore, DNS
ordering — is in `infra/runbooks/lat1-nixos-reinstall.md`. In short:

```sh
nix build .#nixosConfigurations.finite-lat-1.config.system.build.toplevel  # gate: build before you wipe
nix run github:nix-community/nixos-anywhere -- \
  --flake .#finite-lat-1 --target-host root@64.34.82.77 --phases kexec,disko,install
```

Then the secrets checklist below + the data restore in the runbook.

### Every deploy after that

```sh
nixos-rebuild switch --target-host root@finite-lat-1 \
  --flake github:finitecomputer/finite-mono/<tag-or-rev>#finite-lat-1
```

Deploying a release IS pinning the flake: the rev that tagged the binaries is
the rev the host runs. Rollback: `nixos-rebuild --rollback` on the host, or
pin the previous rev. Validation without a linux builder:

```sh
nix flake show
nix eval .#nixosConfigurations.finite-lat-1.config.system.build.toplevel.drvPath
nix eval .#packages.x86_64-linux.finite-saas-core.drvPath
```

## Secrets bootstrap checklist (values NEVER in this repo)

All root-owned, 0600 unless noted. Names only; sources are the old hosts.

| File | Variable names | Value source |
|---|---|---|
| `/etc/finite/core.env` | `FC_CORE_DATABASE_URL` (embeds `POSTGRES_PASSWORD`), `FC_CORE_API_TOKEN`, `FC_CORE_RUNNER_API_TOKEN`, `FC_FINITE_PRIVATE_USAGE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `FC_WORKOS_OPERATOR_ORG_ID` | Existing names come from the k8s Secret on old lat1; provision the missing Runner credential and operator-org predicate before rollout. Route credentials must be distinct. The usage token pairs with the Tinfoil-sealed `FINITE_USAGE_API_SERVICE_KEY` — **do not rotate at cutover**. Core uses the WorkOS API key only to resolve the verified user record for a validated JWT `sub`. |
| `/etc/finite/runner.env` | the generic Core, artifact, Kata, endpoint, and secret-reference names in `infra/hosts/lat1/systemd/runner.env.example` | provision the route-scoped Runner credential; select `kata`, the promoted mono runtime artifact, loopback Core, and the production Sites/Brain endpoints |
| `/etc/finite/runtime-secrets.env` | the optional shared tool-provider names in `infra/hosts/lat1/systemd/runtime-secrets.env.example` | legacy `../finitecomputer/secrets/shared-provider-keys.env`; values remain host-only and specialization credentials stay in their owning service |
| `/etc/finite/dashboard.env` | `FC_CORE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `WORKOS_COOKIE_PASSWORD`, `FC_WORKOS_OPERATOR_ORG_ID`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `GOOGLE_WORKSPACE_CLIENT_ID`, `GOOGLE_WORKSPACE_CLIENT_SECRET` (+ optional `FC_RELAY_ADMIN_TOKEN`, `FC_RELAY_HOST_ENDPOINTS_JSON`) | Existing names come from the k8s Secret on old lat1; provision the same missing operator-org predicate used by Core before rollout |
| `/etc/finite/hosted-web-device.env` | `FINITECHAT_HOSTED_API_TOKEN` | generate for the Hosted Web Device internal service boundary; the service and dashboard read this same server-only value; store it in the team password manager |
| `/etc/finite/sites-viewer-session.env` | `FINITE_SITES_VIEWER_SESSION_TOKEN` | generate exactly 32 random bytes as 64 lowercase hex characters (`openssl rand -hex 32`) for the Sites verified-email viewer-session boundary; systemd/Podman read this root:root 0600 file before dropping service privileges; Sites and the dashboard receive the same server-only value; store it in the team password manager |
| `/etc/finite-saas/sites.env` (0640) | `RESEND_API_KEY` (+ optional `FINITE_IDENTITY_AUTHORITY`) | migrated from lat2 `/etc/finite-saas/sites.env` |
| `/etc/finite-saas/certs/finite-chat-origin.pem` (0644) / `.key` (0640 root:caddy) | — | copied from lat2 at cutover (Cloudflare Origin CA pair; host-agnostic, covers the zone) |
| `/etc/finite/searxng.env` | `SEARXNG_SECRET` (+ optional `SEARXNG_BASE_URL`, `SEARXNG_LIMITER`) | lat2 `finite-search/searxng/.env` |
| `/etc/finite/firecrawl.env` | `BULL_AUTH_KEY`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB`, `MAX_CPU`, `MAX_RAM` | lat2 `finite-search/firecrawl-upstream/.env` |
| `/etc/finite/borg.env` + `/etc/finite/borg_ed25519` | `BORG_PASSPHRASE`; ssh key | generated at bootstrap; passphrase ALSO goes in the team password manager |
| Postgres role password | — | `ALTER ROLE finite WITH PASSWORD '<POSTGRES_PASSWORD>';` before the restore (`modules/postgres.nix` header) |

finite-brain has **no** secret env (plain-config `Environment=` lines only,
per the smoke capture).

## Port map (consolidated box)

| Port | Bind | What | Was |
|---|---|---|---|
| 22 | public | sshd (root key-only) | lat1 |
| 80/443 | public | Caddy — ALL vhosts | lat1 + lat2 + clawland + smoke edges |
| 3000 | 127.0.0.1 | dashboard (podman, host-net) | was lat1 k3s NodePort 30080 |
| 3002 | 127.0.0.1 | firecrawl api (podman) | lat2 |
| 3015 | 127.0.0.1 | finite-brain | smoke (previously public-bound there) |
| 4200 | 127.0.0.1 | finite-saas-core (nix-built binary) | was lat1 k3s ClusterIP |
| 5432 | 127.0.0.1 | postgres 16 native (`finite_core`) | was lat1 k3s StatefulSet |
| 8080 | 127.0.0.1 | searxng (podman) | lat2 |
| 8787 | 127.0.0.1 | finitesitesd | lat2 |
| **8788** | 127.0.0.1 | **finitechat-server (moved off 8787** — sitesd owns it here; public URL unchanged) | clawland 8787 |
| 38918 | 127.0.0.1 | Finite Chat Hosted Web Device (dashboard-internal) | new |
| 9100 | 127.0.0.1 | node-exporter | new |
| 2019 | 127.0.0.1 | caddy admin API | lat1/lat2 |

Caddy vhost → backend: `finite.computer` → 4200 (`/internal/finite-private/*`)
else 3000; `chat.finite.computer` → 8788; `api./*.finite.chat` +
`*.docs.finite.chat` → 8787 (Cloudflare Origin CA). Brain has no independent
edge: authenticated `/client` and `/_admin/*` requests go through the dashboard
to loopback :3015, then Brain applies its Nostr authorization.

## Open follow-ups (post-cutover; grep for TODO)

Resolved during the 2026-07-09 cutover: disko device layout (single-disk,
by-id), gateways/resolvers, root ssh key, dashboard image digest. Still open:

- **Offsite borg backups** (`modules/backups.nix`) — target undecided; this
  is the current redundancy gap while root is single-disk. Highest priority.
- **Disk mirror** — root + /data are single NVMe; two spare NVMes are free
  for a ZFS/mdadm mirror (the mdadm RAID1 bug is why we went single-disk).
- **Runner fast-follow** — Kata is the production adapter; Phala must pass the
  same provider-neutral contract before it is enabled.
- **KATA ISOLATION** (`modules/finitesitesd.nix`): sites run
  `--app-runner none` — tier-2 `app` sites lack microVM isolation until Kata
  (or microvm.nix) is ported.
- **firecrawl API** (:3002) down — searxng works; crawl/scrape degraded.
- Dead-man's-switch ping (`modules/monitoring.nix`); finite-search image
  digest pins (`modules/finite-search.nix`).
