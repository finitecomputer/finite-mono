#!/usr/bin/env bash
set -euo pipefail

host="${1:-lat2}"

ssh -o BatchMode=yes -o ConnectTimeout=10 "$host" '
set -e
printf "host=%s\n" "$(hostname)"
printf "user=%s\n" "$(whoami)"
printf "kernel=%s\n" "$(uname -srmo)"
printf "cpus=%s\n" "$(nproc)"
free -h | sed -n "1,2p"
df -h / | sed -n "1,2p"
printf "docker_path=%s\n" "$(command -v docker || true)"
if command -v docker >/dev/null 2>&1; then
  docker --version || true
  systemctl is-active docker 2>/dev/null || true
  if docker compose version >/dev/null 2>&1; then
    docker compose version
  else
    echo "docker_compose=missing"
  fi
fi
printf "podman_path=%s\n" "$(command -v podman || true)"
if command -v podman >/dev/null 2>&1; then
  podman --version || true
fi
printf "kubectl_path=%s\n" "$(command -v kubectl || true)"
if command -v kubectl >/dev/null 2>&1; then
  kubectl version --client=true 2>/dev/null | head -5 || true
fi
'
