# LAT Logs And Host Metrics Plan

Status: in progress

Repository implementation has pivoted to a hard-cut Ubuntu/systemd monitoring
receiver because Latitude VMs do not offer NixOS as a supported VM image. The
old Docker Compose monitoring stack is removed from the active path. LAT host
collection and production rollout remain separate steps.

Current implementation progress: steps 1 through 4 below are implemented;
step 5 remains pending. During the lat1/lat3 hardware incident, the Grafana
dashboard has been widened to show lat1 through lat4 so the retiring hosts
remain visible while lat2 and lat4 come online.

## Goal

Show centralized logs and basic host performance metrics for the LAT fleet in
Grafana without changing product behavior.

The production dashboard target is the whole `finite-lat-1` through
`finite-lat-4` fleet. `finite-lat-1` and `finite-lat-3` remain visible while
they are retired so operators can see them as down instead of silently losing
the hosts from the dashboard.

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
- `source`

Do not add labels derived from log message content.

Initial unit allowlist:

- `finite-lat-1`: Caddy, Core, dashboard Podman unit, finitechat-server,
  finitechat-hosted-device, Finite Brain, finite-saas-sites, finite-identity,
  finite-saas-runner, finite-healthcheck, Alloy, node exporter, backup and
  storage-health units.
- `finite-lat-3`: finite-saas-runner, storage-health units, WireGuard/network
  units, Alloy, node exporter.
- `finite-lat-2`: app-plane successor for `finite-lat-1`; collect the same
  app-facing service logs once its production role is activated.
- `finite-lat-4`: runner successor for `finite-lat-3`; collect the same
  runner, storage-health, WireGuard/network, Alloy, and node-exporter logs once
  its production role is activated.

Separate host-incident sources:

- Kernel warning-or-higher journal entries (`_TRANSPORT=kernel`,
  `PRIORITY=0..4`) to catch thermal, OOM, filesystem, disk, NIC, and reset
  evidence without forwarding all kernel info/debug logs.
- systemd manager entries (`SYSLOG_IDENTIFIER=systemd`) to preserve unit
  lifecycle evidence that does not belong to the affected service's
  `_SYSTEMD_UNIT`.
- NixOS activation entries (`SYSLOG_IDENTIFIER=nixos`) to preserve
  `switch-to-configuration` and deployment activation context.
- SSH/sudo auth entries (`SYSLOG_IDENTIFIER=sshd|sudo`) for operator-access
  incident context. These remain message-only logs with the same bounded label
  set; auth message contents are not parsed into labels.

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
     fields into `host`, `unit`, `priority`, `role`, and `source`.
   - Point Alloy at the protected Loki push route.
   - Keep the Loki credential in a root-owned environment file or SOPS-managed
     equivalent; do not reuse the Prometheus credential.

   Repo status: implemented in `infra/nixos/modules/metrics.nix` for
   `finite-lat-1` and `finite-lat-3`, including the service allowlist and the
   host-incident sources above. `finite-lat-2` and `finite-lat-4` must use the
   same app and runner collection profiles respectively when their NixOS host
   definitions land. The next LAT NixOS activation will require
   `/etc/finite/logs-write.env` on each host with `FINITE_LOGS_WRITE_USERNAME`
   and `FINITE_LOGS_WRITE_PASSWORD`. The current host-local secret strategy is
   covered by `infra/nixos/scripts/check-lat-monitoring-secrets`; lat1 closure
   deploys run it before activation, and NixOS activation runs it on every host
   with log shipping configured.

5. Roll out one host at a time.
   - Run `scripts/finite-status --json` before each host rollout.
   - Optionally run `infra/nixos/scripts/check-lat-monitoring-secrets` on the
     target host before activation for an early failure; NixOS activation also
     enforces the same check.
   - Roll out the replacement host for the currently failing role first.
   - Verify Prometheus host metrics and Loki logs in Grafana.
   - Keep `finite-lat-1` and `finite-lat-3` visible in Grafana while retired;
     the scrape-health panel provides explicit zero-valued fallback series so
     absent retired hosts render as down.
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
- The scrape-health panel keeps `finite-lat-1` through `finite-lat-4` visible,
  including retired hosts that are no longer remote-writing metrics.
- `scripts/finite-status --json` is captured before and after each host rollout.

## Deferred Follow-Ups

- Application request counters and latency histograms with normalized route
  labels.
- Alerting and paging.
- Trace collection.
- Loki backup, object storage, or high-availability work.
- Agent Runtime logs or customer workload observability.
