# Runtime slots dashboard

The runtime slots dashboard is included in `grafana/production.json` and uses
the shared [dashboard deployment and rollback](dashboards.md). Changes to this
dashboard do not require the full receiver deploy or Tinfoil collector.

Before approving query changes, verify them in Grafana Explore against the
live `finite-prometheus` datasource. The lat2 file-age series must exist, be
below 600 seconds, and advance through a Core collection cycle. Compare counts
with `scripts/finite-status`, allowing the five-minute collection cadence and
excluded incomplete artifact identity.

The existing collector retains its file on failure. Grafana therefore gates
on `node_textfile_mtime_seconds`, not scrape health. Missing or stale collectors
show Unknown. Unused slots are estimates, not admission capacity.

After deployment, open `/d/finite-agent-runtime-slots` and compare counts and
sample age with Explore. The deployment verifies that Grafana loaded the exact
source-owned definition, while these live checks establish its meaning against
current collector data.
