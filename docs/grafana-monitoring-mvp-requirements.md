# Grafana Monitoring MVP Requirements

Status: approved

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
- Public uptime checks run externally through Grafana Cloud Synthetic
  Monitoring for the MVP.

## MVP Scope

The MVP includes:

- One Grafana dashboard.
- One Prometheus-compatible metrics store.
- Uptime checks for the main public production surfaces.
- Internal health status published directly by `finite-healthcheck`, using the
  same probe contract that `finite-status --json` reports.
- Software version and artifact metrics for production architecture
  components.

The MVP does not include:

- Logs, traces, profiling, APM, or distributed tracing.
- Per-request latency analysis beyond HTTP probe duration.
- Business metrics.
- Public status page.
- Automated remediation.
- Alert rules, notifications, paging, and ticketing integrations.
- Per-Agent deep lifecycle dashboards beyond the current aggregate artifact and
  health status.
- A custom incident management workflow.

## Recommended Shape

Use Grafana Cloud as the UI and Grafana Cloud Metrics, its hosted Mimir service,
as the Prometheus-compatible time-series backend.

Send internally collected metrics through the standard Prometheus
`remote_write` protocol. Keep metric names and dashboard queries in Prometheus
formats so the setup can later move to self-hosted Prometheus, Mimir, or another
compatible backend without redesigning the metrics or dashboard. Historical
data does not need to be migrated.

Use Grafana Cloud Synthetic Monitoring for public uptime checks. Configure only
basic HTTP checks for the MVP:

- one public probe location;
- one check every five minutes per target;
- expected HTTP status validation;
- standard `probe_*` metrics for dashboard queries.

Do not use browser checks, scripted checks, private probes, or Synthetic
Monitoring logs for the MVP. Store the target list and expected status behavior
in the repository so the managed checks can later be replaced by Prometheus
blackbox exporter with minimal changes.

Avoid making `finite-lat-1` the only monitoring host. The external uptime checks
must continue recording failures if `finite-lat-1` disappears.

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
probe_success{job="finite.computer",instance="https://finite.computer",probe="NorthVirginia"} 1
probe_duration_seconds{job="finite.computer",instance="https://finite.computer",probe="NorthVirginia"} 0.123
probe_http_status_code{job="finite.computer",instance="https://finite.computer",probe="NorthVirginia"} 200
```

Minimum public targets:

- `https://finite.computer`
- `https://chat.finite.computer/health`
- `https://brain.finite.computer/health`
- `https://finitechat-native-mockup.finite.chat/`
- `https://uptime-probe.docs.finite.chat/`

Each target must be checked from one public probe location every five minutes.
The check only needs to validate that the endpoint returns its expected HTTP
status. The first four targets expect `200`.

No stable public document output currently exists for the `*.docs.finite.chat`
surface. The reserved `uptime-probe.docs.finite.chat` target expects `404` to
verify wildcard DNS, TLS, edge routing, and the Finite Sites unknown-document
handler without creating product content for monitoring. Replace it with a
stable public document target expecting `200` when one exists.

Internal health should be represented separately:

```text
finite_healthcheck_success{host="finite-lat-1"} 1
finite_service_health_status{host="finite-lat-1",service="finitechat-server"} 1
```

Internal service health must be sourced from the existing
`finite-healthcheck`/`finite-status` path. `finite-healthcheck` should publish
the metrics atomically through node-exporter's textfile collector; it must not
introduce a second probe loop. The first dashboard should show the same service
list as `infra/nixos/modules/monitoring.nix`.

Grafana Alloy should scrape the loopback-only node exporter once per minute and
send the retained MVP metrics through standard Prometheus `remote_write`. Retain
only the health, component version, Runtime artifact, version mismatch, `up`,
`node_textfile_scrape_error`, and `node_textfile_mtime_seconds` families. The
latter two make collector failure and stale textfile output visible without
adding another custom metric.

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

## Acceptance Criteria

The MVP is done when:

- Grafana has a working `Finite Production MVP` dashboard.
- Public endpoint uptime is visible for at least the last 24 hours.
- The dashboard shows current versions for every required component.
- The dashboard shows whether the internal `finite-healthcheck` is currently
  green.
- All monitoring configuration lives in the repo or in a documented managed
  service configuration.
- Dashboard queries use standard PromQL and standard `probe_*` metrics so they
  remain portable to a self-hosted Prometheus-compatible setup.
- No secret values are present in metrics, labels, dashboard JSON, or docs.

## Implementation Order

1. Add external uptime checks for the minimum public targets.
2. Export `finite-healthcheck` health through node exporter and standard
   Prometheus `remote_write`.
3. Export component version and runtime artifact metrics.
4. Create the Grafana dashboard.
5. Record the exact managed-service settings or checked-in config needed to
   recreate the setup.
