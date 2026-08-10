# Self-Hosted Grafana Monitoring Migration Plan

Status: in progress

Depends on: `docs/grafana-monitoring-mvp-requirements.md`

## Goal

Replace the Grafana Cloud UI, metrics backend, and public probes with one small
self-hosted monitoring VPS. Keep the existing dashboard, PromQL, version
metrics, health metrics, and five public checks. Historical Grafana Cloud data
does not need to move.

This plan does not add logs, traces, alerts, paging, ticketing, high
availability, monitoring backups, or host-performance dashboards.

## Chosen Shape

- Provider: Hetzner Cloud.
- Operating system: Ubuntu 24.04 LTS on x86_64.
- Storage: the VPS local disk; no separate volume is required for the MVP.
- UI: Grafana OSS.
- Metrics backend: Prometheus with 15-day and 20 GiB retention limits.
- Public probes: Prometheus blackbox exporter every five minutes.
- Public edge: Caddy with automatic TLS.
- Deployment: one digest-pinned Docker Compose stack in
  `infra/monitoring/self-hosted/`.

Use `monitoring.finite.computer` for Grafana and
`metrics-ingest.finite.computer` for Prometheus remote write. Only ports 22,
80, and 443 are public. Prometheus, Grafana's container port, and blackbox
exporter are not published directly.

Caddy exposes the Grafana UI and only `POST /api/v1/write` on the metrics
hostname. The write route uses a dedicated basic-auth credential over TLS.
Grafana anonymous access and public sign-up are disabled.

Before DNS is available, the same stack can run in temporary raw-IP mode. This
serves Grafana over HTTP for operator validation but returns `426 TLS Required`
from remote write. The Hetzner firewall must restrict port 80 to the operator's
current public IP in this mode. LAT credentials must not be sent until the DNS
and TLS cutover.

## What Stays The Same

- `finite-healthcheck` remains the source of internal health metrics.
- Node exporter remains loopback-only on the LAT hosts.
- Alloy continues scraping the existing narrow metric allowlist.
- Metric names and labels remain unchanged.
- The dashboard keeps its seven MVP panels and existing PromQL.
- `scripts/finite-status` remains the operator status command.
- The five public checks keep their targets, expected statuses, three-second
  timeout, and five-minute interval.

## Todo List

### 1. Add The VPS Stack

- [x] Add digest-pinned Grafana, Prometheus, blackbox exporter, and Caddy
  services.
- [x] Persist Grafana, Prometheus, and Caddy state in Docker volumes.
- [x] Limit Prometheus retention to 15 days and 20 GiB.
- [x] Enable the Prometheus remote-write receiver.
- [x] Expose only Caddy ports 80 and 443 from the Compose stack.
- [x] Protect the exact remote-write route with basic authentication.
- [x] Disable Grafana anonymous access and public sign-up.
- [x] Provision the Prometheus data source and dashboard from the repository.
- [x] Add an Ubuntu 24.04 installer that generates secrets only on the VPS.
- [x] Add a temporary raw-IP mode that refuses plaintext remote write.
- [x] Add static, Prometheus, blackbox, Caddy, and Compose validation.
- [x] Run `infra/monitoring/self-hosted/install-ubuntu` on the VPS.
- [ ] Run `/opt/finite-monitoring/verify` in raw-IP mode.
- [ ] Point the two DNS records at the VPS.
- [ ] Rerun the installer in DNS mode and verify TLS.

Done when Grafana loads over HTTPS from an empty VPS and the repository-owned
dashboard is present without manual UI configuration.

### 2. Replace Public Synthetic Monitoring

- [x] Configure the five existing public targets in Prometheus.
- [x] Preserve each target's current `job` and `instance` labels.
- [x] Keep separate expected-`200` and expected-`404` blackbox modules.
- [x] Retain only `probe_success`, `probe_duration_seconds`, and
  `probe_http_status_code` from public probes.
- [ ] Confirm all five targets are green in self-hosted Prometheus.
- [ ] Confirm the public dashboard panels populate.
- [ ] Accumulate 24 hours of self-hosted uptime data.

Done when the VPS records the five checks independently of Grafana Cloud and
the existing uptime panels work without query changes.

### 3. Redirect LAT Remote Write

- [ ] Rename the Cloud-specific Alloy environment file and variables to
  provider-neutral names.
- [ ] Point Alloy at
  `https://metrics-ingest.finite.computer/api/v1/write`.
- [ ] Install the VPS-generated metrics-write password on `finite-lat-1` and
  `finite-lat-3` without committing it.
- [ ] Update the LAT secret bootstrap contracts and operator documentation.
- [ ] Deploy the reviewed LAT closures one host at a time.
- [ ] Run `scripts/finite-status --json` before and after each rollout.
- [ ] Verify internal health, version, Runtime artifact, scrape-error, and
  freshness metrics in self-hosted Prometheus.

Done when both LAT hosts send only the existing metric allowlist to the VPS and
all seven dashboard panels have their expected data.

### 4. Retire Grafana Cloud

- [ ] Verify the complete self-hosted dashboard for at least 24 hours.
- [ ] Verify the stack survives a VPS reboot with its state.
- [ ] Disable the five Grafana Cloud Synthetic Monitoring checks.
- [ ] Revoke the Grafana Cloud metrics-write token and access policy.
- [ ] Delete or cancel the Grafana Cloud stack after the rollback window.
- [ ] Mark this plan complete.

Done when Grafana Cloud can disappear without stopping new uptime or internal
metrics.

## Cutover And Rollback

Bring up the VPS and public checks before changing either LAT host. Keep
Grafana Cloud active during validation. Historical continuity is not required,
so there is no need to migrate old samples.

Cut over one LAT host at a time. If ingestion or dashboard checks fail, restore
that host's previous Grafana Cloud environment file and NixOS generation. Do
not revoke the Cloud token until both LAT hosts and the public checks have been
healthy on the VPS for 24 hours.

Stopping the VPS stack with `docker compose down` preserves its named volumes.
Do not use `--volumes` during rollback unless deleting monitoring history is
intentional.

## Acceptance Criteria

- [ ] Grafana is available over HTTPS with anonymous access disabled.
- [ ] Prometheus and blackbox exporter are not directly internet-accessible.
- [ ] Unauthenticated remote-write requests return `401`.
- [ ] All five public targets have at least 24 hours of self-hosted uptime data.
- [ ] Current internal health, component versions, Runtime artifacts, and
  version drift are visible on `Finite Production MVP`.
- [x] Dashboard and data-source provisioning are repository-owned.
- [x] All application container images are versioned and digest-pinned.
- [x] No secret value is present in Git, metrics, labels, or dashboard JSON.

## Final Authorization Todos

- [ ] Authorize creating the two production DNS records.
- [ ] Authorize running the installer and generating credentials on the
  monitoring VPS.
- [ ] Authorize installing the root-owned metrics-write credential on
  `finite-lat-1` and `finite-lat-3`.
- [ ] Authorize deploying the provider-neutral Alloy changes to
  `finite-lat-1` and `finite-lat-3`.
- [ ] Authorize disabling checks, revoking credentials, and deleting the
  Grafana Cloud stack after the rollback window.
