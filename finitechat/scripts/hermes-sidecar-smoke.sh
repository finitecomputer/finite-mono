#!/usr/bin/env bash
# Human-facing Hermes sidecar smoke.
#
# Exercises the strongest local encrypted flow:
#   live finitechat server -> agent home -> invite URL -> user join
#   -> finitechat hermes serve -> /v1/hermes/inbound NDJSON
#   -> ack/drain -> agent reply -> user decrypts.
#
# Writes a JSON evidence report for runbooks, CI artifacts, and Docker/Tinfoil
# baseline comparisons.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REPORT="${FINITE_HERMES_SIDECAR_SMOKE_REPORT:-$REPO_ROOT/target/hermes-sidecar-smoke/report.json}"
COMMAND=(
    cargo test -p finitechat-cli --test hermes_flow
    hermes_cli_inits_invites_admits_and_round_trips_messages
    -- --nocapture
)

cd "$REPO_ROOT"
mkdir -p "$(dirname "$REPORT")"

set +e
FINITE_HERMES_SIDECAR_SMOKE_REPORT="$REPORT" "${COMMAND[@]}"
status=$?
set -e

if [[ "$status" -ne 0 ]]; then
    python3 - "$REPORT" "$status" <<'PY'
import json
import pathlib
import sys
import time

path = pathlib.Path(sys.argv[1])
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps({
    "status": "failed",
    "exit_code": int(sys.argv[2]),
    "name": "hermes_cli_inits_invites_admits_and_round_trips_messages",
    "generated_at_unix": int(time.time()),
    "command": "scripts/hermes-sidecar-smoke.sh",
}, indent=2) + "\n")
PY
    echo "Hermes sidecar smoke failed; report: $REPORT" >&2
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
data["command"] = "scripts/hermes-sidecar-smoke.sh"
data["proof_layers"] = [
    "finitechat-server",
    "finitechat hermes CLI",
    "encrypted client stores",
    "finitechat hermes serve",
    "sidecar /v1/hermes/inbound NDJSON",
    "ack/drain",
    "agent reply",
    "user decrypt",
]
path.write_text(json.dumps(data, indent=2) + "\n")
PY

echo "Hermes sidecar smoke passed; report: $REPORT"
cat "$REPORT"
