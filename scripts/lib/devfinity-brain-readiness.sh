# Shared semantic readiness gates for devfinity's Brain lifecycle smoke.

brain_readiness_status_is_ready() {
  local daemon_status_json="$1"
  local sync_status_json="$2"
  local activity_json="$3"
  local runtime_started_at="$4"
  node - "$daemon_status_json" "$sync_status_json" "$activity_json" "$runtime_started_at" <<'JS'
const fs = require("node:fs");
const daemon = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const sync = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
const activity = JSON.parse(fs.readFileSync(process.argv[4], "utf8"));
const runtimeStartedAt = Date.parse(process.argv[5]);
// `daemon supervise` owns multiple Working Trees and intentionally leaves the
// single-tree tick fields at zero. Its successful per-tree sync activity is
// the existing durable proof that this Runtime instance completed catch-up.
const completedPostRestartSync = Array.isArray(activity) && activity.some((entry) =>
  entry.kind === "daemon.supervise.notification"
  && Math.floor(Date.parse(entry.at) / 1000) >= Math.floor(runtimeStartedAt / 1000)
);

const ready = daemon.state === "running"
  && daemon.lastError == null
  && daemon.notificationStatus !== "reconnecting"
  && typeof sync.status === "string"
  && sync.status.length > 0
  && sync.status !== "reconnecting"
  && !String(sync.status || "").startsWith("blocked")
  && Number.isFinite(runtimeStartedAt)
  && completedPostRestartSync;
process.exit(ready ? 0 : 1);
JS
}

brain_dependency_status_is_ready() {
  local brain_list_json="$1"
  node - "$brain_list_json" <<'JS'
const fs = require("node:fs");
const response = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
process.exit(Array.isArray(response.brains) ? 0 : 1);
JS
}

agent_brain_list_matches() {
  local brain_list_json="$1"
  local expected_kind="$2"
  local expected_role="$3"
  local expected_count="$4"
  node - "$brain_list_json" "$expected_kind" "$expected_role" "$expected_count" <<'JS'
const fs = require("node:fs");
const response = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const [kind, role, countText] = process.argv.slice(3);
const brains = response.brains || [];
const count = Number(countText);
if (brains.length !== count) {
  throw new Error(`expected ${count} agent-visible Brains, received ${JSON.stringify(brains)}`);
}
if (brains.some((brain) => brain.kind !== kind || brain.role !== role)) {
  throw new Error(`expected every Brain to be ${kind}/${role}, received ${JSON.stringify(brains)}`);
}
JS
}

wait_for_agent_brain_list() {
  local expected_kind="$1"
  local expected_role="$2"
  local expected_count="$3"
  local timeout_secs="${4:-120}"
  local started
  local poll_seconds="${DEVFINITY_BRAIN_READY_POLL_SECONDS:-2}"

  # A healthy Brain server does not imply the Runtime supervisor has already
  # re-discovered or re-synced the freshly-reset Brain set. Poll the
  # agent-visible list until it converges instead of asserting once.
  started="$(date +%s)"
  while true; do
    if runtime_exec "$container_machine_id" fbrain brain list --json \
      >"$brains_json" 2>/dev/null \
      && agent_brain_list_matches \
        "$brains_json" "$expected_kind" "$expected_role" "$expected_count" 2>/dev/null; then
      return 0
    fi
    if (( "$(date +%s)" - started >= timeout_secs )); then
      echo "agent-visible Brain list did not converge to ${expected_count} ${expected_kind}/${expected_role} within ${timeout_secs}s" >&2
      # Re-run the plain check once more so the failure stays informative.
      agent_brain_list_matches \
        "$brains_json" "$expected_kind" "$expected_role" "$expected_count"
      return 1
    fi
    sleep "$poll_seconds"
  done
}

print_brain_readiness_diagnostics() {
  local label="$1"
  local working_tree="${2:-}"

  echo "Brain readiness diagnostics: $label" >&2
  echo "runtime restart request epoch=${restart_started:-unavailable} container-before=${runtime_started_before:-unavailable} container-after=${runtime_started_after:-unavailable}" >&2

  if chat_state >/dev/null 2>&1; then
    node - "$chat_state_json" <<'JS' >&2
const fs = require("node:fs");
const state = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const rooms = (state.rooms || []).map((room) => ({
  roomId: room.room_id,
  state: room.state,
  isAgentChat: Boolean(room.is_agent_chat),
}));
console.error(`hosted-device: ${JSON.stringify({ binding: state.hosted_agent_binding || null, rooms })}`);
JS
  else
    echo "hosted-device: state request failed" >&2
  fi

  if [[ -n "$working_tree" ]]; then
    echo "fbrain daemon status:" >&2
    runtime_exec -w "$working_tree" "$container_machine_id" \
      fbrain daemon status --json >&2 || true
    echo "fbrain sync status:" >&2
    runtime_exec -w "$working_tree" "$container_machine_id" \
      fbrain sync status --json >&2 || true
    echo "fbrain recent activity:" >&2
    runtime_exec -w "$working_tree" "$container_machine_id" \
      fbrain activity --json 2>/dev/null | tail -80 >&2 || true
  fi

  echo "finite-brain health:" >&2
  curl -sS --max-time 5 "${FINITE_BRAIN_URL:?}/health" >&2 || true
  echo >&2
}

wait_for_brain_dependency_ready() {
  local label="$1"
  local timeout_secs="${2:-120}"
  local started
  local brain_list_json="$state_dir/brain-runtime-readiness-list.json"
  local brain_list_error="$state_dir/brain-runtime-readiness-list.stderr"
  local poll_seconds="${DEVFINITY_BRAIN_READY_POLL_SECONDS:-1}"

  started="$(date +%s)"
  while true; do
    if runtime_exec "$container_machine_id" fbrain brain list --json \
      >"$brain_list_json" 2>"$brain_list_error" \
      && brain_dependency_status_is_ready "$brain_list_json"; then
      return 0
    fi
    if (( "$(date +%s)" - started >= timeout_secs )); then
      echo "$label did not become reachable from the Agent Runtime within ${timeout_secs}s" >&2
      if [[ -s "$brain_list_error" ]]; then
        echo "fbrain brain list error:" >&2
        sed -n '1,80p' "$brain_list_error" >&2
      fi
      print_brain_readiness_diagnostics "$label"
      return 1
    fi
    sleep "$poll_seconds"
  done
}

wait_for_brain_runtime_ready_after_restart() {
  local working_tree="$1"
  local timeout_secs="${2:-180}"
  local started
  local daemon_status_json="$state_dir/brain-runtime-readiness-daemon.json"
  local daemon_status_error="$state_dir/brain-runtime-readiness-daemon.stderr"
  local sync_status_json="$state_dir/brain-runtime-readiness-sync.json"
  local sync_status_error="$state_dir/brain-runtime-readiness-sync.stderr"
  local activity_json="$state_dir/brain-runtime-readiness-activity.json"
  local activity_error="$state_dir/brain-runtime-readiness-activity.stderr"
  local poll_seconds="${DEVFINITY_BRAIN_READY_POLL_SECONDS:-1}"

  started="$(date +%s)"
  while true; do
    if runtime_exec -w "$working_tree" "$container_machine_id" \
      fbrain daemon status --json >"$daemon_status_json" 2>"$daemon_status_error" \
      && runtime_exec -w "$working_tree" "$container_machine_id" \
        fbrain sync status --json >"$sync_status_json" 2>"$sync_status_error" \
      && runtime_exec -w "$working_tree" "$container_machine_id" \
        fbrain activity --json >"$activity_json" 2>"$activity_error" \
      && brain_readiness_status_is_ready \
        "$daemon_status_json" "$sync_status_json" "$activity_json" \
        "${runtime_started_after:?}"; then
      return 0
    fi
    if (( "$(date +%s)" - started >= timeout_secs )); then
      echo "Brain supervisor did not complete a healthy post-restart sync within ${timeout_secs}s" >&2
      if [[ -s "$daemon_status_error" ]]; then
        echo "fbrain daemon status error:" >&2
        sed -n '1,80p' "$daemon_status_error" >&2
      fi
      if [[ -s "$sync_status_error" ]]; then
        echo "fbrain sync status error:" >&2
        sed -n '1,80p' "$sync_status_error" >&2
      fi
      if [[ -s "$activity_error" ]]; then
        echo "fbrain activity error:" >&2
        sed -n '1,80p' "$activity_error" >&2
      fi
      print_brain_readiness_diagnostics "post-restart supervisor convergence" "$working_tree"
      return 1
    fi
    sleep "$poll_seconds"
  done
}
