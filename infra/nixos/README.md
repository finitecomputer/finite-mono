# NixOS fleet

The root flake composes `nixosConfigurations.*`; `packages.nix` builds server
binaries from this workspace. finite-lat-2 is the app plane; finite-lat-3,
finite-lat-4, and finite-lat-5 provide Kata capacity. finite-lat-1 is retired.
[Sites runs on Fly](../runbooks/deploy-sites.md).

## Deployment

Use the reviewed CI closure artifacts and [Core deployment runbook](../runbooks/deploy-core.md).
Runner deployments use the corresponding `deploy-latN-closure` recipe, first
`--validate-only` and `--prepare`, then explicitly authorized `--activate`.
Run `scripts/finite-status` before and after activation. Do not build on a
production host or use an operator's ambient builder for production closures.
For destructive installation, use [host installation](../runbooks/install-host.md).

Storage identities, disk geometry, dual-ESP guards, and quotas live in each
host's configuration. RAID is availability protection, not an independent backup.
See [recovery](../runbooks/hosted-web-chat-recovery.md) for custody and restore proof.

## Shared Kata Runner host role (one declaration, no drift)

`modules/kata-runner-host.nix` is the single declaration of the live Kata
Runner role shared by finite-lat-3, finite-lat-4, and finite-lat-5. The
module renders the non-secret, host-identical Runner environment to
`/etc/finite/runner-shared.env` and loads it BEFORE the operator-managed
`/etc/finite/runner.env`, which keeps only credentials, drain state, and
bounded incident overrides (its values still win). The shared file carries the
Kata adapter settings and
`FC_RUNNER_KATA_STOP_TIMEOUT_SECS=180`.

Drift rule: Runner-role changes go in the shared module. Host configs declare
only genuine per-host differences through `finite.kataRunnerHost.*`
(`coreUrl`, `runnerId`, `sourceHostId`, `workRoot`, optional
`kataHostAddress`, `maxSandboxes`). `just runner-host-contract` evaluates the
`nixosConfigurations` and fails CI if the rendered shared env or the
module-owned unit shape drifts outside that declared per-host set. Hosts
import `modules/finite-saas-runner.nix` directly before
`modules/kata-runner-host.nix`; routing the base module through the shared
module's own import changes definition merge order and rewrites rendered unit
lines.

## Secrets bootstrap checklist (values NEVER in this repo)

All root-owned, 0600 unless noted. Names only; values belong in the documented off-host custody and root-owned files.

| File | Variable names | Value source |
|---|---|---|
| `/etc/finite/core.env` | `FC_CORE_DATABASE_URL` (embeds `POSTGRES_PASSWORD`), `FC_CORE_API_TOKEN`, `FC_CORE_RUNNER_CREDENTIALS_JSON`, one `FC_CORE_RUNNER_CREDENTIAL_TOKEN_*` variable per active Runner credential, `FC_FINITE_PRIVATE_USAGE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `FC_WORKOS_OPERATOR_ORG_ID` | Use the Core credential keyring and distinct worker bearer variables documented in the Runner contract and Phala runbook. Route and worker credentials must be distinct. The usage token pairs with the Tinfoil-sealed `FINITE_USAGE_API_SERVICE_KEY`. Core uses the WorkOS API key only to resolve the verified user record for a validated JWT `sub`. |
| `/etc/finite/metrics-remote-write.env` | `FINITE_METRICS_REMOTE_WRITE_USERNAME`, `FINITE_METRICS_REMOTE_WRITE_PASSWORD` | Install the same root-owned, mode `0600` file independently on finite-lat-2, finite-lat-3, finite-lat-4, and finite-lat-5. The username must match the NixOS monitoring receiver's `METRICS_USERNAME`; the password comes from off-host custody and must not be recovered from the Caddy password hash in `/etc/finite/monitoring/caddy.env`. The remote-write URL is fixed in Nix to `https://metrics-ingest.finite.computer/api/v1/write`. This file is read only by Grafana Alloy and must exist before activating a closure that enables Alloy. |
| `/etc/finite/logs-write.env` | `FINITE_LOGS_WRITE_USERNAME`, `FINITE_LOGS_WRITE_PASSWORD` | Install the same root-owned, mode `0600` file independently on finite-lat-2, finite-lat-3, finite-lat-4, and finite-lat-5 before activating a closure with LAT journald shipping. The username must match the monitoring receiver's `LOGS_USERNAME`; the password comes from off-host custody and must not be recovered from the Caddy password hash in `/etc/finite/monitoring/caddy.env`. The Loki push URL is fixed in Nix to `https://metrics-ingest.finite.computer/loki/api/v1/push`. This is deliberately separate from the Prometheus remote-write credential. |
| `/etc/finite/runner.env` | only credentials, the promoted Runtime artifact pin, and deliberate overrides: `FC_CORE_RUNNER_API_TOKEN`, `FC_RUNNER_RUNTIME_ARTIFACT_ID` (required; the shared env has no default), drain state (see `infra/hosts/lat1/systemd/runner.env.example`); the shared non-secret keys are Nix-rendered to `/etc/finite/runner-shared.env` by `modules/kata-runner-host.nix` | provision the route-scoped Runner credential |
| `/etc/finite/phala-runner.env` | `FC_CORE_RUNNER_API_TOKEN`, `FC_RUNNER_PHALA_API_KEY`, `FC_RUNNER_RUNTIME_ARTIFACT_ID` (required promotion pin; no Nix default) | Installed with `scripts/install-phala-canary-credentials` for the ACTIVE one-canary run. The script creates a distinct Core keyring credential named `finite-phala-runner-1`, bound to class `phala` and source host `finite-lat-1-phala-control-1`, and accepts the host-only Phala key through a hidden prompt. Never reuse the Kata token or put either credential in Runtime environment. Set the promoted canonical Runtime artifact after credential bootstrap and before starting the worker. Non-secret workspace/runtime facts are pinned in the Nix unit; shared runtime secrets enter through a systemd credential copy. |
| `/etc/finite/identity-operator.env` | `FINITE_IDENTITY_OPERATOR_TOKEN` | Managed-agent Directory provisioning credential, shared only with trusted Runner processes. Install with `scripts/install-identity-authority-credentials`; never expose it to a Runtime. |
| `/etc/finite/runtime-secrets.env` | the shared tool-provider names selected by Core's names-only `FC_CORE_RUNTIME_SECRET_REFERENCES_JSON` and listed in `infra/hosts/lat1/systemd/runtime-secrets.env.example` | legacy `../finitecomputer/secrets/shared-provider-keys.env`; values remain host-only, and OpenRouter is not selected for the new platform |
| `/etc/finite/dashboard.env` | `FC_CORE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `WORKOS_COOKIE_PASSWORD`, `FC_WORKOS_OPERATOR_ORG_ID`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `GOOGLE_WORKSPACE_CLIENT_ID`, `GOOGLE_WORKSPACE_CLIENT_SECRET` | Provision the same operator-org predicate used by Core |
| `/etc/finite/hosted-web-device.env` | `FINITECHAT_HOSTED_API_TOKEN` | generate for the Hosted Web Device internal service boundary; the service and dashboard read this same server-only value; store it in the team password manager |
| `/etc/finite/sites-viewer-session.env` | `FINITE_SITES_VIEWER_SESSION_TOKEN` | generate exactly 32 random bytes as 64 lowercase hex characters (`openssl rand -hex 32`) for the Sites verified-email viewer-session boundary; systemd/Podman read this root:root 0600 file before dropping service privileges; Sites and the dashboard receive the same server-only value; store it in the team password manager |
| `/var/lib/finitecomputer/backups/rsync-net/{id_ed25519,known_hosts,borg-passphrase}` | existing finitecomputer Borg SSH private key, pinned rsync.net host key, and repository passphrase | copy the established root-only credential bundle from an existing finitecomputer host; the off-host passphrase copy already lives in the ignored `../finitecomputer/workspaces/trf/secrets/` tree. Do not generate a parallel credential set or put values in this repo. Verify the destination restriction before claiming append-only protection. |
| `/etc/finite-saas/sites.env` | `RESEND_API_KEY` | systemd reads the root:root 0600 file before dropping privileges, and Identity and Brain reuse the existing send-only Resend credential without copying its value; Sites receives its mail credential through Fly secrets |
| `/etc/finite-saas/certs/finite-chat-origin.pem` (0644) / `.key` (0640 root:caddy) | — | Cloudflare Origin CA pair; host-agnostic, covers the zone |
| `/etc/finite/litestream-latitude.env` | `LITESTREAM_ACCESS_KEY_ID`, `LITESTREAM_SECRET_ACCESS_KEY` | generate a scoped credential for the `finite-lat-2-litestream` bucket at Latitude.sh object storage; store a copy in the team password manager. If the file is absent, every per-database `finite-litestream-*` replicator unit is condition-skipped (chat and Brain keep serving) and `finite-litestream-health` fails loudly every five minutes until it exists (`infra/runbooks/litestream-chat-replication.md`). |
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
[Recovery procedure](../runbooks/hosted-web-chat-recovery.md).

Monitoring uses host-local env files rather than SOPS.
NixOS activation runs the narrow monitoring preflight automatically when Alloy
log shipping is configured. To check earlier, run the same values-redacting
preflight against the target host:

```sh
ssh root@64.34.80.19 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
ssh root@207.188.7.157 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
ssh root@152.236.34.15 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
```

The helper checks only `/etc/finite/metrics-remote-write.env` and
`/etc/finite/logs-write.env` metadata plus required variable names. It discards
values and prints none.

Finite Brain reads the send-only Resend credential from
`/etc/finite-saas/sites.env` for its own invitation mailer; the retired
`identity-operator.env` and `brain-authority.env` loads are gone (the server
no longer calls the Directory or Core). Clients and Agent Runtimes
never receive any service credential.


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

| Port | Bind | What |
|---|---|---|
| 22 | public | sshd (root key-only) |
| 80/443 | public | Caddy — ALL vhosts |
| 3000 | 127.0.0.1 | dashboard (podman, host-net) |
| 3015 | 127.0.0.1 | finite-brain |
| 4200 | 127.0.0.1 | finite-saas-core (nix-built binary) |
| 5432 | 127.0.0.1 | postgres 16 native (`finite_core`) |
| 8080 | 127.0.0.1 | searxng (podman) |
| 8790 | 127.0.0.1 | Finite Identity Directory (full router; operator routes are loopback-only) |
| 8791 | 127.0.0.1 | Finite Identity Directory public router (Caddy proxies this verbatim) |
| **8788** | 127.0.0.1 | **finitechat-server** (public URL unchanged) |
| 38918 | 127.0.0.1 | Finite Chat Hosted Web Device (dashboard-internal) |
| 9100 | 127.0.0.1 | node-exporter |
| 2019 | 127.0.0.1 | caddy admin API |
| 14200 | 10.254.3.1 (WireGuard) | private proxy to Core :4200 |
| 18790 | 10.254.3.1 (WireGuard) | private proxy to Identity Authority :8790 |
| dynamic 32768-60999 | Runner WireGuard address | Kata Runtime contact/health |

Caddy vhost → backend: `finite.computer` -> 4200 for
`/internal/finite-private/*` and the exact API-key usage/reset paths under
`/api/core/v1/finite-private/`, else 3000; `chat.finite.computer` -> 8788;
`identity.finite.vip` public identity routes -> 8791.
[Finite Sites runs on Fly](../runbooks/deploy-sites.md). The finite.chat content
vhosts import the reviewed private `/etc/finite/sites-redirects.caddy` mapping
and return temporary redirects; unmapped content and retired API/Git routes
return 410. The old data directory and recovery archives are retained; there
is no Sites daemon or Sites listener on this host.
`/etc/finite-saas/sites.env` remains an Identity and Brain mail credential input despite
its historical filename; do not remove it with the daemon.
