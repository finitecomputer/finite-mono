# Grafana Monitoring MVP Requirements

Status: draft

## Goal

Build the smallest useful production monitoring setup that answers two
questions:

1. What software versions and deployment artifacts are running?
2. Are the main production endpoints up, and what has their uptime looked like?

This MVP should work quickly. It is not a full observability platform.

## Existing Contracts

- `scripts/finite-status` remains the operator source of truth for platform and
  fleet status.
- `infra/nixos/modules/monitoring.nix` already runs `finite-healthcheck` every
  minute and curls the internal health endpoints.
- New internal service probes must be added to `finite-healthcheck` and the
  `finite-status` contract instead of becoming one-off monitoring logic.
- Public uptime checks may probe externally through Grafana Synthetic
  Monitoring, Prometheus blackbox exporter, or an equivalent HTTP checker.

## MVP Scope

The MVP includes:

- One Grafana dashboard.
- One Prometheus-compatible metrics store.
- Uptime checks for the main public production surfaces.
- Internal health status derived from `finite-healthcheck` or
  `finite-status --json`.
- Software version and artifact metrics for production architecture
  components.
- A small alert set for endpoint down, internal healthcheck failing, and version
  mismatch.

The MVP does not include:

- Logs, traces, profiling, APM, or distributed tracing.
- Per-request latency analysis beyond HTTP probe duration.
- Business metrics.
- Public status page.
- Automated remediation.
- Per-Agent deep lifecycle dashboards beyond the current aggregate artifact and
  health status.
- A custom incident management workflow.

## Recommended Shape

Use Grafana as the UI and alerting surface. Use a Prometheus-compatible backend
for time-series storage.

For speed, prefer one of these:

- Grafana Cloud with Synthetic Monitoring and hosted Prometheus-compatible
  metrics.
- A small external monitoring host running Prometheus, blackbox exporter, and
  Grafana.

Avoid making `finite-lat-1` the only monitoring host. If anything runs on
`finite-lat-1`, there must still be an external dead-man or uptime check that
pages when lat1 disappears.

## Version Metrics

Expose one info-style metric per component:

```text
finite_component_build_info{
  host="finite-lat-1",
  component="finitechat-server",
  version="...",
  git_sha="...",
  image_digest="...",
  source="nix|image|core|env"
} 1
```

Required components for the first dashboard:

- `finite-saas-core`
- `finite-saas-dashboard`
- `finitechat-server`
- `finitechat-hosted-device`
- `finite-brain`
- `finite-saas-sites`
- `searxng`
- `firecrawl`
- `postgres`
- `finite-saas-runner`
- `finite-agent-runtime`
- host NixOS system profile for `finite-lat-1`
- host NixOS system profile for `finite-lat-3`

Runtime artifact state should use a separate metric because it is fleet state,
not a single service binary:

```text
finite_runtime_artifact_info{
  source_host_id="finite-lat-3",
  artifact_id="...",
  version_label="...",
  promoted="true"
} 1
```

Version mismatch should be a simple numeric metric:

```text
finite_component_version_mismatch{
  host="finite-lat-1",
  component="finitechat-server"
} 0
```

Rules:

- Metric labels must not contain secrets, tokens, customer ids, or private user
  data.
- Versions must come from deployed state: Nix system profile, image digest,
  release tag, environment pin, or Core-recorded runtime artifact state.
- Do not hand-maintain versions in Grafana.

## Uptime Metrics

Public endpoint checks should emit standard blackbox or synthetic monitoring
metrics:

```text
probe_success{target="https://finite.computer"} 1
probe_duration_seconds{target="https://finite.computer"} 0.123
probe_http_status_code{target="https://finite.computer"} 200
```

Minimum public targets:

- `https://finite.computer`
- `https://chat.finite.computer`
- `https://brain.finite.computer`
- one representative `https://*.finite.chat` route
- one representative `https://*.docs.finite.chat` route

Internal health should be represented separately:

```text
finite_healthcheck_success{host="finite-lat-1"} 1
finite_service_health_status{host="finite-lat-1",service="finitechat-server"} 1
```

Internal service health must be sourced from the existing
`finite-healthcheck`/`finite-status` path. The first dashboard should show the
same service list as `infra/nixos/modules/monitoring.nix`.

## Dashboard Requirements

Create one dashboard named `Finite Production MVP`.

It must have these sections:

- Current production status: all public probes and internal healthcheck result.
- Uptime charts: 24h and 7d `probe_success` by public target.
- Response time: `probe_duration_seconds` by public target.
- Running versions: table of component, host, version, git SHA, image digest,
  and source.
- Runtime artifact: current promoted artifact and active source-host
  distribution.
- Version drift: components where `finite_component_version_mismatch == 1`.

Keep the dashboard operational and dense. Do not add exploratory panels until
the MVP is already working.

## Alerts

Create only these alerts for MVP:

- Page: any critical public target has `probe_success == 0` for 2 consecutive
  checks.
- Page: `finite_healthcheck_success == 0` or missing for 5 minutes.
- Ticket: `finite_component_version_mismatch == 1` for 15 minutes.
- Ticket: version metrics are missing for any required component for 15 minutes.

Alert routing can be minimal: one paging destination and one non-paging
destination.

## Acceptance Criteria

The MVP is done when:

- Grafana has a working `Finite Production MVP` dashboard.
- Public endpoint uptime is visible for at least the last 24 hours.
- The dashboard shows current versions for every required component.
- The dashboard shows whether the internal `finite-healthcheck` is currently
  green.
- At least one safe test proves alert delivery works without mutating
  production state.
- All monitoring configuration lives in the repo or in a documented managed
  service configuration.
- No secret values are present in metrics, labels, dashboard JSON, or docs.

## Implementation Order

1. Add external uptime checks for the minimum public targets.
2. Export `finite-healthcheck` or `finite-status --json` health into
   Prometheus-compatible metrics.
3. Export component version and runtime artifact metrics.
4. Create the Grafana dashboard.
5. Add the four MVP alerts.
6. Record the exact managed-service settings or checked-in config needed to
   recreate the setup.
