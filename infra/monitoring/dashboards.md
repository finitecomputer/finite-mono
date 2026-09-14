# Dashboard deployment

`grafana/production.json` maps production filenames to their stable Grafana
UIDs. Agents edit the JSON under `grafana/dashboards/` and submit a PR.
`just monitoring-nixos-contract` validates the production list, monitoring
configuration, runtime queries, and deployment recovery tests in both PR CI
and the deployment workflow. Draft-tagged dashboards cannot be deployed.

After a merge to `main` touching `infra/monitoring/` or the deployment workflow,
`.github/workflows/monitoring-dashboards.yml` validates the exact commit and
queues a deployment in the existing `production` GitHub environment. Its
required-reviewer gate still applies. There is no additional shell deployment
step after approval. Manual dispatch on `main` retries the workflow; dispatch
from other branches does not run. Workflow changes and operational docs also
trigger validation/deployment, which is a no-op when the files already match.

## Access and activation

The workflow uses existing environment secrets by name only:

| GitHub `production` environment secret | Required access |
| --- | --- |
| `FINITE_PRODUCTION_SSH_KEY` | SSH to `ubuntu@152.236.5.27` with noninteractive sudo, and read-only status execution via `root@64.34.80.19` |
| `FINITE_PRODUCTION_KNOWN_HOSTS` | Previously verified SSH host keys for both IP addresses |

The environment currently permits deployments only from the `production`
branch. Activation requires an owner-authorized addition of `main` to its
deployment branch policies, retaining the current required reviewers and the
existing `production` branch policy. Without that addition GitHub blocks this
workflow before credentials or deployment are available.

Both secrets already existed when this workflow was prepared. Their presence
does not prove that the stored key reaches the monitoring host or that its host
key is included. The first approved run checks host pins, SSH/sudo access, and
platform status before any dashboard mutation. A missing pin or denied login
fails closed. Update these secrets only from operator-held credentials and
independently verified host keys; never use an unauthenticated `ssh-keyscan`
result as the sole source of trust. GitHub's environment protection is retained.

Grafana's admin password remains on the monitoring host in
`/etc/finite/monitoring/grafana-admin-password`. The script reads it locally for
GET-only API verification; it is never copied into GitHub or logged.

Before the first deployment, review the candidate against live files without
modifying them:

```sh
scripts/with-dev-env python3 infra/monitoring/deploy_dashboards.py preview
```

After the branch-policy prerequisite, merging the workflow and approving its
first `production` deployment activates it. Future matching merges use the same
approval gate. Fully unattended deploys
would require a separately authorized change to GitHub environment protection.

## Deployment contract

The source is an exact merged revision with a clean checkout. If a newer
monitoring or deployment-workflow change exists on `main`, an old run fails
instead of overwriting it. Approve the newer run or dispatch from current main.
Actions never cancels an in-progress deployment. A host lock serializes the
dashboard transaction and the existing full-stack deployment script.

The deploy script checks every candidate before writing anything:

- The live file-provider config must exactly match the repository config.
- Every production filename must already be a regular file with its declared
  UID. Every UID must already belong to that same file provider in Grafana.
- The current Grafana API representation must match the current file, ignoring
  database-assigned ID/version and server-added top-level fields.

This path updates existing dashboards. Adding an entirely new dashboard or
changing a UID requires a separately reviewed initial provisioning operation;
adding it to the manifest alone fails closed. Removing a manifest entry stops
its deployment and leaves its live dashboard intact. No dashboard deletion is
automated. The Tinfoil draft remains excluded.

The only dashboard writes replace the listed JSON files under
`/var/lib/finite-monitoring/grafana/dashboards/`. Each file is replaced by an
atomic rename. The independent dashboards can briefly show different revisions
while Grafana reloads; this is not a cross-dashboard database transaction.
There is no service restart, datasource change, collector change, or API write.
Grafana polls every 30 seconds; deployment waits up to 100 seconds, plus bounded
API requests, for source-owned fields to match through the GET API. A healthy
HTTP endpoint alone is not success. This proves the definition loaded, not that
every query returns useful live data; review query behavior separately.

The workflow executes `scripts/finite-status` before and after the rollout on
the app-plane host. Reports are retained as Actions artifacts for 14 days;
existing red/unknown platform state is recorded and does not prevent repairing
an observability dashboard. Transport or invalid-report failures fail the step.
The deploy log records candidate hashes, source revision, and backup location.

## Rollback

Before replacement, exact previous JSON bytes and before/after hashes are saved
under `/var/backups/finite-monitoring-dashboards/<revision>.<unique-suffix>/`.
Backups stay on the monitoring host and have no automatic retention deletion.

If installation or API verification fails, the script restores all previous
files and verifies that Grafana reloads them. If restoration or verification
also fails, the job stays failed and its log names the backup boundary. No
automation can complete recovery after runner/SSH loss or process termination;
use the named backup and the normal operator recovery procedure in that case.

For a successful deploy whose queries later prove incorrect, revert the JSON
change in Git, merge the revert, and approve its deployment. That produces a
new merged revision and preserves history. Do not retry an old workflow as a
rollback: the stale-revision guard will reject it.

For an explicitly authorized manual deployment from a clean current `main`
checkout, use the same script and record status outside the checkout:

```sh
scripts/with-dev-env python3 infra/monitoring/deploy_dashboards.py status --output /tmp/finite-status-before.json
scripts/with-dev-env python3 infra/monitoring/deploy_dashboards.py deploy
# Run after the attempt even when deploy exits nonzero.
scripts/with-dev-env python3 infra/monitoring/deploy_dashboards.py status --output /tmp/finite-status-after.json
```

`ubuntu/deploy` remains the full-stack/bootstrap path and also writes the MVP
dashboard under the same host lock. Use dashboard-only deployment for routine
panel/query updates; an older full-stack checkout must not be used to roll back
unrelated monitoring configuration just to change a dashboard.
