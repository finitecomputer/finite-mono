# Grafana Cloud Monitoring

Grafana Cloud provides the managed UI, Prometheus-compatible metrics backend,
and public Synthetic Monitoring probes for the monitoring MVP.

## Public Uptime Checks

`public-uptime-checks.json` is the non-secret source of intent and recreation
record for the managed HTTP checks. Keep it synchronized with Grafana Cloud.
Do not record stack credentials, access tokens, or other secret values here.

All managed checks use the `NorthVirginia` public probe, a five-minute
frequency, HTTP `GET`, a three-second timeout, the reduced metric set, and no
per-check alerts.

| Job | Target | Expected status | Coverage |
|---|---|---:|---|
| `finite.computer` | `https://finite.computer` | `200` | Main product frontend |
| `chat.finite.computer` | `https://chat.finite.computer/health` | `200` | Public Chat health |
| `brain.finite.computer` | `https://brain.finite.computer/health` | `200` | Public Brain health |
| `finitechat-native-mockup.finite.chat` | `https://finitechat-native-mockup.finite.chat/` | `200` | Published Finite Site |
| `uptime-probe.docs.finite.chat` | `https://uptime-probe.docs.finite.chat/` | `404` | Document wildcard routing |

The document wildcard check deliberately accepts the unknown-document `404`.
It verifies DNS, TLS, Cloudflare, Caddy, and the Finite Sites request handler
without publishing monitoring-only product content. Replace it with a stable
public document target expecting `200` when one exists.

The managed Prometheus data source returned the required metrics for all five
checks after creation:

| Job | `probe_success` | HTTP status | Duration (seconds) |
|---|---:|---:|---:|
| `finite.computer` | `1` | `200` | `0.139703177` |
| `chat.finite.computer` | `1` | `200` | `0.104056804` |
| `brain.finite.computer` | `1` | `200` | `0.251101944` |
| `finitechat-native-mockup.finite.chat` | `1` | `200` | `0.289723682` |
| `uptime-probe.docs.finite.chat` | `1` | `404` | `0.293538005` |

These values are verification evidence from the initial executions on
2026-08-10, not expected fixed values for future executions.

Dashboard queries should use the standard Synthetic Monitoring metrics and
labels, including:

```promql
probe_success{
  job="finite.computer",
  instance="https://finite.computer",
  probe="NorthVirginia"
}
```

The managed checks can later be replaced by Prometheus blackbox exporter. Keep
dashboard queries on standard `probe_*` metrics and preserve the `job`,
`instance`, and `probe` label meanings during that migration.
