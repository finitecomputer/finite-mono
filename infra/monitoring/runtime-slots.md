# Deploy the runtime slots dashboard

This is a dashboard-only change on the existing Ubuntu monitoring receiver.
It does not restart services, change collectors, deploy LAT closures, roll
Agent images, or write Core/chat state. Do not use `ubuntu/deploy` for this
operation: that command also changes and restarts the monitoring stack.

## Data contract and compatibility

The writer is `finite-runtime-metrics` on the app-plane host, finite-lat-2.
Every five minutes it reads Core and atomically replaces
`/run/finite-monitoring/finite-runtime.prom`. Node exporter reads that file;
Alloy scrapes it every fifteen seconds and remote-writes the allowed metrics
to Prometheus. Grafana reads Prometheus through `finite-prometheus`.

The dashboard uses the existing `finite_runtime_artifact_active_agents` and
`node_textfile_mtime_seconds` series, restricted to
`instance="finite-lat-2",job="finite-internal-health"`. Old lat1 samples
cannot contribute counts or establish freshness. The existing collector
retains its previous file on failure: a healthy scrape can keep old counts
alive indefinitely. Counts are therefore hidden at a file age of ten minutes,
for a future-dated file, or when the exact file-age series is absent. The
sample-age panel stays visible when stale. No exporter change is required.

An older collector/transport without the file-age series produces **Unknown**,
not an optimistic capacity number. An empty host also produces Unknown because
the current writer does not emit per-host zeroes. Incomplete artifact identity
is excluded from the count. Unused slots are an upper estimate against the
source-controlled 42-slot ceiling, not an admission signal: drain, readiness,
reservations and uncounted Runtimes can reduce actual capacity. Use
`scripts/finite-status` for platform/fleet decisions.

## Before the production mutation

Production deployment requires explicit owner authorization. Merging this PR
does not deploy the dashboard. From a clean checkout of the authorized merged
revision, run `just monitoring-nixos-contract` and record
`scripts/finite-status` from the production app-plane environment using the
established operator workflow. A local laptop status is not production proof.

On the monitoring receiver / in Grafana, verify:

1. Grafana is healthy and `finite-prometheus` is the existing Prometheus data
   source. The file provider matches
   `ubuntu/grafana/provisioning/dashboards/finite.yml` (the install below also
   checks this). It polls the dashboard directory every thirty seconds.
2. Run the new dashboard's **Core Sample Age** query in Explore. It must be
   nonnegative and below 600 seconds, and advance through a successful Core
   collection cycle. Confirm the `file` label is the full path above. If the
   live labels differ, stop and correct/test the dashboard before deploying;
   do not remove its freshness gate.
3. Run the recorded-count queries in Explore and compare them with current
   `scripts/finite-status` evidence, allowing the five-minute collection
   cadence. Investigate discrepancies, including incomplete artifact identity.
4. Check UID `finite-agent-runtime-slots`. It must either be absent, or be the
   expected file-provisioned dashboard. If it is an unrelated/UI-managed
   dashboard, stop; do not overwrite it. Export any existing dashboard JSON
   for the rollout evidence.

Live label/count verification and visual acceptance remain production gates;
contract and PromQL fixture tests alone do not establish them.

## Install only this dashboard

Run these commands from the repository root after the checks and authorization:

```sh
set -euo pipefail
test -z "$(git status --porcelain)"
git fetch origin main
git merge-base --is-ancestor HEAD origin/main
slots_rev="$(git rev-parse HEAD)"
slots_target=ubuntu@152.236.5.27
slots_stage="$(mktemp -d)"
git show "${slots_rev}:infra/monitoring/grafana/dashboards/finite-agent-runtime-slots.json" > "${slots_stage}/slots.json"
git show "${slots_rev}:infra/monitoring/ubuntu/grafana/provisioning/dashboards/finite.yml" > "${slots_stage}/provider.yml"
printf '%s\n' "${slots_rev}" > "${slots_stage}/revision"
slots_remote="$(ssh -o BatchMode=yes "${slots_target}" mktemp -d /tmp/finite-runtime-slots.XXXXXX)"
scp "${slots_stage}/slots.json" "${slots_stage}/provider.yml" "${slots_stage}/revision" "${slots_target}:${slots_remote}/"
ssh -o BatchMode=yes "${slots_target}" sudo bash -s -- "${slots_remote}" <<'REMOTE'
set -euo pipefail
staging="$1"
exec 9>/run/lock/finite-runtime-slots.lock
flock -n 9
cmp "${staging}/provider.yml" /etc/finite/monitoring/grafana/provisioning/dashboards/finite.yml
destination=/var/lib/finite-monitoring/grafana/dashboards/finite-agent-runtime-slots.json
test -d "$(dirname "${destination}")"
test ! -L "${destination}"
install -d -m 0700 /var/backups/finite-monitoring/runtime-slots
backup="$(mktemp -d /var/backups/finite-monitoring/runtime-slots/deploy.XXXXXX)"
if test -e "${destination}"; then
  cp -p "${destination}" "${backup}/previous.json"
else
  touch "${backup}/previously-absent"
fi
cp "${staging}/revision" "${backup}/revision"
cp "${staging}/slots.json" "${backup}/candidate.json"
next="$(mktemp "${destination}.XXXXXX")"
trap 'rm -f "${next}"' EXIT
install -m 0644 "${staging}/slots.json" "${next}"
mv -T "${next}" "${destination}"
printf 'Dashboard installed; backup: %s\n' "${backup}"
sha256sum "${destination}"
REMOTE
```

Wait for the provider poll, then open `/d/finite-agent-runtime-slots` in Grafana.
Verify all eight panels load, the source age and both hosts' recorded counts
match Explore, and estimates are clearly labelled. Confirm the original MVP
dashboard still works. Record the source revision, installed hash, backup
directory, visual evidence and post-change `scripts/finite-status`. Remove
only the printed local/remote staging directories when evidence is saved.

## Rollback boundary

Only this dashboard's JSON and its Grafana database representation change.
The installer preserves the prior JSON (or an absence marker) in the printed
root-only backup directory. Retain that backup through acceptance.

For an existing dashboard, atomically restore `previous.json` to the exact
destination above using a temporary file in that same directory, mode 0644,
and `mv -T`; allow the provider's next poll to restore the previous dashboard.
No Grafana or Prometheus restart is needed.

For a first installation, remove only
`finite-agent-runtime-slots.json`, then delete UID
`finite-agent-runtime-slots` through Grafana after the next provider poll
unprovisions it. The provider has `disableDeletion: true`, so removing the
file alone leaves the dashboard in Grafana. Grafana 13.0.2 explicitly removes
the provisioning metadata for a missing file in this mode
([implementation](https://github.com/grafana/grafana/blob/v13.0.2/pkg/services/provisioning/dashboards/file_reader.go#L223-L243)).
Verify the MVP dashboard and platform status again after either rollback.
