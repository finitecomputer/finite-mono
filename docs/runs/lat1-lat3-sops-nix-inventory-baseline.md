# lat1/lat3 sops-nix Inventory Baseline

Status: VALUES-FREE BASELINE

Opened: 2026-08-12

Related plan: `docs/runs/lat1-lat3-sops-nix-phased-migration.md`.

## Scope

This baseline records the source-derived production secret inventory needed
before adding the `sops-nix` foundation. It does not read live secret files,
connect to production hosts, evaluate production NixOS closures on this Darwin
workstation, or implement SOPS plumbing.

The static scan reviewed likely consumers in `infra/nixos`, `infra/hosts`,
`scripts`, and `docs` using the pattern from the phased migration plan. Live
production classification below is based on the NixOS host imports and module
source, not a rendered closure from finite-lat-2. The finite-lat-2 rendered eval
remains the required proof before switching any consumer to SOPS.

## Evaluation Status

- Local machine: Darwin arm64. Production Nix evaluation/builds are documented
  as finite-lat-2 work, so they were not run locally.
- No production SSH was used.
- No secret values, hashes, or fingerprints were read or recorded.
- `infra/nixos/hosts/finite-lat-1/secret-bootstrap-contract.json` remains the
  current values-free lat1 contract.
- A values-free lat3 contract does not exist yet.

## Live lat1 Secret Inputs

| Logical name | Current path | Kind | Required names / contents | Live consumers | Notes |
|---|---|---|---|---|---|
| core-env | `/etc/finite/core.env` | env | `FC_CORE_DATABASE_URL`, `FC_CORE_API_TOKEN`, `FC_CORE_RUNNER_CREDENTIALS_JSON`, `FC_CORE_RUNNER_CREDENTIAL_TOKEN_*`, `FC_FINITE_PRIVATE_USAGE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `FC_WORKOS_OPERATOR_ORG_ID` | `finite-saas-core.service`; Phala installer mutates it today | Dynamic requirement: every active Core runner credential needs a matching token env var. Postgres role password must match the URL. |
| metrics-remote-write | `/etc/finite/metrics-remote-write.env` | env | `FINITE_METRICS_REMOTE_WRITE_USERNAME`, `FINITE_METRICS_REMOTE_WRITE_PASSWORD` | `alloy.service` | Used on both lat1 and lat3. Good SOPS pilot, but inherently two-host unless deliberately scoped. |
| runner-env | `/etc/finite/runner.env` | env | `FC_CORE_RUNNER_API_TOKEN`, `FC_RUNNER_FINITE_PRIVATE_SPECIALIZATION_WORKER_API_KEY`, plus drain/incident overrides | `finite-saas-runner.service`; Phala installer reads specialization key | Mixed credential plus operational-control file. Keep legacy until a split or migration policy is chosen. |
| phala-runner-env | `/etc/finite/phala-runner.env` | env | `FC_CORE_RUNNER_API_TOKEN`, `FC_RUNNER_PHALA_API_KEY`, `FC_RUNNER_FINITE_PRIVATE_SPECIALIZATION_WORKER_API_KEY` | `finite-saas-runner-phala.service` | Current installer creates this and updates Core metadata. |
| identity-operator | `/etc/finite/identity-operator.env` | env | `FINITE_IDENTITY_OPERATOR_TOKEN` | `finite-identity.service`, `finite-saas-runner.service`, `finite-saas-runner-phala.service`, `finite-brain-app.service`, `finitechat-hosted-device.service` | Also needed on lat3 for the remote Runner. |
| identity-sites-notification | `/etc/finite/identity-sites-notification.env` | env | `FINITE_IDENTITY_SITES_NOTIFICATION_TOKEN` | `finite-identity.service`, `finite-saas-sites.service` | Narrow Identity-to-Sites mail credential. |
| runtime-secrets | `/etc/finite/runtime-secrets.env` | env | `FAL_KEY`, `FRED_API_KEY`, `GOOGLE_PLACES_API_KEY`, `XAI_API_KEY`, `X_API_BEARER_TOKEN`, `ELEVENLABS_API_KEY`, `FIRECRAWL_API_KEY`, `PERPLEXITY_API_KEY` | Kata Runner via `FC_RUNNER_RUNTIME_SECRET_ENV_FILE`; Phala Runner via `LoadCredential` | Also needed on lat3. Core records names only. |
| dashboard-env | `/etc/finite/dashboard.env` | env | `FC_CORE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `WORKOS_COOKIE_PASSWORD`, `FC_WORKOS_OPERATOR_ORG_ID`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `GOOGLE_WORKSPACE_CLIENT_ID`, `GOOGLE_WORKSPACE_CLIENT_SECRET` | `podman-finite-saas-dashboard.service` container env files | Dashboard also reads hosted-device and sites-viewer-session env files. |
| hosted-web-device-env | `/etc/finite/hosted-web-device.env` | env | `FINITECHAT_HOSTED_API_TOKEN` | `finitechat-hosted-device.service`, dashboard container | Server-only internal boundary shared with dashboard. |
| brain-authority-env | `/etc/finite/brain-authority.env` | env | `FC_CORE_API_TOKEN` | `finite-brain-app.service` | Brain's trusted Core resolution credential. |
| sites-viewer-session-env | `/etc/finite/sites-viewer-session.env` | env | `FINITE_SITES_VIEWER_SESSION_TOKEN` | `finite-saas-sites.service`, dashboard container | Must remain exactly 64 lowercase hex chars. |
| sites-env | `/etc/finite-saas/sites.env` | env | `RESEND_API_KEY` | `finite-saas-sites.service`, `finite-identity.service`, `finite-brain-app.service` | Existing send-only mail credential. |
| searxng-env | `/etc/finite/searxng.env` | env | `SEARXNG_SECRET` plus optional `SEARXNG_BASE_URL`, `SEARXNG_LIMITER` | `podman-searxng.service` | Search-only, lower blast radius. |
| firecrawl-env | `/etc/finite/firecrawl.env` | env | `BULL_AUTH_KEY`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB`, `MAX_CPU`, `MAX_RAM` | `podman-firecrawl-nuq-postgres.service`, `podman-firecrawl-api.service` | Firecrawl stack currently uses one shared env file. |
| wireguard-private-key | `/etc/finite/wireguard-private-key` | opaque | WireGuard private key | `wg-finite` interface | Shared path name on both hosts, distinct value per host. |
| cloudflare-origin-cert | `/etc/finite-saas/certs/finite-chat-origin.pem` | opaque/public cert | Public Origin CA cert | Caddy vhosts for `api.finite.chat`, `*.finite.chat`, `*.docs.finite.chat` | Public certificate; migrate with key for atomic Caddy path management if useful. |
| cloudflare-origin-key | `/etc/finite-saas/certs/finite-chat-origin.key` | opaque/secret key | Origin CA private key | Caddy vhosts for `api.finite.chat`, `*.finite.chat`, `*.docs.finite.chat` | Must remain `root:caddy 0640` for Caddy. |
| borg-transport-key | `/var/lib/finitecomputer/backups/rsync-net/id_ed25519` | opaque | rsync.net SSH key | Borg offsite backup job | Recovery/bootstrap credential. |
| borg-known-hosts | `/var/lib/finitecomputer/backups/rsync-net/known_hosts` | opaque | pinned rsync.net host key | Borg offsite backup job | Integrity material, not a secret value, but part of bootstrap set. |
| borg-passphrase | `/var/lib/finitecomputer/backups/rsync-net/borg-passphrase` | opaque | Borg repository passphrase | Borg offsite backup job | Recovery/bootstrap credential. |
| litestream-latitude | `/etc/finite/litestream-latitude.env` | env | `LITESTREAM_ACCESS_KEY_ID`, `LITESTREAM_SECRET_ACCESS_KEY` | `finite-litestream.service`, `finite-litestream-health.service` | Missing file condition-skips replication and makes health fail loudly. |
| postgres-role-password | Postgres role state, also embedded in `FC_CORE_DATABASE_URL` | dynamic | finite role password | Postgres auth, Core database URL | Not a file by itself. Needs an auth drill, not just contract metadata. |

## Live lat3 Secret Inputs

| Logical name | Current path | Kind | Required names / contents | Live consumers | Notes |
|---|---|---|---|---|---|
| runner-env | `/etc/finite/runner.env` | env | `FC_CORE_RUNNER_API_TOKEN`, `FC_RUNNER_FINITE_PRIVATE_SPECIALIZATION_WORKER_API_KEY`, plus drain/incident overrides | `finite-saas-runner.service`; unit `ConditionPathExists` | File shape is documented by `infra/nixos/hosts/finite-lat-3/runner.env.example`. |
| runtime-secrets | `/etc/finite/runtime-secrets.env` | env | same selected runtime provider names as lat1 | Kata Runner via `FC_RUNNER_RUNTIME_SECRET_ENV_FILE` | Direct path is rendered into `/etc/finite/runner-shared.env`, not serviceConfig. |
| identity-operator | `/etc/finite/identity-operator.env` | env | `FINITE_IDENTITY_OPERATOR_TOKEN` | `finite-saas-runner.service` | Used to bind managed Agent Email through lat1's private Identity Authority. |
| metrics-remote-write | `/etc/finite/metrics-remote-write.env` | env | `FINITE_METRICS_REMOTE_WRITE_USERNAME`, `FINITE_METRICS_REMOTE_WRITE_PASSWORD` | `alloy.service` | Same logical credential as lat1 unless later split. |
| wireguard-private-key | `/etc/finite/wireguard-private-key` | opaque | WireGuard private key | `wg-finite` interface | Distinct host key from lat1. |

## Non-Live Or Historical Hits

- `infra/nixos/modules/oauth2-proxy.nix` references `/etc/finite/oauth2-proxy.env`,
  but neither finite-lat-1 nor finite-lat-3 imports that module today.
- `infra/hosts/lat1`, `infra/hosts/lat2`, `infra/hosts/smoke`, and
  `infra/hosts/clawland` contain historical captures, examples, and legacy
  non-NixOS surfaces. They are useful source notes, not current lat1/lat3
  production consumers unless a current NixOS module or active runbook still
  points at the same path.
- `scripts/install-identity-authority-credentials`,
  `scripts/install-identity-sites-notification-credential`, and
  `scripts/install-phala-canary-credentials` still create or mutate live
  `/etc/finite` files. They should remain valid only for legacy-backed entries
  or be replaced by SOPS-aware workflows later.
- `scripts/finite_status.py`, `scripts/rollout-lat1-runtime-artifact`, and
  several runbooks still read `/etc/finite/core.env` or
  `/etc/finite/runner.env` directly. They do not block the foundation, but they
  are mandatory follow-ups before legacy paths are removed.

## Initial SOPS Source Layout Hypothesis

This is a planning convenience, not implementation:

- `infra/nixos/secrets/shared/metrics-remote-write.env`
- `infra/nixos/secrets/shared/runtime-secrets.env` if the provider bundle stays
  intentionally shared
- `infra/nixos/secrets/shared/identity-operator.env` if the same operator token
  remains deliberate across lat1 and lat3
- `infra/nixos/secrets/finite-lat-1/*.env` for lat1-only service files
- `infra/nixos/secrets/finite-lat-1/wireguard-private-key`
- `infra/nixos/secrets/finite-lat-1/finite-chat-origin.{pem,key}` if Caddy TLS
  material is migrated through SOPS
- `infra/nixos/secrets/finite-lat-1/borg-rsync-net/*`
- `infra/nixos/secrets/finite-lat-3/runner.env`
- `infra/nixos/secrets/finite-lat-3/wireguard-private-key`

The root `.gitignore` currently ignores `secrets/`, so this layout needs
explicit unignore rules or a different tracked directory before implementation.

## Deferred Proofs Before Migration

- Run rendered NixOS consumer extraction on finite-lat-2 for both hosts.
- Create a machine-readable values-free lat3 secret contract.
- Add a checker that compares rendered consumers to the `finite.secrets`
  contract once that option exists.
- Validate dynamic Core runner credential token coverage without printing
  values.
- Validate Postgres role password agreement with `FC_CORE_DATABASE_URL` through
  an authorized auth drill.
