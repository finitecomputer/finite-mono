# Production Monitoring

Production monitoring uses a narrow Ubuntu/systemd setup on the existing
Latitude VM. No Docker Compose is part of the active path.

The active receiver config is `infra/monitoring/ubuntu/`. It runs:

- Grafana at `monitoring.finite.computer`
- Prometheus remote-write ingestion at `metrics-ingest.finite.computer/api/v1/write`
- Loki log ingestion at `metrics-ingest.finite.computer/loki/api/v1/push`
- Blackbox HTTP probes for the narrow public uptime dashboard
- Caddy as the only public edge

Production dashboard updates use the
[dashboard-only deployment workflow](dashboards.md). The reviewed production
list is `grafana/production.json`; it includes the overview and Agent Runtime
slots dashboards and excludes the Tinfoil draft. Merges queue a deployment
through GitHub's existing `production` environment approval gate. See
[runtime slots](runtime-slots.md) for its query semantics and live checks.

The separate [Finite Private requests and usage dashboard](finite-private-requests.md)
documents limiter telemetry, seven-day request diagnostics, and its pending
production handoff. Initial provisioning is separate from dashboard updates.

Prometheus, Loki, Grafana, and blackbox exporter bind only to loopback. Caddy
terminates TLS and protects the metrics/log ingest routes with separate basic
auth credentials.

The Chat public probe is semantic rather than process-only: once per minute it
calls `https://chat.finite.computer/readyz`. The server must acquire its shared
delivery lock and commit a service-owned SQLite probe row within one second;
the blackbox edge-to-store request has a 1.5-second timeout. Either a 503 or a
slow response makes the existing `chat.finite.computer` availability series
red in Grafana. The service caches results for thirty seconds to coalesce the
host and public checks and bound the write rate of this public endpoint.

Roll out the lat1 closure that serves `/readyz` before deploying the monitoring
receiver change. A rollback to a pre-`/readyz` server closure must also roll the
receiver target back to `/health`; otherwise Chat can be serving while the
newer probe correctly reports that its expected semantic endpoint is absent.

Finite Sites has two public checks every minute against its Fly deployment:
`finite.site` requires HTTP 200 from `https://finite.site/api/v2/healthz`
(including the server's Git dependency preflight), and `uptime-probe.finite.site`
requires HTTP 404 from an unallocated wildcard hostname. The latter checks
wildcard DNS, TLS and HTTP routing; keep that hostname unallocated. Neither
check proves publishing, stored content, or private viewer access. Both appear
in the overview's current status, 24-hour/7-day uptime and response-time panels.
New series begin at rollout; historical `finite.chat` samples are not relabeled.

The dashboard-only workflow does **not** deploy `ubuntu/prometheus.yml`.
Updating Sites monitoring requires deploying the receiver's probe configuration
as well as the dashboard. Back up the live configuration before replacing it,
validate it with the installed `promtool check config`, reload Prometheus with
SIGHUP, and verify both targets and their `probe_success` samples. Restore the
previous configuration and reload if validation fails. Preserve unrelated live
scrape jobs when reconciling receiver drift. Record `scripts/finite-status`
before and after the rollout. Then use the
[dashboard-only workflow](dashboards.md).

## Sites usage metrics

The `finite-sites-metrics` scrape reads `https://finite.site/internal/v1/metrics`
every minute with a dedicated bearer credential. It cannot publish or access
private Sites. Provisioning and rollout order are in the
[Sites deployment runbook](../runbooks/deploy-sites.md#usage-metrics).

| Metric | Meaning |
| --- | --- |
| `finite_sites_existing` | Registry Sites excluding soft-deleted rows; includes unpublished and disabled Sites. |
| `finite_sites_published` | Rows with published status, across all visibility settings. |
| `finite_sites_created_by_day{date="YYYY-MM-DD"}` | Site allocations grouped by stored UTC creation date, including soft-deleted rows. |
| `finite_sites_metrics_collected_at_seconds` | Time of the registry snapshot used for this scrape. |

These are gauges derived from the existing registry, not process-local event
counters. The daily series cover 90 UTC calendar days including partial today,
with explicit zeroes for empty days. Bare Project Repositories, project-init
replays, and republishes do not add creations. No owner, name, email, site ID,
or per-site series is exported. Imported/restored rows use their stored dates;
purged historical rows cannot be recovered by monitoring. Verify retained
creation dates before interpreting pre-cutover counts as complete history.

The overview uses an instant table query and a date-sorted bar chart, so all
90 retained days appear on the first successful scrape, regardless of the
selected dashboard duration or Prometheus's 15-day sample retention. The
selected end time determines which snapshot is queried; dates before
collection began (or outside sample retention) have no snapshot. It does not
estimate calendar counts with a rolling `increase()` or rewrite old samples.
The two totals and the chart show no data if scraping fails or the snapshot is
at least three minutes old (or future-dated); the collection tile explicitly
shows `UNAVAILABLE`. A genuine empty registry displays zeroes.

Collection runs on an existing read-only SQLite reader, on the blocking pool,
outside the publishing mutex. A read transaction keeps totals and date buckets
consistent. Its source of truth is `sites.created_at` written at allocation
and `sites.status` updated by publishing/operator lifecycle operations; metrics
add no schema, durable counters, or writes. Rollbacks to older Sites images
make collection unavailable without affecting the public uptime probes.

## Credentials and deployment

### Billing account dashboard (awaiting initial provisioning)

`grafana/dashboards/finite-billing-accounts.json` reports one account per Core
`customer_orgs` row, including missing `customer_billing_accounts` rows. It does
not count users, infer human engagement, verify receipts, or measure revenue.
An active subscription is **not confirmed cash payment**; discounts and delayed
webhooks can affect its meaning. The dashboard explains the exact precedence of
its nine exclusive categories: subscribed, trial, sponsored, grandfathered,
expired/past-due, incomplete, no subscription, missing billing account, unknown.
Trial deadlines use the stored period end, falling back to redeemed-at plus
campaign days only for redeemed trials. Reservations alone do not prove access.

The lat2 `finite-billing-metrics` timer runs every five minutes, reusing the
existing Core connection loader and credentials locally. Its single read-only
SQL aggregate scans organizations and joins billing/trial rows by indexed keys;
only category counts and a snapshot timestamp leave Postgres. There are nine
fixed status series, a collection-success gauge, and a snapshot-time gauge.
No schema, Core request path, Stripe call, credential, or public endpoint is
added. Query/lock/connect/process deadlines are 5/1/5/15 seconds. The hardened
oneshot has a 20-second limit and atomically caches mode-0640 text in
`/run/finite-monitoring/finite-billing.prom`; existing loopback node-exporter
and Alloy transport it. Scrapes do not query Core. A collection failure replaces
counts with failure status; timestamp and scrape gates hide missing, future or
at-least-ten-minute-old data. A healthy empty database exports explicit zeroes.
The timestamp measures collection freshness, not Stripe reconciliation freshness.

Deployment is separate from dashboard layout updates and runtime/trial rollouts:

1. In a separately authorized rollout, deploy the reviewed lat2 NixOS closure
   containing the collector and Alloy allowlist. Verify the timer, eleven
   `finite_billing_*` samples, and unchanged Core/Chat health with `finite-status`.
   No Core image, migrations, Stripe credentials, or new database role is needed.
2. Before initial provisioning, record whether the file and UID already exist,
   their prior content, and the candidate file hash in an operator receipt.
   Provision the new JSON file through the existing Grafana file provider on the
   monitoring receiver; verify its UID `finite-billing-accounts`, category sum,
   freshness, and failure behavior. Do not edit Grafana's database directly.
3. Only after initial provisioning, add its filename/UID to `grafana/production.json`
   for routine dashboard CI updates. The current deployment helper requires an
   existing file and UID; adding it early blocks the entire dashboard bundle.
   An overview navigation link can be added with the separate layout change.

Rollback the collector via the prior lat2 closure and remove only its cached
textfile if left behind; collection gates also expire it after ten minutes.
Restore a backed-up dashboard file, or remove a newly created file only if its
bytes still match the receipt. The provider has `disableDeletion: true`, so
removing the file **does not remove its Grafana UID**. This is an incomplete
Grafana rollback: preserve the receipt and have an operator reconcile that
exact UID before retrying, following [dashboard rollback](dashboards.md).
No billing or user data is written, so there is no data migration to reverse.
Local/CI proof is part of `just monitoring-nixos-contract`: disposable Postgres
with the actual owning migrations, exporter failure/label tests, evaluated Nix
service/transport checks, and Promtool fixtures for every panel query.

The monitoring host stores operational credentials only as operator-provisioned
host files:

- `/etc/finite/monitoring/grafana-admin-password`
- `/etc/finite/monitoring/grafana-secret-key`
- `/etc/finite/monitoring/sites-metrics-token` (raw read-only Sites bearer token)
- `/etc/finite/monitoring/caddy.env`

`caddy.env` contains only Caddy basic-auth usernames and password hashes:

```env
METRICS_USERNAME=...
METRICS_PASSWORD_HASH=...
LOGS_USERNAME=...
LOGS_PASSWORD_HASH=...
```

Do not put credential values, password hashes, or generated Grafana secrets in
this repository.

LAT hosts send data with separate root-owned env files:

- `/etc/finite/metrics-remote-write.env`
- `/etc/finite/logs-write.env`

The logs file must contain `FINITE_LOGS_WRITE_USERNAME` and
`FINITE_LOGS_WRITE_PASSWORD`, matching the monitoring receiver's logs-write
credential. It is intentionally separate from the Prometheus remote-write
credential.

The repository-provisioned Grafana dashboard includes `finite-lat-1` through
`finite-lat-5`. Retired hosts remain visible in the scrape-health panel as
`DOWN` after their remote-written series goes stale, while replacement hosts
appear as soon as their Alloy collectors begin writing with the corresponding
host label.

Before activating a LAT host closure that includes journald log shipping, an
operator can validate the host-local files early without printing values:

```sh
ssh root@finite-lat-1 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
ssh root@finite-lat-2 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
ssh root@finite-lat-3 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
ssh root@finite-lat-4 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
ssh root@finite-lat-5 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
```

`scripts/deploy-lat1-closure-cache` runs this preflight automatically for lat1
when the target revision contains the log-shipping Alloy config. The NixOS
activation also runs the preflight on every host with Alloy log shipping
configured, including finite-lat-3.

Deploy from a clean checkout after the change is on `origin/main`:

```sh
infra/monitoring/ubuntu/deploy --replace-compose ubuntu@152.236.5.27
```

The explicit `--replace-compose` flag is required because the deploy stops the
old container stack before starting systemd Caddy on ports 80 and 443.

Validate the values-free contracts locally with:

```sh
python3 infra/monitoring/ubuntu/check_contract.py
just monitoring-nixos-contract
```
