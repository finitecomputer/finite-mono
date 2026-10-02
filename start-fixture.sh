#!/usr/bin/env bash
set -euo pipefail
root=/tmp/grafana-cleanup-evidence
chmod -R a+rX "$root/provisioning" "$root/dashboards"
docker network create finite-evidence-net
docker run -d --name finite-evidence-prometheus --network finite-evidence-net --user 0:0 -p 127.0.0.1:19090:9090 -v "$root/prometheus.yml:/etc/prometheus/prometheus.yml:ro" -v "$root/prometheus-data:/prometheus" prom/prometheus:v3.13.2 --config.file=/etc/prometheus/prometheus.yml --storage.tsdb.path=/prometheus --storage.tsdb.retention.time=10d
docker run -d --name finite-evidence-loki --network finite-evidence-net --user 0:0 -p 127.0.0.1:13100:3100 -v "$root/loki.yml:/etc/loki/local-config.yaml:ro" -v "$root/loki-data:/loki" grafana/loki:3.5.8 -config.file=/etc/loki/local-config.yaml
docker run -d --name finite-evidence-grafana --network finite-evidence-net -p 127.0.0.1:13000:3000 -e NO_PROXY=localhost,127.0.0.1,finite-evidence-prometheus,finite-evidence-loki -e no_proxy=localhost,127.0.0.1,finite-evidence-prometheus,finite-evidence-loki -e GF_PLUGINS_PREINSTALL_DISABLED=true -e GF_AUTH_ANONYMOUS_ENABLED=true -e GF_AUTH_ANONYMOUS_ORG_ROLE=Viewer -e GF_AUTH_DISABLE_LOGIN_FORM=true -e GF_USERS_DEFAULT_THEME=dark -e GF_ANALYTICS_REPORTING_ENABLED=false -e GF_ANALYTICS_CHECK_FOR_UPDATES=false -e GF_NEWS_NEWS_FEED_ENABLED=false -v "$root/provisioning:/etc/grafana/provisioning:ro" -v "$root/dashboards:/var/lib/grafana/dashboards:ro" grafana/grafana:13.0.2
