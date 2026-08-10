# Grafana Monitoring MVP Plan

Status: in progress

Requirements: `docs/grafana-monitoring-mvp-requirements.md`

## Objective

Ship a lean Grafana monitoring MVP for production that shows:

- current software versions and deployment artifacts;
- public endpoint uptime;
- current internal healthcheck state.

Do not expand this plan into logs, traces, APM, business analytics, public
status pages, alert notifications, paging, ticketing, or automated repair.

## Todo List

### 1. Choose the MVP Hosting Shape

- [x] Use Grafana Cloud as the managed monitoring host.
- [x] Use Grafana Cloud Metrics, backed by hosted Mimir, as the
  Prometheus-compatible metrics backend.
- [x] Send internally collected metrics using standard Prometheus
  `remote_write`.
- [x] Use Grafana Cloud Synthetic Monitoring basic HTTP checks for public
  uptime.
- [x] Use one public probe location and a five-minute check interval.
- [x] Record the selected services and portability requirements without
  secrets.

Done when:

- Grafana Cloud, Grafana Cloud Metrics, and Synthetic Monitoring are recorded as
  the MVP services.
- The setup does not depend only on `finite-lat-1`.
- Metric collection uses standard Prometheus formats and can be redirected to a
  self-hosted Prometheus-compatible backend.

### 2. Add Public Uptime Checks

- [x] Add check for `https://finite.computer`.
- [x] Add check for `https://chat.finite.computer/health`.
- [x] Add check for `https://brain.finite.computer/health`.
- [x] Add check for `https://finitechat-native-mockup.finite.chat/`.
- [x] Add routing check for `https://uptime-probe.docs.finite.chat/`, expecting
  the unknown-document `404` response until a stable public document exists.
- [x] Run implemented checks from one public probe location every five minutes.
- [x] Validate implemented checks against the expected HTTP status.
- [x] Store the resolved uptime target list and expected statuses in the
  repository.
- [x] Record all implemented check settings in
  `infra/monitoring/grafana-cloud/public-uptime-checks.json`.
- [x] Verify the `finite.computer` check emits `probe_success`, response
  duration, and HTTP status code.
- [x] Verify each remaining check emits `probe_success`, response duration, and
  HTTP status code.

Done when:

- All minimum public targets are visible as time-series data.
- Checks continue accumulating history for at least one hour before dashboard
  work starts.

### 3. Export Internal Health Metrics

- [x] Publish metrics directly from `finite-healthcheck`; keep its journal as
  the matching evidence source consumed by `finite-status --json`.
- [x] Export aggregate health:

  ```text
  finite_healthcheck_success{host="finite-lat-1"} 1
  ```

- [x] Export per-service health:

  ```text
  finite_service_health_status{host="finite-lat-1",service="finitechat-server"} 1
  ```

- [x] Include the same internal service list as
  `infra/nixos/modules/monitoring.nix`.
- [x] Add a contract test or static check if new repo code owns these metric
  names.
- [x] Limit Alloy remote write to the MVP health, scrape, and freshness metric
  families.
- [x] Record the host-only Grafana Cloud remote-write credential names and
  bootstrap file without secret values.
- [x] Verify no metric labels contain secrets, tokens, customer ids, or private
  user data.
- [ ] Install the Grafana Cloud `metrics:write` credential file on
  `finite-lat-1` and deploy the evaluated NixOS closure.
- [ ] Verify the current aggregate and per-service metrics in Grafana Cloud.

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

### 6. Final Verification

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
- Alert rules and notifications.
- Paging and ticketing integrations.
- Incident workflow automation.
- Automated remediation.
- Deep per-Agent lifecycle dashboards.

## Final Authorization Todos

These actions change production systems or managed-service state and require
explicit authorization before execution:

- [ ] Authorize creating a stack-scoped Grafana Cloud access-policy token with
  only `metrics:write` permission.
- [ ] Authorize installing the root-owned, mode `0600`
  `/etc/finite/grafana-cloud-metrics.env` credential file on `finite-lat-1`.
- [ ] Authorize deploying the reviewed monitoring and version-metric NixOS
  changes to `finite-lat-1`.
- [ ] Authorize creating or updating the `Finite Production MVP` dashboard and
  saving its managed Grafana Cloud configuration.
- [ ] Authorize rolling `finite-lat-1` back to its previous NixOS generation if
  post-deploy health checks fail.
