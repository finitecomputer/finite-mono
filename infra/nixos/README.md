# infra/nixos — finite-lat-1 as code

The NixOS definition of the single app server (finite-lat-1, 64.34.82.77).
The root flake's `nixosConfigurations.finite-lat-1` composes the modules here;
`packages.nix` builds every server binary from this workspace.

**LIVE since 2026-07-09.** The cutover is done — lat1 was reinstalled as
NixOS and now runs the whole coupled cluster (Core, dashboard, native
Postgres, chat, sites, search, one Caddy edge). This tree IS lat1's current
config; copying and directly switching to the exact CI-built closure artifact
is the deploy.
The historical cutover and its hard-won gotchas (single-disk/no-mdadm, disks
by-id, WAN-by-MAC) are in `infra/runbooks/lat1-nixos-reinstall.md`; its
destructive procedure is paused and is not current recovery authority. Brain is
served under the WorkOS-protected dashboard origin. The Hosted Web Chat
offsite-health jobs and first archive now pass; its complete empty-target
restore and the complete Agent/host Recovery Set remain unproved. Its snapshot
is deploy/manual-triggered, not periodic; the former 15-minute stop/start timer
was removed because it broke chat streams. A disk mirror remains deferred and
is defense in depth, not a backup.

## Shared Kata Runner host role (one declaration, no drift)

`modules/kata-runner-host.nix` is the single declaration of the Kata Runner
role shared by finite-lat-1 and finite-lat-3. It renders the non-secret,
host-identical Runner environment to `/etc/finite/runner-shared.env` and loads
it BEFORE the operator-managed `/etc/finite/runner.env`, which keeps only
credentials, drain state, and bounded incident overrides (its values still
win). The shared file carries the pinned Runtime artifact, the Kata adapter
settings, and `FC_RUNNER_KATA_STOP_TIMEOUT_SECS=180` — the value operators had
to raise by hand on both hosts after the stock 30s timeout caused two false
upgrade failures and halted a 25-Agent rollout.

Drift rule: Runner-role changes go in the shared module. Host configs declare
only genuine per-host differences through `finite.kataRunnerHost.*`
(`coreUrl`, `runnerId`, `sourceHostId`, `workRoot`, optional
`kataHostAddress`, `maxSandboxes`). `just runner-host-contract` evaluates both
`nixosConfigurations` and fails CI if the rendered shared env or the
module-owned unit shape drifts outside that declared per-host set. Hosts
import `modules/finite-saas-runner.nix` directly before
`modules/kata-runner-host.nix`; routing the base module through the shared
module's own import changes definition merge order and rewrites rendered unit
lines.

## finite-lat-3 storage canary

`nixosConfigurations.finite-lat-3` is the pinned NixOS 26.05 storage-qualified
Runner candidate at `207.188.7.157`. Its host-specific definition is
`hosts/finite-lat-3/`: two exact-size RAID1 arrays, two independently mounted
removable-path ESPs, ext4 project quotas on `/data`, a 64-GiB swapfile with
bounded zswap, stable disk/partition/filesystem identities, and fail-closed
storage health checks. The bootloader wrapper refuses an update unless both
expected FAT ESPs are mounted read-write with their exact PARTUUIDs.

It was installed and storage-qualified on 2026-07-20. The current generation
adds Kata/containerd and a timer-disabled Runner configured for a drained
private-path proof. It is not customer capacity or a Recovery Authority until
the synthetic handoff passes. The authoritative sequence and dated evidence are in
`docs/runs/finite-lat-capacity-and-redundancy.md`. Production lat1 closure
builds use the `Lat1 NixOS Closure` workflow; lat2's services and storage are
unchanged.

## Deploy story

### Bare-metal rebuild (paused; historical transcript follows)

Do not run the original lat2-driven cutover transcript. The helper and commands
that built and drove the install from `finite-lat-2` have been removed in the
hard cut. A future recovery-proved bare-metal procedure must consume a
`lat1-nixos-closure-REV` artifact, prove the complete Recovery Set, and replace
this historical section before any destructive reinstall is considered
repeatable. Until then, start an incident with `infra/runbooks/break-glass.md`
and preserve state.

Do not run Nix evaluation, `nix build`, `nixos-rebuild`, or `nixos-anywhere`
for the production closure on macOS. Nix would inherit `/etc/nix/machines` or
the operator's personal builder settings. The historical transcript below used
lat2 as the x86_64 Linux builder/driver; that path is no longer supported. The
current routine deploy path is the CI-built closure artifact documented in
`infra/runbooks/deploy-core.md`.

### Every deploy after that

The routine deploy path is:

1. Dispatch `.github/workflows/lat1-nixos-closure.yml` for the exact reviewed
   `origin/main` revision. The workflow runs on `depot-ubuntu-24.04` by default,
   builds `nixosConfigurations.finite-lat-1.config.system.build.toplevel` and
   the matching disko script with remote builders disabled, and uploads a file
   binary cache artifact named `lat1-nixos-closure-REV`.
2. Download that artifact and run `just deploy-lat1-closure ARTIFACT_DIR`.
   `scripts/deploy-lat1-closure-cache` validates the manifest, copies the
   prebuilt closure to lat1, advances `/nix/var/nix/profiles/system`, activates
   the exact `SYSTEM` path in a transient systemd unit, and verifies
   `/run/current-system` equals that path. For revisions that include LAT
   journald shipping, the deploy script first runs the values-redacting
   `infra/nixos/scripts/check-lat-monitoring-secrets` preflight on lat1 so a
   missing `/etc/finite/logs-write.env` blocks before activation.

Do not build production closures on the Mac, clawland, lat1, or lat2. There is
no lat2 fallback deploy path. Rollback remains
`ssh root@64.34.82.77 nixos-rebuild switch --rollback`, or the same artifact
workflow for a previous known-good revision followed by `just deploy-lat1-closure`.

## Secrets bootstrap checklist (values NEVER in this repo)

All root-owned, 0600 unless noted. Names only; sources are the old hosts.

| File | Variable names | Value source |
|---|---|---|
| `/etc/finite/core.env` | `FC_CORE_DATABASE_URL` (embeds `POSTGRES_PASSWORD`), `FC_CORE_API_TOKEN`, `FC_CORE_RUNNER_CREDENTIALS_JSON`, one `FC_CORE_RUNNER_CREDENTIAL_TOKEN_*` variable per active Runner credential, `FC_FINITE_PRIVATE_USAGE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `FC_WORKOS_OPERATOR_ORG_ID` | Existing names come from the k8s Secret on old lat1. The checked-in production Kata generation may temporarily retain legacy `FC_CORE_RUNNER_API_TOKEN`; before any second worker starts, replace it with the metadata keyring and separately named Kata/Phala bearer variables documented in `finitecomputer-v2/docs/finite-stack-deployment.md`. Route and worker credentials must be distinct. The usage token pairs with the Tinfoil-sealed `FINITE_USAGE_API_SERVICE_KEY` — **do not rotate at cutover**. Core uses the WorkOS API key only to resolve the verified user record for a validated JWT `sub`. |
| `/etc/finite/metrics-remote-write.env` | `FINITE_METRICS_REMOTE_WRITE_USERNAME`, `FINITE_METRICS_REMOTE_WRITE_PASSWORD` | Install the same root-owned, mode `0600` file independently on finite-lat-1 and finite-lat-3. The username must match the NixOS monitoring receiver's `METRICS_USERNAME`; the password comes from off-host custody and must not be recovered from the Caddy password hash in `/etc/finite/monitoring/caddy.env`. The remote-write URL is fixed in Nix to `https://metrics-ingest.finite.computer/api/v1/write`. This file is read only by Grafana Alloy and must exist before activating a closure that enables Alloy. |
| `/etc/finite/logs-write.env` | `FINITE_LOGS_WRITE_USERNAME`, `FINITE_LOGS_WRITE_PASSWORD` | Install the same root-owned, mode `0600` file independently on finite-lat-1 and finite-lat-3 before activating a closure with LAT journald shipping. The username must match the monitoring receiver's `LOGS_USERNAME`; the password comes from off-host custody and must not be recovered from the Caddy password hash in `/etc/finite/monitoring/caddy.env`. The Loki push URL is fixed in Nix to `https://metrics-ingest.finite.computer/loki/api/v1/push`. This is deliberately separate from the Prometheus remote-write credential. |
| `/etc/finite/runner.env` | only credentials and deliberate overrides: `FC_CORE_RUNNER_API_TOKEN`, `FC_RUNNER_FINITE_PRIVATE_SPECIALIZATION_WORKER_API_KEY`, drain state (see `infra/hosts/lat1/systemd/runner.env.example`); the shared non-secret keys are Nix-rendered to `/etc/finite/runner-shared.env` by `modules/kata-runner-host.nix` | provision the route-scoped Runner credential; copy the dedicated specialization worker client token from its owning host secret without reusing the GLM key |
| `/etc/finite/phala-runner.env` | `FC_CORE_RUNNER_API_TOKEN`, `FC_RUNNER_PHALA_API_KEY`, `FC_RUNNER_FINITE_PRIVATE_SPECIALIZATION_WORKER_API_KEY` | Installed with `scripts/install-phala-canary-credentials` for the ACTIVE one-canary run. The script creates a distinct Core keyring credential named `finite-phala-runner-1`, bound to class `phala` and source host `finite-lat-1-phala-control-1`, and accepts the host-only Phala key through a hidden prompt. Never reuse the Kata token or put either credential in Runtime environment. Non-secret workspace/artifact/runtime facts are pinned in the Nix unit; shared runtime secrets enter through a systemd credential copy. |
| `/etc/finite/identity-operator.env` | `FINITE_IDENTITY_OPERATOR_TOKEN` | Created on lat1 without displaying the value by `scripts/install-identity-authority-credentials`. Systemd reads the same trusted provisioning credential for `finite-identityd`, Kata Runner, Phala Runner, Brain, and Hosted Device; it never enters a browser or Agent Runtime. The replaceable token is not identity data and may be regenerated consistently after host loss. |
| `/etc/finite/identity-sites-notification.env` | `FINITE_IDENTITY_SITES_NOTIFICATION_TOKEN` | Created on lat1 without displaying the value by `scripts/install-identity-sites-notification-credential`. Only `finite-identityd` and `finitesitesd` read this narrow same-host credential. It authorizes Sites publication/access-request mail delivery only; it is not the Identity operator credential and must never enter the dashboard, Hosted Device, Runner, or Agent Runtime. Install it before switching to a system closure that requires the file. |
| `/etc/finite/runtime-secrets.env` | the shared tool-provider names selected by Core's names-only `FC_CORE_RUNTIME_SECRET_REFERENCES_JSON` and listed in `infra/hosts/lat1/systemd/runtime-secrets.env.example` | legacy `../finitecomputer/secrets/shared-provider-keys.env`; values remain host-only, OpenRouter is not selected for the new platform, and specialization credentials stay in their owning service |
| `/etc/finite/dashboard.env` | `FC_CORE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `WORKOS_COOKIE_PASSWORD`, `FC_WORKOS_OPERATOR_ORG_ID`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `GOOGLE_WORKSPACE_CLIENT_ID`, `GOOGLE_WORKSPACE_CLIENT_SECRET` | Existing names come from the k8s Secret on old lat1; provision the same missing operator-org predicate used by Core before rollout |
| `/etc/finite/hosted-web-device.env` | `FINITECHAT_HOSTED_API_TOKEN` | generate for the Hosted Web Device internal service boundary; the service and dashboard read this same server-only value; store it in the team password manager |
| `/etc/finite/brain-authority.env` | `FC_CORE_API_TOKEN` | provision Brain's trusted service credential for the narrow Core account/Agent resolution routes; never expose it to the Product Client |
| `/etc/finite/sites-viewer-session.env` | `FINITE_SITES_VIEWER_SESSION_TOKEN` | generate exactly 32 random bytes as 64 lowercase hex characters (`openssl rand -hex 32`) for the Sites verified-email viewer-session boundary; systemd/Podman read this root:root 0600 file before dropping service privileges; Sites and the dashboard receive the same server-only value; store it in the team password manager |
| `/var/lib/finitecomputer/backups/rsync-net/{id_ed25519,known_hosts,borg-passphrase}` | existing finitecomputer Borg SSH private key, pinned rsync.net host key, and repository passphrase | copy the established root-only credential bundle from an existing finitecomputer host; the off-host passphrase copy already lives in the ignored `../finitecomputer/workspaces/trf/secrets/` tree. Do not generate a parallel credential set or put values in this repo. Verify the destination restriction before claiming append-only protection. |
| `/etc/finite-saas/sites.env` | `RESEND_API_KEY` (+ optional `FINITE_IDENTITY_AUTHORITY`) | migrated from lat2 `/etc/finite-saas/sites.env`; systemd reads the root:root 0600 file before dropping privileges, and Sites, Identity, and Brain reuse the existing send-only Resend credential without copying its value |
| `/etc/finite-saas/certs/finite-chat-origin.pem` (0644) / `.key` (0640 root:caddy) | — | copied from lat2 at cutover (Cloudflare Origin CA pair; host-agnostic, covers the zone) |
| `/etc/finite/litestream-latitude.env` | `LITESTREAM_ACCESS_KEY_ID`, `LITESTREAM_SECRET_ACCESS_KEY` | generate a scoped credential for the `finite-lat-1-litestream` bucket at Latitude.sh object storage (chi region — nearest to lat1); store a copy in the team password manager. If the file is absent, every per-database `finite-litestream-*` replicator unit is condition-skipped (chat and Brain keep serving) and `finite-litestream-health` fails loudly every five minutes until it exists (`infra/runbooks/litestream-chat-replication.md`). |
| `/etc/finite/searxng.env` | `SEARXNG_SECRET` (+ optional `SEARXNG_BASE_URL`, `SEARXNG_LIMITER`) | lat2 `finite-search/searxng/.env` |
| `/etc/finite/firecrawl.env` | `BULL_AUTH_KEY`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB`, `MAX_CPU`, `MAX_RAM` | lat2 `finite-search/firecrawl-upstream/.env` |
| Postgres role password | — | `ALTER ROLE finite WITH PASSWORD '<POSTGRES_PASSWORD>';` before the restore (`modules/postgres.nix` header) |

The machine-readable, values-free file inventory is
`hosts/finite-lat-1/secret-bootstrap-contract.json`. From a reviewed checkout
on the host, validate only existence, file type, mode, and ownership by default:

```sh
sudo scripts/check-lat1-secret-bootstrap
```

After separately authorizing a read of the secret files, add
`--check-env-names` to validate required variable names. The checker discards
values and never prints them. Neither mode proves that an off-host custodian
actually has the value, that the value still works, or that the Postgres role
password matches `FC_CORE_DATABASE_URL`; those require an encrypted custody
record and an isolated restore/authentication drill. Do not add values,
fingerprints, or password-derived hashes to the public contract.
The complete custody and operator-copy gate is
[`../runbooks/lat1-catastrophic-recovery-copy.md`](../runbooks/lat1-catastrophic-recovery-copy.md).

The current monitoring MVP still uses host-local env files rather than SOPS.
Before any manual LAT activation that includes Alloy log shipping, run the
narrow monitoring preflight against the target host:

```sh
ssh root@64.34.82.77 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
ssh root@207.188.7.157 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
```

The helper checks only `/etc/finite/metrics-remote-write.env` and
`/etc/finite/logs-write.env` metadata plus required variable names. It discards
values and prints none.

Finite Brain reads `/etc/finite/identity-operator.env`,
`/etc/finite/brain-authority.env`, and the send-only Resend credential from
`/etc/finite-saas/sites.env`; the Product Client and Agent Runtime never
receive any of those credentials.

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
| 8790 | 127.0.0.1 | Finite Identity Authority | new |
| 8787 | 127.0.0.1 | finitesitesd | lat2 |
| **8788** | 127.0.0.1 | **finitechat-server (moved off 8787** — sitesd owns it here; public URL unchanged) | clawland 8787 |
| 38918 | 127.0.0.1 | Finite Chat Hosted Web Device (dashboard-internal) | new |
| 9100 | 127.0.0.1 | node-exporter | new |
| 2019 | 127.0.0.1 | caddy admin API | lat1/lat2 |
| 14200 | 10.254.3.1 (WireGuard) | private proxy to Core :4200 | lat3 Runner only |
| 18790 | 10.254.3.1 (WireGuard) | private proxy to Identity Authority :8790 | lat3 Runner only |
| dynamic 32768-60999 | 10.254.3.2 (WireGuard) | lat3 Kata Runtime contact/health | lat1 peer only |

Caddy vhost → backend: `finite.computer` → 4200 for
`/internal/finite-private/*` and the exact API-key usage/reset paths under
`/api/core/v1/finite-private/`, else 3000; `chat.finite.computer` → 8788; `api./*.finite.chat` +
`*.docs.finite.chat` → 8787 (Cloudflare Origin CA);
`identity.finite.vip` public identity routes → 8790. Brain has no independent
edge: authenticated `/client` and `/_admin/*` requests go through the dashboard
to loopback :3015, then Brain applies its Nostr authorization.

## Open follow-ups (post-cutover; grep for TODO)

Resolved during the 2026-07-09 cutover: disko device layout (single-disk,
by-id), gateways/resolvers, root ssh key, dashboard image digest. Still open:

- **Non-disruptive recovery cadence + restore proof** (`modules/backups.nix`) —
  the service-consistent Hosted Web Chat snapshot service, rsync.net
  repository, Borg 1.2 selection, established credential paths, and
  stale-health units are defined. Snapshot creation is deploy/manual-triggered;
  no 15-minute timer exists. The 2026-07-18 live inventory observed the
  offsite jobs healthy and a verified first archive. Add a stream-safe cadence
  and complete an empty-target drill before claiming the accepted RPO. A
  destination-enforced append-only upload credential remains recommended
  hardening.
- **Disk mirror** — root + `/data` are single NVMe. The matching Micron and
  Samsung disks contain stale MD metadata from the failed 2026-07-09 install;
  they are not free/untouched spares. The accepted `finite-lat-3` rehearsal
  must prove exact member sizing, release-matched assembly, dual-ESP boot, and
  degraded rebuild before a separately authorized lat1 reprovision.
- **Runner fast-follow** — Kata is the production adapter; Phala must pass the
  same provider-neutral contract before it is enabled.
- **KATA ISOLATION** (`modules/finitesitesd.nix`): sites run
  `--app-runner none` — tier-2 `app` sites lack microVM isolation until Kata
  (or microvm.nix) is ported.
- **firecrawl API** (:3002) down — searxng works; crawl/scrape degraded.
- Dead-man's-switch ping (`modules/monitoring.nix`); finite-search image
  digest pins (`modules/finite-search.nix`).
