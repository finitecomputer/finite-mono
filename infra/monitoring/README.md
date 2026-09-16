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

The monitoring host stores operational credentials only as operator-provisioned
host files:

- `/etc/finite/monitoring/grafana-admin-password`
- `/etc/finite/monitoring/grafana-secret-key`
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
`finite-lat-4`. Retired hosts remain visible in the scrape-health panel as
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
