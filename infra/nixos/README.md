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
offsite backups remain deferred. The Hosted Web Chat offsite destination is
configured, but credentials, deployment, first archive, and restore evidence
are still outstanding.

## Deploy story

### Rebuild from bare metal (wipes the box)

Full procedure — install, rescue-mode recovery, secrets, data restore, DNS
ordering — is in `infra/runbooks/lat1-nixos-reinstall.md`. In short:

```sh
just nixos-build-lat1  # explicit x86_64 build on finite-lat-2; gate before you wipe
nix run github:nix-community/nixos-anywhere -- \
  --flake .#finite-lat-1 --target-host root@64.34.82.77 --phases kexec,disko,install
```

Then the secrets checklist below + the data restore in the runbook.

Do not run the system build through a plain local `nix build` on macOS. Nix
would inherit `/etc/nix/machines` or the operator's personal builder settings.
The root recipe pins `ssh-ng://finite-lat-2`, verifies the remote store, archives
the dirty or clean flake into it, and evaluates/builds on lat2 itself. It never
consults the Mac's builder scheduler. `FINITE_NIX_X86_BUILDER` is available
only as an explicit replacement-builder override.

### Every deploy after that

`finite-lat-2` is the required x86_64 build host for finite-mono production
closures. **Do not use clawland and do not build on finite-lat-1.** From macOS,
run `just nixos-build-lat1` first; that recipe pins `finite-lat-2` and refuses
to inherit a personal or stale builder choice. The switch must deploy the same
reviewed tag or revision that was built.

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
| `/etc/finite/core.env` | `FC_CORE_DATABASE_URL` (embeds `POSTGRES_PASSWORD`), `FC_CORE_API_TOKEN`, `FC_CORE_RUNNER_CREDENTIALS_JSON`, one `FC_CORE_RUNNER_CREDENTIAL_TOKEN_*` variable per active Runner credential, `FC_FINITE_PRIVATE_USAGE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `FC_WORKOS_OPERATOR_ORG_ID` | Existing names come from the k8s Secret on old lat1. The checked-in production Kata generation may temporarily retain legacy `FC_CORE_RUNNER_API_TOKEN`; before any second worker starts, replace it with the metadata keyring and separately named Kata/Phala bearer variables documented in `finitecomputer-v2/docs/finite-stack-deployment.md`. Route and worker credentials must be distinct. The usage token pairs with the Tinfoil-sealed `FINITE_USAGE_API_SERVICE_KEY` — **do not rotate at cutover**. Core uses the WorkOS API key only to resolve the verified user record for a validated JWT `sub`. |
| `/etc/finite/runner.env` | the generic Core, artifact, Kata, endpoint, and secret-reference names in `infra/hosts/lat1/systemd/runner.env.example` | provision the route-scoped Runner credential; select `kata`, the promoted mono runtime artifact, loopback Core, and the production Sites/Brain endpoints |
| `/etc/finite/phala-runner.env` | `FC_CORE_RUNNER_API_TOKEN`, `FC_RUNNER_PHALA_API_KEY`, `FC_RUNNER_RUNTIME_ARTIFACT_ID` (+ the bounded runtime env/secret-reference names in `infra/hosts/lat1/systemd/phala-runner.env.example`) | **dark/not installed by this definition**; separately authorize a Core keyring credential named `finite-phala-runner-1`, bound to class `phala` and source host `finite-lat-1-phala-control-1`, plus a host-only Phala HTTPS API key; never reuse the Kata token or put either credential in Runtime environment |
| `/etc/finite/runtime-secrets.env` | the optional shared tool-provider names in `infra/hosts/lat1/systemd/runtime-secrets.env.example` | legacy `../finitecomputer/secrets/shared-provider-keys.env`; values remain host-only and specialization credentials stay in their owning service |
| `/etc/finite/dashboard.env` | `FC_CORE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `WORKOS_COOKIE_PASSWORD`, `FC_WORKOS_OPERATOR_ORG_ID`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `GOOGLE_WORKSPACE_CLIENT_ID`, `GOOGLE_WORKSPACE_CLIENT_SECRET` (+ optional `FC_RELAY_ADMIN_TOKEN`, `FC_RELAY_HOST_ENDPOINTS_JSON`) | Existing names come from the k8s Secret on old lat1; provision the same missing operator-org predicate used by Core before rollout |
| `/etc/finite/hosted-web-device.env` | `FINITECHAT_HOSTED_API_TOKEN` | generate for the Hosted Web Device internal service boundary; the service and dashboard read this same server-only value; store it in the team password manager |
| `/etc/finite/sites-viewer-session.env` | `FINITE_SITES_VIEWER_SESSION_TOKEN` | generate exactly 32 random bytes as 64 lowercase hex characters (`openssl rand -hex 32`) for the Sites verified-email viewer-session boundary; systemd/Podman read this root:root 0600 file before dropping service privileges; Sites and the dashboard receive the same server-only value; store it in the team password manager |
| `/var/lib/finitecomputer/backups/rsync-net/{id_ed25519,known_hosts,borg-passphrase}` | existing finitecomputer Borg SSH private key, pinned rsync.net host key, and repository passphrase | copy the established root-only credential bundle from an existing finitecomputer host; the off-host passphrase copy already lives in the ignored `../finitecomputer/workspaces/trf/secrets/` tree. Do not generate a parallel credential set or put values in this repo. Verify the destination restriction before claiming append-only protection. |
| `/etc/finite-saas/sites.env` (0640) | `RESEND_API_KEY` (+ optional `FINITE_IDENTITY_AUTHORITY`) | migrated from lat2 `/etc/finite-saas/sites.env` |
| `/etc/finite-saas/certs/finite-chat-origin.pem` (0644) / `.key` (0640 root:caddy) | — | copied from lat2 at cutover (Cloudflare Origin CA pair; host-agnostic, covers the zone) |
| `/etc/finite/searxng.env` | `SEARXNG_SECRET` (+ optional `SEARXNG_BASE_URL`, `SEARXNG_LIMITER`) | lat2 `finite-search/searxng/.env` |
| `/etc/finite/firecrawl.env` | `BULL_AUTH_KEY`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB`, `MAX_CPU`, `MAX_RAM` | lat2 `finite-search/firecrawl-upstream/.env` |
| Postgres role password | — | `ALTER ROLE finite WITH PASSWORD '<POSTGRES_PASSWORD>';` before the restore (`modules/postgres.nix` header) |

finite-brain has **no** secret env (plain-config `Environment=` lines only,
per the smoke capture).

## Google Workspace OAuth production setup

The dashboard connection flow uses one operator-managed Google OAuth client;
users connect it from their machine's **Connections** page. The live credential
must be an OAuth 2.0 Client ID with application type **Web application**. In
Google Cloud Console, its Authorized redirect URI must be exactly:

```text
https://finite.computer/google-workspace/callback
```

That is a separate callback from WorkOS' `/callback`; do not substitute one
for the other or add a trailing slash. The server performs the code exchange,
so this flow does not require a browser-side Google secret.

Before enabling the connection:

1. Configure the OAuth consent screen for the intended canary accounts. Use
   **Internal** when the project and every user belong to the same Google
   Workspace organization. Otherwise keep the app in **Testing** and add each
   participating account as a test user until the app's publication and
   verification work is deliberately taken on.
2. Enable the Gmail, Google Calendar, Google Drive, Google Sheets, Google Docs,
   People, and Google Apps Script APIs in that project.
3. Configure the consent screen with the exact checked-in scope contract in
   `finite-skills/skills/productivity/google-workspace-finite/references/google-workspace-scopes.json`.
   This includes the OpenID identity scopes used to bind the connected email;
   omitting an API or requested scope makes the dashboard reject the grant.
4. Put only the corresponding values in `/etc/finite/dashboard.env`, under
   the names `GOOGLE_WORKSPACE_CLIENT_ID` and
   `GOOGLE_WORKSPACE_CLIENT_SECRET`. `WORKOS_COOKIE_PASSWORD` is also required
   there to seal the short-lived, user-bound OAuth state. Never copy those
   values into this repository, a command transcript, or logs.
5. Keep the checked-in `FC_DASHBOARD_BASE_URL` and
   `NEXT_PUBLIC_WORKOS_REDIRECT_URI` origins (or an explicit
   `FC_DASHBOARD_PUBLIC_URL` override) pointed at the production dashboard.
   Browser-facing OAuth redirects must use that configured origin rather than
   the dashboard container's loopback request URL.

Acceptance is not a configuration inspection or a callback-only probe. From
one real, authorized production account, click **Connect**, complete Google's
consent, return to Connections with the connected Google email visible, and
then perform one real operation through the agent whose API and permission are
inside the granted scope (for example, a Drive search or Calendar list). Keep
that final operation read-only unless the tester explicitly intends a write.

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

- **Offsite Borg activation + restore proof** (`modules/backups.nix`) — the
  15-minute service-consistent Hosted Web Chat snapshot, rsync.net repository,
  Borg 1.2 selection, established finitecomputer credential paths, and
  stale-health units are defined. Copying the existing credential bundle and
  verifying the destination-side append-only restriction remain operator work;
  no deployment/archive or empty-target drill is claimed by repository config.
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
