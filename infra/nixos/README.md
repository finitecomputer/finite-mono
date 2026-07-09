# infra/nixos — finite-lat-1 as code

The NixOS definition of the single app server (finite-lat-1, 64.34.82.77).
The root flake's `nixosConfigurations.finite-lat-1` composes the modules here;
`packages.nix` builds every server binary from this workspace. **Executing
the cutover follows `finite-fable/single-server-plan.md` Phase 2 —
scheduled, supervised, backups restore-verified FIRST.** This tree is the
migration artifact; review it like code, because it is.

## Deploy story

### One-time bootstrap (Phase 2 step 8 — wipes the box)

```sh
# From a machine with nix (or the lat2 runner). Verify disko device names
# against the rescue env FIRST (hosts/finite-lat-1/disko.nix TODO), and put
# operator ssh keys in hosts/finite-lat-1/default.nix.
nix run github:nix-community/nixos-anywhere -- \
  --flake .#finite-lat-1 root@64.34.82.77
```

Then complete the secrets checklist below and restore data (Postgres dump,
sites tar, chat sqlite, brain sqlite — plan Phase 2 steps 7/9).

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
| `/etc/finite/core.env` | `FC_CORE_DATABASE_URL` (embeds `POSTGRES_PASSWORD`), `FC_CORE_API_TOKEN`, `FC_FINITE_PRIVATE_USAGE_API_TOKEN` | k8s Secret `finite-computer-secrets` on old lat1. The usage token pairs with the Tinfoil-sealed `FINITE_USAGE_API_SERVICE_KEY` — **do not rotate at cutover** |
| `/etc/finite/runner.env` | the 22 `FC_RUNNER_*`/`FC_CORE_*`/`PHALA_CLOUD_API_KEY` names in `infra/hosts/lat1/systemd/runner.env.example` | old lat1 `/etc/finite-computer/runner.env`; **edit**: `FC_CORE_URL=http://127.0.0.1:4200`, `FC_RUNNER_WORK_ROOT=/var/lib/finite-saas-runner`, `FC_RUNNER_PHALA_BIN` → wherever the phala CLI lands (see module TODO) |
| `/etc/finite/dashboard.env` | `FC_CORE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `WORKOS_COOKIE_PASSWORD`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `GOOGLE_WORKSPACE_CLIENT_ID`, `GOOGLE_WORKSPACE_CLIENT_SECRET` (+ optional `FC_RELAY_ADMIN_TOKEN`, `FC_RELAY_HOST_ENDPOINTS_JSON`) | k8s Secret `finite-computer-secrets` on old lat1 |
| `/etc/finite-saas/sites.env` (0640) | `RESEND_API_KEY` (+ optional `FINITE_IDENTITY_AUTHORITY`) | lat2 `/etc/finite-saas/sites.env` |
| `/etc/finite-saas/certs/finite-chat-origin.pem` (0644) / `.key` (0640 root:caddy) | — | copied from lat2 at cutover (Cloudflare Origin CA pair; host-agnostic, covers the zone) |
| `/etc/finite/oauth2-proxy.env` | `OAUTH2_PROXY_CLIENT_ID`, `OAUTH2_PROXY_CLIENT_SECRET`, `OAUTH2_PROXY_COOKIE_SECRET` | Google OAuth client from smoke's `fc-auth` k8s Secret; generate a fresh cookie secret |
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
| 3000 | 127.0.0.1 | dashboard (podman, host-net) | lat1 k3s NodePort 30080 |
| 3002 | 127.0.0.1 | firecrawl api (podman) | lat2 |
| 3015 | 127.0.0.1 | finite-brain | smoke (bound 0.0.0.0 there — fixed) |
| 4180 | 127.0.0.1 | oauth2-proxy | smoke (in-cluster) |
| 4200 | 127.0.0.1 | finite-saas-core | lat1 k3s ClusterIP |
| 5432 | 127.0.0.1 | postgres 16 (`finite_core`) | lat1 k3s StatefulSet |
| 8080 | 127.0.0.1 | searxng (podman) | lat2 |
| 8787 | 127.0.0.1 | finitesitesd | lat2 |
| **8788** | 127.0.0.1 | **finitechat-server (moved off 8787** — sitesd owns it here; public URL unchanged) | clawland 8787 |
| 9100 | 127.0.0.1 | node-exporter | new |
| 2019 | 127.0.0.1 | caddy admin API | lat1/lat2 |

Caddy vhost → backend: `finite.computer` → 4200 (`/internal/finite-private/*`)
else 3000; `chat.finite.computer` → 8788; `api./*.finite.chat` +
`*.docs.finite.chat` → 8787 (Cloudflare Origin CA); `brain.smoke.finite.computer`
→ oauth2-proxy(4180)-gated 3015 (only `/health` is open — `/_admin` is now
gated too, unlike smoke).

## Known gaps (grep for TODO)

- **KATA ISOLATION TODO** (`modules/finitesitesd.nix`): sites run
  `--app-runner none` — tier-2 apps don't run until Kata (or microvm.nix) is
  ported. Explicit, tracked, must not block cutover.
- Dashboard image digest (`modules/dashboard.nix`) — pin after first CI build.
- `phala` CLI not nix-packaged (`modules/finite-saas-runner.nix`).
- disko device names + IPv4/IPv6 gateway + resolvers + root ssh keys
  (`hosts/finite-lat-1/`).
- Borg offsite target (`modules/backups.nix`).
- Dead-man's-switch ping (`modules/monitoring.nix`).
- finite-search images digest-pins (`modules/finite-search.nix`).
