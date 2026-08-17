# Production Monitoring

Production monitoring is a NixOS host, not a Docker Compose stack.

The active receiver is `nixosConfigurations.finite-monitoring`. It runs native
NixOS services for:

- Grafana at `monitoring.finite.computer`
- Prometheus remote-write ingestion at `metrics-ingest.finite.computer/api/v1/write`
- Loki log ingestion at `metrics-ingest.finite.computer/loki/api/v1/push`
- Blackbox HTTP probes for the narrow public uptime dashboard
- Caddy as the only public edge

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

Validate the values-free contract locally with:

```sh
just monitoring-nixos-contract
```
