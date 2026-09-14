# Dashboard deployment

Agents edit JSON under `grafana/dashboards/` and submit a PR. The explicit
production list, `grafana/production.json`, includes the overview and runtime
slots dashboards and excludes the Tinfoil draft. `just monitoring-nixos-contract`
validates the list, monitoring configuration, queries, and deployment recovery
in both PR CI and the deployment workflow.

Merges to `main` touching `infra/monitoring/` or the deployment workflow queue
`.github/workflows/monitoring-dashboards.yml`. It validates the merged revision,
then waits for the existing `production` environment reviewer approval. After
approval it deploys without a separate shell command or Grafana restart.
Manual dispatch on `main` retries this workflow; other branches do not run.
The environment permits `main` and `production`, with its existing reviewers.

## Restricted CI access

| GitHub `production` environment secret | Purpose |
| --- | --- |
| `FINITE_MONITORING_SSH_KEY` | Dedicated Ed25519 private key for the two forced commands described below |
| `FINITE_MONITORING_KNOWN_HOSTS` | Verified host pins for `152.236.5.27` and `64.34.80.19` |
| `FINITE_PRODUCTION_KNOWN_HOSTS` | Existing production pins, preserved alongside the monitoring pins |

The existing `FINITE_PRODUCTION_SSH_KEY` is not used or changed by this workflow.
Grafana's admin password stays at `/etc/finite/monitoring/grafana-admin-password`
on the monitoring host. GET-only verification reads it locally; no credential
value is copied into source or logs.

The dedicated key has `restrict` plus a forced command on each host:

- On `ubuntu@152.236.5.27`, a root-owned dispatcher permits only the installed
  dashboard helper's `preflight` and `apply` operations. Other commands fail.
- On `root@64.34.80.19`, the forced command runs the installed canonical
  `scripts/finite-status --json`. It ignores caller-supplied commands and code.

Neither connection permits an interactive shell, PTY, or forwarding. Dashboard
bundles carry a schema version and the expected helper SHA-256. A changed helper
cannot deploy until an operator installs that reviewed revision; CI credentials
cannot update their own dispatcher, Python helper, or status command.

## Operator setup and helper updates

From a clean, reviewed revision on `main`, using existing operator SSH access:

```sh
scripts/with-dev-env python3 infra/monitoring/install_ci_access.py /secure/path/monitoring-ci.pub
```

The installer copies the reviewed helper/dispatcher to
`/var/lib/finite-monitoring-ci/` on the monitoring host and copies the canonical
status entry point/module into that directory's `scripts/` subdirectory on the
app-plane host. Files are root-owned. It preserves unrelated authorized keys,
replaces only the reserved `finite-monitoring-ci` key entry, and records previous
files under `/var/backups/finite-monitoring-ci.<unique-suffix>/` on each host.
The app-plane authorized key is in `/root/.ssh/authorized_keys`; the monitoring
key is in `/home/ubuntu/.ssh/authorized_keys`. The existing Nix-managed operator
keys remain unchanged.

Store the matching private key in `FINITE_MONITORING_SSH_KEY` through GitHub's
secret API/UI; never commit it or print it. Obtain host pins through existing
strictly verified SSH connections or an independent trusted channel. Each run
fails before dashboard mutation if host trust, key access, helper version, or
status collection is invalid. The restricted key should be tested against both
hosts before approving the first Actions deployment.

Re-run the installer with the existing public key after reviewed changes to
`deploy_dashboards.py`, `ci_dispatch`, or the installed status implementation.
Record `scripts/finite-status` before and after installing production access.
To revoke access, remove only the reserved key entry from those two authorized
key files and delete the dedicated GitHub secret. Restore backed-up helper files
if rolling back an installation; do not overwrite unrelated newer operator keys.

## Dashboard transaction

The source must be a clean merged revision. If newer monitoring/workflow changes
exist on `main`, an old run fails rather than overwriting them. Approve the newer
run or dispatch current main. Actions never cancels an in-progress deployment;
a host lock serializes the dashboard transaction, helper installation, and the
existing full-stack deployment script.

Before any file replacement, every dashboard must already exist as a regular
file with its declared UID and belong to that same file provider in Grafana.
The provider config must match the repository, and the API representation must
match its current file. Database-assigned ID/version and server-added top-level
fields are ignored. Ambiguous ownership or drift fails without mutation.

This path updates existing dashboards. New dashboards or UID changes require
separately reviewed initial provisioning; adding a manifest entry alone fails
closed. Removing an entry stops deploying it and leaves the live dashboard
intact. No dashboard deletion is automated.

Exact previous bytes and before/after hashes are backed up under
`/var/backups/finite-monitoring-dashboards/<revision>.<unique-suffix>/`. Each
listed JSON file is replaced by an atomic rename. Grafana polls every 30 seconds;
verification waits up to 100 seconds plus bounded API requests for source-owned
fields to match. Independent dashboards can briefly show different revisions;
this is not a cross-dashboard database transaction. No services, data sources,
collectors, or metrics/log stores are changed. Loaded definitions do not prove
useful query results; review live query behavior separately.

The workflow records the installed canonical `scripts/finite-status` before and
after deployment on the app-plane host, retaining reports as Actions artifacts
for 14 days. Existing red/unknown platform state is recorded and does not prevent
a dashboard repair. Transport and invalid-report failures fail the step.

## Rollback

Installation or Grafana reload failures restore all previous dashboard files
and verify their reload. If recovery also fails, the job stays failed and logs
the backup boundary. Runner/SSH loss or process termination may require operator
recovery from the named backup. Backups have no automatic retention deletion.

For a successful deploy whose queries later prove incorrect, revert the JSON
change in Git, merge the revert, and approve its new deployment. Retrying an old
workflow is not rollback: the stale-revision guard rejects it.

An explicitly authorized manual deployment uses the same helper and records
status outside the checkout (run the final status command even if deploy fails):

```sh
scripts/with-dev-env python3 infra/monitoring/deploy_dashboards.py preview
scripts/with-dev-env python3 infra/monitoring/deploy_dashboards.py status --output /tmp/finite-status-before.json
scripts/with-dev-env python3 infra/monitoring/deploy_dashboards.py deploy
scripts/with-dev-env python3 infra/monitoring/deploy_dashboards.py status --output /tmp/finite-status-after.json
```

`ubuntu/deploy` remains the full-stack/bootstrap path and also writes the MVP
under the same lock. Use dashboard-only deployment for routine panel/query edits.
