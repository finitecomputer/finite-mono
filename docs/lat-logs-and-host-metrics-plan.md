# LAT Logs And Host Metrics Plan

Status: in progress

Repository implementation has pivoted to a hard-cut Ubuntu/systemd monitoring
receiver because Latitude VMs do not offer NixOS as a supported VM image. The
old Docker Compose monitoring stack is removed from the active path. LAT host
collection and production rollout remain separate steps.

Current implementation progress: steps 1 through 3 below are implemented;
steps 4 and 5 remain pending.

## Goal

Show centralized logs and basic host performance metrics for the LAT fleet in
Grafana without changing product behavior.

The production rollout target is `finite-lat-1` and `finite-lat-3`. `finite-lat-2`
is included only after an explicit role decision, because the current infra
inventory names it as a decommission target rather than production capacity.

## Non-Goals

- No tracing, OpenTelemetry rollout, or request-span collection.
- No alerts, paging, ticketing, or incident workflow.
- No application `/metrics` instrumentation in this phase.
- No Agent Runtime or customer process stdout/stderr collection.
- No managed observability backend.
- No Docker Compose monitoring stack.
- No high availability, backup, or object-storage redesign for monitoring.
- No labels containing user, account, email, project, runtime, request, route,
  file path, or other high-cardinality/customer-derived values.
- No production mutation before the normal read-only status and rollout checks.

## Existing Base

- The active monitoring receiver is the Latitude Ubuntu VM, configured from
  `infra/monitoring/ubuntu/` with systemd services for Grafana, Prometheus,
  Loki, blackbox exporter, and Caddy.
- The repository still contains the earlier `nixosConfigurations.finite-monitoring`
  experiment, but it is not the deploy target for the Latitude VM path.
- LAT NixOS hosts already run Grafana Alloy and node exporter through
  `infra/nixos/modules/metrics.nix`.
- Alloy already remote-writes selected Prometheus series to
  `https://metrics-ingest.finite.computer/api/v1/write`.
- The current Prometheus relabel allowlist is intentionally narrow and excludes
  most node-exporter host-performance series.

## Design

### Metrics

Keep Prometheus as the metrics backend. Expand only the existing Alloy
Prometheus relabel allowlist to include basic node-exporter host metrics.

Initial metric families:

- CPU: `node_cpu_seconds_total`, `node_load1`, `node_load5`, `node_load15`.
- Memory and swap: `node_memory_MemTotal_bytes`,
  `node_memory_MemAvailable_bytes`, `node_memory_SwapTotal_bytes`,
  `node_memory_SwapFree_bytes`.
- Filesystems: `node_filesystem_size_bytes`, `node_filesystem_avail_bytes`,
  `node_filesystem_readonly`.
- Disk I/O: `node_disk_read_bytes_total`, `node_disk_written_bytes_total`,
  `node_disk_io_time_seconds_total`.
- Network: `node_network_receive_bytes_total`,
  `node_network_transmit_bytes_total`, `node_network_receive_errs_total`,
  `node_network_transmit_errs_total`.
- Existing health/version series remain unchanged.

Bound the cardinality at collection time where practical:

- Keep filesystem panels focused on `/` and `/data`.
- Drop pseudo filesystems and ephemeral runtime mounts from dashboard queries.
- Keep network panels focused on physical WAN and `wg-finite` interfaces.
- Do not add per-container, per-Kata-sandbox, or per-Agent labels.

### Logs

Run Grafana Loki as a native systemd service on the Ubuntu monitoring VM and
keep Alloy as the only LAT-side collector. Alloy reads journald and pushes
selected service logs to Loki over HTTPS.

Initial Loki labels:

- `host`
- `unit`
- `priority`
- `role`

Do not add labels derived from log message content.

Initial unit allowlist:

- `finite-lat-1`: Caddy, Core, dashboard Podman unit, finitechat-server,
  finitechat-hosted-device, Finite Brain, finite-saas-sites, finite-identity,
  finite-saas-runner, finite-healthcheck, Alloy, node exporter, backup and
  storage-health units.
- `finite-lat-3`: finite-saas-runner, storage-health units, WireGuard/network
  units, Alloy, node exporter.
- `finite-lat-2`: excluded from the production dashboard unless its role is
  updated. If explicitly monitored while decommissioning, label it
  `role="decommission"` and keep it out of production availability summaries.

### Ingest

Use a separate Loki write credential from the existing Prometheus remote-write
credential.

Expose Loki ingestion only over TLS at
`https://metrics-ingest.finite.computer/loki/api/v1/push`. Do not permit raw-IP
or plaintext log ingest.

Set a short initial Loki retention window: 14 days until log volume is measured.

## Implementation Steps

1. Add the Ubuntu/systemd monitoring receiver.
   - Add repo-owned Ubuntu configs and systemd units for the existing Latitude
     monitoring VM.
   - Run Grafana, Prometheus, blackbox exporter, Loki, and Caddy as native
     systemd services.
   - Keep Prometheus, Loki, blackbox exporter, and Grafana loopback-only behind
     Caddy.
   - Add repository-owned Loki configuration with local filesystem storage and
     bounded retention.
   - Add a Grafana Loki datasource with a stable UID.
   - Add static contract checks for the Ubuntu service files, datasources,
     retention, protected routes, and lack of direct public port exposure.

2. Add protected Loki ingest.
   - Use a separate logs-write credential from the metrics-write credential.
   - Extend the Ubuntu Caddy config with a TLS-only Loki push route.
   - Return 404 for non-push Loki routes.
   - Document the credential file names and env variable names without
     committing values.

3. Expand host metrics collection.
   - Extend the Alloy Prometheus relabel keep regex for the metric families in
     this plan.
   - Keep node exporter loopback-only.
   - Add Grafana panels for CPU, load, memory, swap, disk usage, disk I/O,
     network throughput, and scrape health.
   - Do not add app request latency panels in this phase.

4. Add LAT journald collection.
   - Add Nix options or a small Nix data structure for the host/unit allowlist.
   - Configure Alloy `loki.source.journal`, relabeling only safe journald
     fields into `host`, `unit`, `priority`, and `role`.
   - Point Alloy at the protected Loki push route.
   - Keep the Loki credential in a root-owned environment file or SOPS-managed
     equivalent; do not reuse the Prometheus credential.

5. Roll out one host at a time.
   - Run `scripts/finite-status --json` before each host rollout.
   - Roll out `finite-lat-3` first.
   - Verify Prometheus host metrics and Loki logs in Grafana.
   - Roll out `finite-lat-1` after `finite-lat-3` is clean.
   - Add `finite-lat-2` only after its role is explicitly updated or a
     decommission-only monitoring view is approved.
   - Run `scripts/finite-status --json` after each host rollout.

## Acceptance Criteria

- Grafana has a repository-provisioned `LAT Fleet` dashboard.
- The dashboard shows CPU, load, memory, swap, filesystem, disk I/O, network,
  and scrape-health data for each monitored LAT host.
- Grafana Explore can query recent logs by `host`, `unit`, `priority`, and
  `role`.
- Loki is not directly internet-accessible.
- Prometheus remote write and Loki push use separate credentials.
- No log or metric label contains user, account, email, project, runtime,
  request, route, file path, or other customer-derived values.
- `finite-lat-2` is either absent from production availability panels or
  clearly labeled as decommission-only.
- `scripts/finite-status --json` is captured before and after each host rollout.

## Deferred Follow-Ups

- Application request counters and latency histograms with normalized route
  labels.
- Alerting and paging.
- Trace collection.
- Loki backup, object storage, or high-availability work.
- Agent Runtime logs or customer workload observability.
