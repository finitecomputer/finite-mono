#!/usr/bin/env bash
# Docker runtime smoke against a local S3-compatible MinIO endpoint.
#
# This exercises the same restic S3 code path as Latitude without using real
# object-storage credentials. It is not a substitute for the actual Latitude S3
# gate; it writes a separate report under target/hermes-docker-s3-emulator-smoke.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

REPORT="${FINITE_HERMES_DOCKER_S3_EMULATOR_REPORT:-$REPO_ROOT/target/hermes-docker-s3-emulator-smoke/report.json}"
PREFLIGHT_REPORT="${FINITE_HERMES_DOCKER_S3_EMULATOR_PREFLIGHT_REPORT:-$(dirname "$REPORT")/restic-preflight.json}"
MINIO_IMAGE="${FINITE_S3_EMULATOR_MINIO_IMAGE:-quay.io/minio/minio:latest}"
MC_IMAGE="${FINITE_S3_EMULATOR_MC_IMAGE:-quay.io/minio/mc:latest}"
MINIO_CONTAINER="${FINITE_S3_EMULATOR_CONTAINER:-finite-hermes-minio-$(date +%s)}"
MINIO_ACCESS_KEY="${FINITE_S3_EMULATOR_ACCESS_KEY:-finite-minio-access}"
MINIO_SECRET_KEY="${FINITE_S3_EMULATOR_SECRET_KEY:-finite-minio-secret}"
MINIO_BUCKET="${FINITE_S3_EMULATOR_BUCKET:-finite-hermes-runtime-smoke}"
MINIO_PREFIX="${FINITE_S3_EMULATOR_PREFIX:-agent-runtimes/tinfoil-canary-001/restic}"
MINIO_PORT="${FINITE_S3_EMULATOR_PORT:-$(python3 - <<'PY'
import socket

sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
)}"
RESTIC_PASSWORD="${FINITE_S3_EMULATOR_RESTIC_PASSWORD:-finite-s3-emulator-restic-key}"

cleanup() {
    docker rm -f "$MINIO_CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cd "$REPO_ROOT"
mkdir -p "$(dirname "$REPORT")"

docker rm -f "$MINIO_CONTAINER" >/dev/null 2>&1 || true
docker run \
    --detach \
    --name "$MINIO_CONTAINER" \
    --publish "127.0.0.1:${MINIO_PORT}:9000" \
    --env "MINIO_ROOT_USER=$MINIO_ACCESS_KEY" \
    --env "MINIO_ROOT_PASSWORD=$MINIO_SECRET_KEY" \
    "$MINIO_IMAGE" server /data >/dev/null

python3 - "$MINIO_PORT" <<'PY'
import sys
import time
import urllib.request

port = sys.argv[1]
deadline = time.monotonic() + 60
url = f"http://127.0.0.1:{port}/minio/health/ready"
while time.monotonic() < deadline:
    try:
        with urllib.request.urlopen(url, timeout=2) as response:
            if response.status == 200:
                raise SystemExit(0)
    except Exception:
        time.sleep(0.25)
raise SystemExit(f"MinIO at {url} did not become ready")
PY

docker run --rm \
    --network "container:$MINIO_CONTAINER" \
    --env "MC_HOST_local=http://${MINIO_ACCESS_KEY}:${MINIO_SECRET_KEY}@127.0.0.1:9000" \
    "$MC_IMAGE" mb --ignore-existing "local/$MINIO_BUCKET" >/dev/null

FINITE_HERMES_DOCKER_SMOKE_REPORT="$REPORT" \
FINITE_HERMES_DOCKER_RESTIC_PREFLIGHT_REPORT="$PREFLIGHT_REPORT" \
FINITE_DOCKER_RESTIC_BACKEND=s3 \
FINITE_DOCKER_RESTIC_REPOSITORY="s3:http://host.docker.internal:${MINIO_PORT}/${MINIO_BUCKET}/${MINIO_PREFIX}" \
FINITE_DOCKER_RESTIC_PASSWORD="$RESTIC_PASSWORD" \
AWS_ACCESS_KEY_ID="$MINIO_ACCESS_KEY" \
AWS_SECRET_ACCESS_KEY="$MINIO_SECRET_KEY" \
AWS_DEFAULT_REGION=us-east-1 \
AWS_REGION=us-east-1 \
    scripts/hermes-sidecar-docker-smoke.sh

python3 - "$REPORT" "$PREFLIGHT_REPORT" "$MINIO_IMAGE" "$MC_IMAGE" "$MINIO_BUCKET" "$MINIO_PREFIX" "$MINIO_PORT" <<'PY'
import json
import pathlib
import sys

report_path = pathlib.Path(sys.argv[1])
preflight_path = pathlib.Path(sys.argv[2])
data = json.loads(report_path.read_text(encoding="utf-8"))
facts = data.setdefault("facts", {})
facts["s3_endpoint_kind"] = "local_emulator"
facts["s3_emulator"] = {
    "server_image": sys.argv[3],
    "client_image": sys.argv[4],
    "bucket": sys.argv[5],
    "prefix": sys.argv[6],
    "endpoint": f"http://127.0.0.1:{sys.argv[7]}",
}
data["command"] = "scripts/hermes-sidecar-docker-s3-emulator-smoke.sh"
data["preflight_report"] = str(preflight_path)
report_path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
PY

echo "Hermes Docker S3 emulator smoke passed; report: $REPORT"
cat "$REPORT"
