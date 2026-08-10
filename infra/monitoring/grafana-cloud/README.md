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

## Internal Health Metrics

`infra/nixos/modules/monitoring.nix` owns the internal-health publisher for
`finite-lat-1`, while `infra/nixos/modules/metrics.nix` owns the shared
loopback scrape and remote-write transport:

1. `finite-healthcheck` runs the existing internal endpoint probes and writes
   `finite-healthcheck.prom` atomically after every attempt.
2. Node exporter reads that file through its standard textfile collector on
   loopback port `9100`.
3. Grafana Alloy scrapes node exporter once per minute, keeps only the MVP
   health, scrape, and freshness metrics, and sends them using Prometheus
   `remote_write`.

The publisher runs inside `finite-healthcheck` rather than parsing the journal
or invoking `finite-status --json`. This keeps one probe loop. The journal
remains the evidence source read by `scripts/finite-status`, and the contract
test requires the Prometheus service labels to match the same probe list.

The two application-owned metric families are:

```promql
finite_healthcheck_success{host="finite-lat-1"}
finite_service_health_status{host="finite-lat-1"}
```

The remote-write filter also retains these standard collector signals:

```promql
up{job="finite-internal-health",instance="finite-lat-1"}
node_textfile_scrape_error{job="finite-internal-health",instance="finite-lat-1"}
time() - node_textfile_mtime_seconds{job="finite-internal-health",instance="finite-lat-1"}
```

The last query reports the age in seconds of the latest atomic healthcheck
publication. A green value is stale when that age is materially greater than
the one-minute timer interval.

Alloy reads its remote-write settings from the root-owned, mode `0600` file
`/etc/finite/grafana-cloud-metrics.env`:

```dotenv
GRAFANA_CLOUD_PROMETHEUS_URL=<Grafana Cloud remote-write URL>
GRAFANA_CLOUD_PROMETHEUS_USERNAME=<Grafana Cloud Prometheus username>
GRAFANA_CLOUD_PROMETHEUS_PASSWORD=<stack-scoped metrics:write token>
```

The Grafana Cloud objects created for this transport on 2026-08-10 are:

- stack: `savvybanana1713`;
- access policy: `finite-production-metrics-write`;
- access-policy scope: `metrics:write` only;
- token name: `finite-production-alloy`;
- token expiration: no expiry.

The token value is displayed once by Grafana Cloud and is not stored in this
repository.

The file is a deployment prerequisite and never belongs in this repository.
Install it independently on `finite-lat-1` and `finite-lat-3`. The access-policy
token needs only the `metrics:write` scope. Redirecting the URL and credentials
to a self-hosted Prometheus-compatible receiver does not change the emitted
metrics or dashboard PromQL.

## Version And Artifact Metrics

Each evaluated NixOS closure renders a static Prometheus textfile containing its
native package versions, host system profile, and configured image tags or
digests. Nix links that file into node exporter's textfile directory during
activation, so static versions need no process or timer. These are deployment
facts, not values maintained in Grafana.

Only `finite-lat-1` runs `finite-runtime-metrics` every five minutes. It reuses
the read-only Core query path from `scripts/finite-status` to publish the current
promoted artifact and active source-host distribution. No Runtime process
inspection or second inventory service is part of the MVP.

The retained metric families are:

```promql
finite_component_build_info
finite_component_version_mismatch
finite_runtime_artifact_info
```

`finite_component_version_mismatch` is `0` for components fixed by the active
NixOS closure. For `finite-agent-runtime`, it is `1` on a source host when any
active Core-recorded Runtime uses an artifact other than the current promoted
artifact. The exporter fails without replacing its previous metric file when
Core evidence is unavailable or incomplete.

## MVP Dashboard

The managed dashboard is
[`Finite Production MVP`](https://savvybanana1713.grafana.net/d/finite-production-mvp/finite-production-mvp).
Its provider-neutral source moved to
`../self-hosted/grafana/dashboards/finite-production-mvp.json`. The self-hosted
stack provisions its `finite-prometheus` data source UID automatically.

The dashboard contains exactly the MVP views:

- current public and internal status;
- public uptime over 24 hours and 7 days;
- public response time;
- running component versions;
- Runtime artifact distribution;
- version drift.

The public status and uptime panels were verified against all five Synthetic
Monitoring targets on 2026-08-10. Internal health and version panels remain
empty until the reviewed NixOS metrics closures and Grafana Cloud credentials
are deployed to the production hosts.
