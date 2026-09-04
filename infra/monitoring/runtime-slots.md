# Deploy only the runtime slots dashboard

This updates one Grafana file on `ubuntu@152.236.5.27`. Use this procedure
instead of `ubuntu/deploy`, which changes and restarts the monitoring stack.

Before owner-authorized deployment, run `just monitoring-nixos-contract` and
record production `scripts/finite-status`. In Grafana Explore, verify the
candidate queries against the live `finite-prometheus` datasource: the lat2
file-age series must exist, be below 600 seconds, and advance through a Core
collection cycle. Compare counts with platform status, allowing the five-minute
collection cadence and excluded incomplete artifact identity. Check dashboard
UID `finite-agent-runtime-slots` is absent or belongs to the expected
file-provisioned dashboard; export it before replacing an existing dashboard.

The existing writer retains its file on failure. Grafana therefore gates on
`node_textfile_mtime_seconds`, not scrape health. Missing/older collectors show
Unknown. The dashboard explains why unused slots are estimates, not admission
capacity; no collector or LAT-host deployment is required.

From a clean checkout of the **exact owner-approved merged revision**:

```bash
set -euo pipefail
: "${slots_rev:?Set slots_rev to the full owner-approved commit SHA}"
[[ "${slots_rev}" =~ ^[0-9a-f]{40}$ ]]
test "$(git rev-parse HEAD)" = "${slots_rev}"
test -z "$(git status --porcelain)"
git fetch origin main
git merge-base --is-ancestor "${slots_rev}" origin/main
slots_target=ubuntu@152.236.5.27
slots_remote="$(ssh -o BatchMode=yes "${slots_target}" mktemp -d /tmp/finite-runtime-slots.XXXXXX)"
scp infra/monitoring/grafana/dashboards/finite-agent-runtime-slots.json infra/monitoring/ubuntu/grafana/provisioning/dashboards/finite.yml "${slots_target}:${slots_remote}/"
ssh -o BatchMode=yes "${slots_target}" sudo bash -s -- "${slots_remote}" "${slots_rev}" <<'REMOTE'
set -euo pipefail
staging="$1"
exec 9>/run/lock/finite-runtime-slots.lock
flock -n 9
cmp "${staging}/finite.yml" /etc/finite/monitoring/grafana/provisioning/dashboards/finite.yml
destination=/var/lib/finite-monitoring/grafana/dashboards/finite-agent-runtime-slots.json
test -d "$(dirname "${destination}")"
test ! -L "${destination}"
backup="$(mktemp -d "/var/backups/finite-runtime-slots.$2.XXXXXX")"
if test -e "${destination}"; then
  cp -p "${destination}" "${backup}/previous.json"
else
  touch "${backup}/previously-absent"
fi
cp "${staging}/finite-agent-runtime-slots.json" "${backup}/candidate.json"
next="$(mktemp "${destination}.XXXXXX")"
trap 'rm -f "${next}"' EXIT
install -m 0644 "${backup}/candidate.json" "${next}"
mv -T "${next}" "${destination}"
printf 'Dashboard installed; backup: %s\n' "${backup}"
sha256sum "${destination}"
REMOTE
```

After the provider's thirty-second poll, open `/d/finite-agent-runtime-slots`.
Verify the counts and sample age against Explore and confirm the MVP dashboard
still works. Record the installed hash, backup path and production
`scripts/finite-status`; remove the staging directory after saving evidence.

**Rollback:** restore `previous.json` using a mode-0644 temporary file in the
same dashboard directory and `mv -T` to the destination above. Grafana reloads
it on the next poll. For a first installation, remove only the new dashboard
file, wait for Grafana to remove its file-provisioning association, then delete UID
`finite-agent-runtime-slots` in Grafana. The provider's `disableDeletion: true`
leaves the database dashboard behind when a file is removed. Verify the MVP
dashboard and platform status after rollback; no service restart is required.
