#!/usr/bin/env bash
set -euo pipefail
spike="$(cd "$(dirname "$0")/.." && pwd)"
docker build -f "$spike/notifications/Dockerfile" --build-arg "BASE_IMAGE=${SPIKE_BASE_IMAGE:?}" -t "${SPIKE_IMAGE:?}" "$spike"
docker push "$SPIKE_IMAGE"
