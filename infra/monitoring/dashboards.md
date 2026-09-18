# Dashboard deployment

Agents edit JSON under `grafana/dashboards/` and submit a PR. The explicit
production list, `grafana/production.json`, includes the overview, runtime
slots, and Tinfoil dashboards. Tinfoil requires the initial provisioning below. `just monitoring-nixos-contract`
validates the list, monitoring configuration, queries, and deployment recovery
in both PR CI and the deployment workflow.

Merges to `main` touching `infra/monitoring/` or the deployment workflow queue
`.github/workflows/monitoring-dashboards.yml`. It validates the merged revision,
then deploys automatically without a separate approval, shell command, or
Grafana restart. The `production` environment retains its secrets and branch
restrictions but has no required reviewers; this also works with private
repositories on GitHub Team.
Manual dispatch on `main` retries this workflow; other branches do not run.
The environment permits `main` and `production`.

## Restricted CI access

| GitHub `production` environment secret | Purpose |
| --- | --- |
| `FINITE_MONITORING_SSH_KEY` | Dedicated Ed25519 private key for the two forced commands described below |
| `FINITE_MONITORING_KNOWN_HOSTS` | Verified host pins for `152.236.5.27` and `64.34.80.19` |

The existing production SSH key and host-pin secrets are not used or changed by this workflow.
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
hosts before enabling the first Actions deployment.

Re-run the installer with the existing public key after reviewed changes to
`deploy_dashboards.py`, `ci_dispatch`, or the installed status implementation.
Record `scripts/finite-status` before and after installing production access.
To revoke access, remove only the reserved key entry from those two authorized
key files and delete the dedicated GitHub secret. Restore backed-up helper files
if rolling back an installation; do not overwrite unrelated newer operator keys.

## Dashboard transaction

The source must be a clean merged revision. If newer monitoring/workflow changes
exist on `main`, an old run fails rather than overwriting them. Use the newer
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
after deployment on the app-plane host, retaining only overall/section status
summaries as Actions artifacts for 14 days. Regardless of repository visibility,
full reports with agent names, project IDs, addresses, and diagnostic details
are never uploaded. Existing red/unknown platform state is recorded and does
not prevent a dashboard repair. Transport and invalid-report failures fail the step.

## Rollback

Installation or Grafana reload failures restore all previous dashboard files
and verify their reload. If recovery also fails, the job stays failed and logs
the backup boundary. Runner/SSH loss or process termination may require operator
recovery from the named backup. Backups have no automatic retention deletion.

For a successful deploy whose queries later prove incorrect, revert the JSON
change in Git and merge the revert to trigger its deployment. Retrying an old
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

## Initial Tinfoil provisioning (FIN-33)

The Tinfoil dashboard is in the production manifest, but the normal workflow
still requires its file and UID to exist. Merge the telemetry PR first, then
this provisioning PR. The dashboard workflow fails closed until the operator
bootstrap below succeeds; then dispatch the workflow on current `main`.
Use a clean checkout of current `main`.
This is a monitoring-only rollout; the enclave, model, limiter, chat services,
and persistent user data are not changed. Alert delivery is deferred from
this basic-metrics MVP (scope and live acceptance remain in
[FIN-33](https://linear.app/finitecomputer/issue/FIN-33)).

An operator must explicitly authorize the rollout and provision
`TINFOIL_API_KEY` in `/etc/finite/monitoring/tinfoil.env`, owned by
`root:finite-monitoring`, mode `0640`. It is the Tinfoil **admin** credential
for control-plane reads, not an inference API key. No login command, credential
transfer, secret overwrite, or enclave relaunch is performed by the scripts.
The CLI and jq are pinned in `ubuntu/versions.env`; the usage source is fixed
in code, with no arbitrary usage-command configuration.

Prepare the values-free bundle locally, outside the checkout:

```sh
scripts/with-dev-env just monitoring-nixos-contract
scripts/with-dev-env python3 infra/monitoring/provision_tinfoil.py --export /tmp/tinfoil-dashboard.json
git archive --format=tar.gz --output=/tmp/tinfoil-monitoring-source.tar.gz HEAD \
  infra/monitoring scripts/finite-status scripts/finite_status.py scripts/finite_tinfoil_status.py
scp /tmp/tinfoil-dashboard.json /tmp/tinfoil-monitoring-source.tar.gz ubuntu@152.236.5.27:/tmp/
```

On the monitoring host, create a new root-owned staging directory, unpack that
reviewed archive, and place the bundle alongside it. Run these commands from
the unpacked source root with existing operator access. The archive contains
configuration/scripts only; nothing is compiled on the host.

```sh
sudo python3 scripts/finite-status --tinfoil --json
sudo bash infra/monitoring/tinfoil/bootstrap-collector
sudo python3 scripts/finite-status --tinfoil --json
sudo python3 infra/monitoring/provision_tinfoil.py --bundle /tmp/tinfoil-dashboard.json
sudo python3 infra/monitoring/provision_tinfoil.py --bundle /tmp/tinfoil-dashboard.json --apply
sudo python3 scripts/finite-status --tinfoil --json
```

Also record the normal platform-wide `scripts/finite-status --json` before and
after the rollout using existing app-plane access (including after a failed
attempt). `--tinfoil` runs on the monitoring host and supplements that evidence;
it does not replace the chat/fleet baseline. Keep full reports outside public
CI artifacts. The bootstrap and provisioner save Tinfoil-only status receipts.

`bootstrap-collector` holds the existing monitoring lock, verifies upstream
binary SHA-256 pins before installation, backs up every replaced file and
prior unit activity/enabled state, starts the collector and textfile exporter,
and reloads Prometheus with SIGHUP. Its only allowed Prometheus config delta
is the repository's Tinfoil scrape job; unrelated live drift stops the rollout.
The separately activated `finite-sites-metrics` job stays absent or present as
found; adding Tinfoil never activates Sites metrics or provisions its token.
Every other job and global setting must match the reviewed source (full-line
comments and blank lines may differ). The candidate retains existing live bytes
and is checked with the installed `promtool` before any service changes. A live
baseline change during tool preparation aborts before taking rollback ownership.
Grafana, Loki, Caddy, and inference services are not restarted. The script waits
for fresh source/status samples, valid utilization, GPU allocation, and both
limiter dependency checks through the canonical status command. A failed
check restores previous files/services automatically. Transport interruption
or process termination may require the printed standalone rollback command.

The initial provisioner defaults to read-only preflight. `--apply` requires
fresh live metrics, the exact provider/helper contract, an absent dashboard
file, no other file using its UID, and a Grafana GET returning exactly 404 for
that UID. It writes only `finite-tinfoil-gpu.json`, waits for Grafana's normal
file polling, verifies loaded fields and file ownership, and rechecks metrics.
It neither changes the provider nor gives the restricted CI key new powers.

Open [Finite Private Tinfoil GPUs](https://monitoring.finite.computer/d/finite-tinfoil-gpu)
and confirm values, units, the two independent freshness panels, and readable
layout. `scripts/finite-status --tinfoil --json` requires exactly one selected
series per metric, rejecting duplicate targets and absent/NaN samples. The
usage window is one hour, returning two-minute allocation-wide means; normal
sample lag is reflected in Sample Age. This does not prove per-GPU health,
inference throughput, accounting writes, or attestation. Retain a screenshot
and source/query comparison in FIN-33 before marking it Done.

### Tinfoil rollback

The collector bootstrap prints a root-only directory under
`/var/backups/finite-tinfoil-bootstrap.*`. Run its `rollback` script to restore
the exact prior binaries, units, scrape config, and textfile, restore prior
unit activity/enabled state, and reload Prometheus. It preserves secrets,
Prometheus history and all unrelated files. Use this receipt only for this
rollout; it intentionally restores those recorded files and would overwrite
a later change to the same paths. Record canonical status after rollback.

Initial dashboard provisioning records prior **absence** and candidate bytes
under `/var/backups/finite-monitoring-dashboards/tinfoil-initial-*`. On failure,
it removes its file only if the bytes still match its candidate. Since the
existing provider has `disableDeletion: true`, Grafana may retain the imported
UID in its database. This is reported as incomplete Grafana rollback; do not
retry by overwriting or deleting an ambiguous UID. Reconcile that exact UID
with the receipt before retrying. For a successful initial deployment,
subsequent query corrections use the normal Git revert/update workflow above.
