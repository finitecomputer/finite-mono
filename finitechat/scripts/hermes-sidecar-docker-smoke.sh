#!/usr/bin/env bash
# Docker runtime smoke for the Finite Chat Hermes sidecar.
#
# Builds containers/agent/Dockerfile, starts the real Hermes gateway in Docker,
# admits finitechat CLI users through invite URL before and after restore,
# snapshots/restores agent state through restic, and writes a JSON report.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ENV_FILE="${FINITE_HERMES_SMOKE_ENV_FILE:-$REPO_ROOT/.env}"
if [[ -f "$ENV_FILE" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$ENV_FILE"
    set +a
fi

if [[ -z "${AWS_ACCESS_KEY_ID:-}" && -n "${FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID:-}" ]]; then
    export AWS_ACCESS_KEY_ID="$FINITE_DOCKER_RESTIC_AWS_ACCESS_KEY_ID"
fi
if [[ -z "${AWS_SECRET_ACCESS_KEY:-}" && -n "${FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY:-}" ]]; then
    export AWS_SECRET_ACCESS_KEY="$FINITE_DOCKER_RESTIC_AWS_SECRET_ACCESS_KEY"
fi
if [[ -z "${AWS_SESSION_TOKEN:-}" && -n "${FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN:-}" ]]; then
    export AWS_SESSION_TOKEN="$FINITE_DOCKER_RESTIC_AWS_SESSION_TOKEN"
fi
if [[ -z "${AWS_REGION:-}" && -n "${FINITE_DOCKER_RESTIC_AWS_REGION:-}" ]]; then
    export AWS_REGION="$FINITE_DOCKER_RESTIC_AWS_REGION"
fi
if [[ -z "${AWS_DEFAULT_REGION:-}" && -n "${FINITE_DOCKER_RESTIC_AWS_DEFAULT_REGION:-}" ]]; then
    export AWS_DEFAULT_REGION="$FINITE_DOCKER_RESTIC_AWS_DEFAULT_REGION"
fi
if [[ "${FINITE_DOCKER_RESTIC_USE_AWS_SHARED_CONFIG:-1}" != "0" ]]; then
    eval "$("$REPO_ROOT/scripts/hermes-restic-preflight.py" \
        --aws-profile "${AWS_PROFILE:-default}" \
        --export-aws-shared-env)"
fi
if [[ "${FINITE_DOCKER_RESTIC_BACKEND:-local}" == "s3" && -z "${FINITE_DOCKER_RESTIC_REPOSITORY:-}" && -n "${FINITE_LATITUDE_STORAGE_BUCKET:-}" ]]; then
    LATITUDE_ENDPOINT="${FINITE_LATITUDE_OBJECT_ENDPOINT:-https://objects.nyc.storage.sh}"
    LATITUDE_PREFIX="${FINITE_DOCKER_RESTIC_PREFIX:?set FINITE_DOCKER_RESTIC_PREFIX when deriving an S3 restic repository from FINITE_LATITUDE_STORAGE_BUCKET}"
    export FINITE_DOCKER_RESTIC_REPOSITORY="s3:${LATITUDE_ENDPOINT%/}/${FINITE_LATITUDE_STORAGE_BUCKET}/${LATITUDE_PREFIX#/}"
fi

REPORT="${FINITE_HERMES_DOCKER_SMOKE_REPORT:-$REPO_ROOT/target/hermes-docker-smoke/report.json}"
HERMES_VERSION="${FINITE_HERMES_AGENT_VERSION:-0.18.2}"
RESTIC_BACKEND="${FINITE_DOCKER_RESTIC_BACKEND:-local}"
RESTIC_PREFLIGHT_REPORT="${FINITE_HERMES_DOCKER_RESTIC_PREFLIGHT_REPORT:-$(dirname "$REPORT")/restic-preflight.json}"
COMMAND=(
    python3 -m unittest
    tests.container.test_agent_docker_e2e -v
)

cd "$REPO_ROOT"
mkdir -p "$(dirname "$REPORT")"

scripts/hermes-restic-preflight.py --report "$RESTIC_PREFLIGHT_REPORT"

set +e
FINITE_DOCKER_E2E=1 \
FINITE_HERMES_AGENT_VERSION="$HERMES_VERSION" \
FINITE_HERMES_DOCKER_SMOKE_REPORT="$REPORT" \
"${COMMAND[@]}"
status=$?
set -e

if [[ "$status" -ne 0 ]]; then
    python3 - "$REPORT" "$status" "$HERMES_VERSION" "$RESTIC_BACKEND" <<'PY'
import json
import pathlib
import sys
import time

path = pathlib.Path(sys.argv[1])
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps({
    "status": "failed",
    "exit_code": int(sys.argv[2]),
    "name": "docker_real_gateway_admission_and_restore",
    "generated_at_unix": int(time.time()),
    "command": "scripts/hermes-sidecar-docker-smoke.sh",
    "hermes_agent_version_expected": sys.argv[3],
    "backup_backend": "restic",
    "restic_backend": sys.argv[4],
}, indent=2) + "\n")
PY
    echo "Hermes Docker smoke failed; report: $REPORT" >&2
    exit "$status"
fi

python3 - "$REPORT" <<'PY'
import json
import pathlib
import sys
import time

path = pathlib.Path(sys.argv[1])
data = json.loads(path.read_text())
data["generated_at_unix"] = int(time.time())
data["command"] = "scripts/hermes-sidecar-docker-smoke.sh"
path.write_text(json.dumps(data, indent=2) + "\n")
PY

echo "Hermes Docker smoke passed; report: $REPORT"
cat "$REPORT"
