# Production Monitoring

Production monitoring uses a narrow Ubuntu/systemd setup on the existing
Latitude VM. No Docker Compose is part of the active path.

The active receiver config is `infra/monitoring/ubuntu/`. It runs:

- Grafana at `monitoring.finite.computer`
- Prometheus remote-write ingestion at `metrics-ingest.finite.computer/api/v1/write`
- Loki log ingestion at `metrics-ingest.finite.computer/loki/api/v1/push`
- Blackbox HTTP probes for the narrow public uptime dashboard
- Caddy as the only public edge

Prometheus, Loki, Grafana, and blackbox exporter bind only to loopback. Caddy
terminates TLS and protects the metrics/log ingest routes with separate basic
auth credentials.

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

Before activating a LAT host closure that includes journald log shipping,
validate the host-local files without printing values:

```sh
ssh root@64.34.82.77 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
ssh root@207.188.7.157 'bash -s' < infra/nixos/scripts/check-lat-monitoring-secrets
```

`scripts/deploy-lat1-closure-cache` runs this preflight automatically for lat1
when the target revision contains the log-shipping Alloy config. Lat3 currently
needs the explicit preflight before its activation path.

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
