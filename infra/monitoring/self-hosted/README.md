# Self-Hosted Monitoring

This is the complete monitoring MVP for the dedicated Ubuntu 24.04 VPS:

- Grafana OSS provides the dashboard UI.
- Prometheus stores metrics and accepts authenticated remote write.
- Prometheus blackbox exporter checks the five public endpoints every five
  minutes.
- Caddy exposes Grafana and the exact remote-write path, using HTTPS once DNS
  is configured.

The stack intentionally excludes logs, traces, alerting, paging, host
performance dashboards, high availability, and backups. Monitoring history may
be lost; the configuration and dashboard can be recreated from this directory.

## Temporary Raw-IP Install

The repository does not need to remain on the VPS. The installer copies this
directory to `/opt/finite-monitoring`; keeping a clone is only the easiest way
to pull and apply future reviewed updates. The
`infra/monitoring/self-hosted/` directory is self-contained and can instead be
copied to the VPS by itself.

Before using raw-IP mode, restrict inbound port 80 in the provider firewall to
your current public IP. Grafana login credentials travel over plaintext HTTP
in this temporary mode. Do not leave port 80 open to the internet.

From a repository checkout on the VPS, run:

```bash
sudo env \
  MONITORING_MODE=ip \
  MONITORING_IP=<VPS_PUBLIC_IPV4> \
  ./infra/monitoring/self-hosted/install-ubuntu
```

Then verify the stack and read the initial password:

```bash
sudo /opt/finite-monitoring/verify
sudo sh -c 'cat /etc/finite/monitoring/grafana-admin-password; echo'
```

Open `http://<VPS_PUBLIC_IPV4>` and sign in as `admin`. The public uptime
panels begin populating immediately. Raw-IP mode deliberately returns HTTP
`426` from `/api/v1/write`; do not point LAT Alloy at an unencrypted endpoint.

## DNS And TLS Cutover

Create DNS `A` records for `monitoring.finite.computer` and
`metrics-ingest.finite.computer` pointing to the VPS public IPv4 address. Keep
ports 80 and 443 public in the provider firewall and restrict port 22 to operator
IP addresses. Do not expose ports 3000, 9090, or 9115.

From a checkout of this repository on the VPS, run:

```bash
sudo env \
  MONITORING_MODE=dns \
  GRAFANA_DOMAIN=monitoring.finite.computer \
  METRICS_DOMAIN=metrics-ingest.finite.computer \
  ACME_EMAIL=ops@finite.computer \
  ./infra/monitoring/self-hosted/install-ubuntu
```

The installer uses Docker's official Ubuntu apt repository, generates or reuses
the Grafana and metrics-write credentials on the VPS, copies the stack to
`/opt/finite-monitoring`, pulls digest-pinned images, and starts the services.
It synchronizes Grafana's persisted admin account to the password file without
printing either credential.

After DNS resolves and Caddy has obtained certificates:

```bash
sudo /opt/finite-monitoring/verify
sudo sh -c 'cat /etc/finite/monitoring/grafana-admin-password; echo'
```

Open `https://monitoring.finite.computer` and sign in as `admin`. The
Prometheus data source and `Finite Production MVP` dashboard are provisioned;
there is no UI setup step.

The existing Docker volumes and credentials survive the switch from raw-IP to
DNS mode.

## LAT Remote Write

LAT hosts write internal health, version, and Runtime artifact metrics only to
`https://metrics-ingest.finite.computer/api/v1/write`. Install this root-only
file independently on `finite-lat-1` and `finite-lat-3` before activating a
closure that enables Alloy:

```dotenv
FINITE_METRICS_REMOTE_WRITE_USERNAME=<METRICS_USERNAME from /etc/finite/monitoring/stack.env on the monitoring VPS>
FINITE_METRICS_REMOTE_WRITE_PASSWORD=<contents of /etc/finite/monitoring/metrics-write-password on the monitoring VPS>
```

Apply the host permissions exactly:

```bash
sudo install -d -m 0700 -o root -g root /etc/finite
sudo install -m 0600 -o root -g root /tmp/metrics-remote-write.env /etc/finite/metrics-remote-write.env
```

The password is a shared write credential for the monitoring ingest endpoint.
It must not enter Git, shell history, logs, screenshots, or metric labels.

The version tables use Prometheus sample timestamps to display an `Observed`
time and to choose the newest row for each logical identity. Do not add changing
timestamps as metric labels; that creates additional Prometheus series and makes
duplicate-looking rows more likely.

## Operations

Inspect or follow the stack without reading secret values:

```bash
sudo docker compose \
  --env-file /etc/finite/monitoring/stack.env \
  -f /opt/finite-monitoring/compose.yaml ps
sudo docker compose \
  --env-file /etc/finite/monitoring/stack.env \
  -f /opt/finite-monitoring/compose.yaml logs -f --tail=100
```

To apply reviewed repository updates, rerun `install-ubuntu` from the updated
checkout. Existing credentials and Docker volumes are retained. To stop the
stack without deleting history, run `docker compose down` with the same env and
Compose file arguments. Do not add `--volumes` unless deleting monitoring data
is intentional.

## Secret Files

These root-owned files exist only on the VPS:

| File | Purpose |
| --- | --- |
| `/etc/finite/monitoring/grafana-admin-password` | Initial Grafana `admin` password |
| `/etc/finite/monitoring/metrics-write-password` | Plaintext credential installed on authorized LAT senders later |
| `/etc/finite/monitoring/caddy.env` | Caddy domains, username, and one-way password hash |
| `/etc/finite/monitoring/stack.env` | Non-secret Compose and verification settings |

No value from these files belongs in Git, metrics, labels, screenshots, or
documentation.

The secret directory is `root:root 0700`. The Grafana password file is
`root:472 0640` so the official Grafana container's unprivileged UID/GID can
read its mounted copy; the other files are `root:root 0600`.

The Grafana password file is authoritative. Rerunning `install-ubuntu` resets
the persisted `admin` account to that file's value.
