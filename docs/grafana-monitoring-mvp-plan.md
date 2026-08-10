# Grafana Monitoring MVP Plan

Status: not started

Requirements: `docs/grafana-monitoring-mvp-requirements.md`

## Objective

Ship a lean Grafana monitoring MVP for production that shows:

- current software versions and deployment artifacts;
- public endpoint uptime;
- current internal healthcheck state;
- a minimal alert set for obvious production failures.

Do not expand this plan into logs, traces, APM, business analytics, public
status pages, or automated repair.

## Todo List

### 1. Choose the MVP Hosting Shape

- [ ] Choose Grafana Cloud or a small external monitoring host.
- [ ] Confirm the Prometheus-compatible metrics backend.
- [ ] Confirm the public uptime probe mechanism:
  - Grafana Synthetic Monitoring, or
  - Prometheus blackbox exporter, or
  - equivalent HTTP checker.
- [ ] Confirm the paging destination and non-paging ticket destination.
- [ ] Record chosen service names, owners, and recreation notes without secrets.

Done when:

- The monitoring host/provider is chosen.
- The setup does not depend only on `finite-lat-1`.
- Alert destinations are known and documented by name only.

### 2. Add Public Uptime Checks

- [ ] Add check for `https://finite.computer`.
- [ ] Add check for `https://chat.finite.computer`.
- [ ] Add check for `https://brain.finite.computer`.
- [ ] Add check for one representative `https://*.finite.chat` route.
- [ ] Add check for one representative `https://*.docs.finite.chat` route.
- [ ] Verify each check emits `probe_success`.
- [ ] Verify each check emits response duration.
- [ ] Verify each check records HTTP status code or equivalent result detail.

Done when:

- All minimum public targets are visible as time-series data.
- At least one hour of probe history is visible before dashboard work starts.

### 3. Export Internal Health Metrics

- [ ] Decide whether the exporter reads `finite-healthcheck` journal state or
  `finite-status --json`.
- [ ] Export aggregate health:

  ```text
  finite_healthcheck_success{host="finite-lat-1"} 1
  ```

- [ ] Export per-service health:

  ```text
  finite_service_health_status{host="finite-lat-1",service="finitechat-server"} 1
  ```

- [ ] Include the same internal service list as
  `infra/nixos/modules/monitoring.nix`.
- [ ] Add a contract test or static check if new repo code owns these metric
  names.
- [ ] Verify no metric labels contain secrets, tokens, customer ids, or private
  user data.

Done when:

- Grafana can query current internal health.
- Internal health matches the latest `finite-healthcheck`/`finite-status`
  result.

### 4. Export Version and Artifact Metrics

- [ ] Export `finite_component_build_info` for `finite-saas-core`.
- [ ] Export `finite_component_build_info` for `finite-saas-dashboard`.
- [ ] Export `finite_component_build_info` for `finitechat-server`.
- [ ] Export `finite_component_build_info` for `finitechat-hosted-device`.
- [ ] Export `finite_component_build_info` for `finite-brain`.
- [ ] Export `finite_component_build_info` for `finite-saas-sites`.
- [ ] Export `finite_component_build_info` for `searxng`.
- [ ] Export `finite_component_build_info` for `firecrawl`.
- [ ] Export `finite_component_build_info` for `postgres`.
- [ ] Export `finite_component_build_info` for `finite-saas-runner`.
- [ ] Export `finite_component_build_info` for `finite-agent-runtime`.
- [ ] Export host NixOS system profile info for `finite-lat-1`.
- [ ] Export host NixOS system profile info for `finite-lat-3`.
- [ ] Export `finite_runtime_artifact_info` for promoted Runtime artifact state.
- [ ] Export `finite_component_version_mismatch` for required components.
- [ ] Verify versions come from deployed state, not hand-maintained dashboard
  values.
- [ ] Verify no version metric exposes secrets or private user data.

Done when:

- Every required component appears in Grafana with version/source data.
- Runtime artifact state is visible separately from single-service binary
  versions.
- Version mismatch queries return a clear `0` or `1`.

### 5. Build the Grafana Dashboard

- [ ] Create dashboard named `Finite Production MVP`.
- [ ] Add current production status panel.
- [ ] Add 24h uptime chart by public target.
- [ ] Add 7d uptime chart by public target.
- [ ] Add response-time panel by public target.
- [ ] Add running versions table.
- [ ] Add Runtime artifact panel.
- [ ] Add version drift panel.
- [ ] Keep panel count limited to the MVP sections.
- [ ] Export dashboard JSON or document managed-service dashboard recreation.

Done when:

- The dashboard answers the two MVP questions without requiring ad-hoc queries.
- A fresh operator can identify down endpoints, failing internal health, current
  versions, and version drift from one dashboard.

### 6. Add MVP Alerts

- [ ] Page when any critical public target has `probe_success == 0` for 2
  consecutive checks.
- [ ] Page when `finite_healthcheck_success == 0` for 5 minutes.
- [ ] Page when `finite_healthcheck_success` is missing for 5 minutes.
- [ ] Ticket when `finite_component_version_mismatch == 1` for 15 minutes.
- [ ] Ticket when version metrics are missing for any required component for 15
  minutes.
- [ ] Run one safe alert-delivery test that does not mutate production state.
- [ ] Document the alert names and destinations without secret values.

Done when:

- Paging and ticket delivery are proven once.
- Alert rules are either checked into the repo or documented well enough to
  recreate in the managed service.

### 7. Final Verification

- [ ] Confirm public uptime is visible for at least 24 hours.
- [ ] Confirm all required version metrics are present.
- [ ] Confirm internal `finite-healthcheck` state is visible and current.
- [ ] Confirm dashboard JSON or managed-service recreation notes are stored.
- [ ] Confirm no secrets appear in metrics, labels, dashboard JSON, or docs.
- [ ] Run `scripts/finite-status --json` as retained evidence, if appropriate
  for the rollout.
- [ ] Update this plan status to `complete`.

Done when:

- Every acceptance criterion in
  `docs/grafana-monitoring-mvp-requirements.md` is satisfied.

## Deferred Work

Do not start these until the MVP above is complete:

- Logs.
- Traces.
- APM.
- Business metrics.
- Public status page.
- Incident workflow automation.
- Automated remediation.
- Deep per-Agent lifecycle dashboards.
