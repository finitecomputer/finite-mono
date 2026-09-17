# Production infrastructure

`infra/` owns deployment configuration. Services run CI-built, digest-pinned
images or exact CI-built NixOS closure artifacts. Nothing is built on a
production host.

## Current boundaries

| Surface | Authority |
| --- | --- |
| Core, dashboard, Chat, Hosted Device, Brain, Identity and edge | `nixos/hosts/finite-lat-2/` and `nixos/modules/` |
| Kata Runner hosts | `nixos/hosts/finite-lat-{3,4,5}/` |
| Sites at `finite.site` | `fly/sites/fly.toml` |
| Finite Private inference | `tinfoil/` satellite configuration and measured images |
| Monitoring | `monitoring/`, receiver configuration and dashboard workflow |

Lat1 is retired. Legacy host captures under `hosts/` are not deployment
authority. Read-only observation, not a documentation timestamp or old row
count, establishes live physical state. Caddy on lat2 retains old Sites content
redirects; the Sites daemon runs on Fly.

## Operations

Use [the runbook index](runbooks/README.md). Run `scripts/finite-status` before
and after every rollout. App-plane deployment, Runtime artifact promotion and
existing-Agent rollout are separate operations. A source merge authorizes none
of them by itself.

Use the exact reviewed revision, immutable artifact and named target. Preserve
accepted writes across binary rollback; restore requires an isolated empty
target and the complete Recovery Set. A persistent volume, successful upload
or green timer alone does not prove recoverability.

## Secrets and monitoring

Only secret names and custody locations belong in git. See
[the credential inventory](nixos/README.md#secrets-bootstrap-checklist-values-never-in-this-repo)
and [SOPS operations](secret/OPERATIONS.md). Rotate exposed secrets before
removing them from source. Keep independent recovery credentials outside the
compute they recover.

[Dashboard deployment](monitoring/dashboards.md) uses the GitHub production
environment's `FINITE_MONITORING_SSH_KEY` and `FINITE_MONITORING_KNOWN_HOSTS`.
Grafana's password remains on the receiver at
`/etc/finite/monitoring/grafana-admin-password`.

Sites metrics use `FINITE_SITES_METRICS_TOKEN` on Fly and
`/etc/finite/monitoring/sites-metrics-token` on the receiver; see
[metrics operations](runbooks/deploy-sites.md#usage-metrics).

[Images](images/README.md), [NixOS](nixos/README.md), and
[Tinfoil](tinfoil/README.md) document their respective artifact contracts.
