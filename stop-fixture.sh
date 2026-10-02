#!/usr/bin/env bash
set -euo pipefail
docker rm -f finite-evidence-grafana finite-evidence-prometheus finite-evidence-loki
docker network rm finite-evidence-net
