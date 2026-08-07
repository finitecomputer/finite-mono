#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/../.." && pwd)
# shellcheck source=../lib/devfinity-brain-readiness.sh
source "$repo_root/scripts/lib/devfinity-brain-readiness.sh"

test_root="$(mktemp -d)"
trap 'rm -rf -- "$test_root"' EXIT

state_dir="$test_root/state"
mkdir -p "$state_dir"
container_machine_id="test-runtime"
FINITE_BRAIN_URL="http://127.0.0.1:1"
chat_state_json="$state_dir/chat-state.json"
printf '{"hosted_agent_binding":{"canonical_room_id":"agent"},"rooms":[{"room_id":"agent","state":"Connected","is_agent_chat":true}]}' >"$chat_state_json"
DEVFINITY_BRAIN_READY_POLL_SECONDS=0
runtime_started_after="2026-07-31T12:00:00.500Z"

chat_state() {
  return 0
}

curl() {
  printf '{"status":"ok"}\n'
}

runtime_exec_calls=0
runtime_exec() {
  runtime_exec_calls=$((runtime_exec_calls + 1))
  local command="${*: -3}"
  if [[ "$command" == "daemon status --json" ]]; then
    local attempt=$(((runtime_exec_calls + 2) / 3))
    if (( attempt == 1 )); then
      printf '{"state":"reconnecting","notificationStatus":"reconnecting","lastTickAt":null,"lastError":"connection refused","tickCount":0}\n'
    else
      printf '{"state":"running","notificationStatus":null,"lastTickAt":null,"lastError":null,"tickCount":0}\n'
    fi
  elif [[ "$command" == "sync status --json" ]]; then
    if (( runtime_exec_calls < 8 )); then
      printf '{"status":"reconnecting","latestSequence":0}\n'
    else
      printf '{"status":"idle-no-local-changes","latestSequence":4}\n'
    fi
  elif [[ "$command" == "fbrain activity --json" ]]; then
    if (( runtime_exec_calls < 9 )); then
      printf '[{"at":"2026-07-31T11:59:59Z","kind":"daemon.supervise.notification"}]\n'
    else
      printf '[{"at":"2026-07-31T12:00:00Z","kind":"daemon.supervise.notification"}]\n'
    fi
  else
    printf '{"brains":[]}\n'
  fi
}

wait_for_brain_runtime_ready_after_restart /workspace/brain 2
if (( runtime_exec_calls < 9 )); then
  echo "readiness gate did not wait for both daemon and sync convergence" >&2
  exit 1
fi

runtime_exec_calls=0
runtime_exec() {
  runtime_exec_calls=$((runtime_exec_calls + 1))
  printf 'connection refused\n' >&2
  return 1
}

if wait_for_brain_runtime_ready_after_restart /workspace/brain 1 \
  >"$test_root/persistent.stdout" 2>"$test_root/persistent.stderr"; then
  echo "readiness gate unexpectedly accepted a persistent connection failure" >&2
  exit 1
fi
grep -Fq "did not complete a healthy post-restart sync within 1s" "$test_root/persistent.stderr"
grep -Fq "Brain readiness diagnostics: post-restart supervisor convergence" "$test_root/persistent.stderr"
grep -Fq "hosted-device:" "$test_root/persistent.stderr"
grep -Fq "finite-brain health:" "$test_root/persistent.stderr"

runtime_exec_calls=0
runtime_exec() {
  runtime_exec_calls=$((runtime_exec_calls + 1))
  if (( runtime_exec_calls < 3 )); then
    printf 'connection refused\n' >&2
    return 1
  fi
  printf '{"brains":[]}\n'
}
wait_for_brain_dependency_ready "FiniteBrain reset dependency" 2
if (( runtime_exec_calls != 3 )); then
  echo "dependency gate did not wait for Runtime-to-Brain reachability" >&2
  exit 1
fi

# wait_for_agent_brain_list must poll until the agent-visible list converges;
# a healthy Brain server does not imply the supervisor has re-synced yet.
brains_json="$state_dir/agent-brains.json"
runtime_exec_calls=0
runtime_exec() {
  runtime_exec_calls=$((runtime_exec_calls + 1))
  if (( runtime_exec_calls < 3 )); then
    printf '{"brains":[]}\n'
  else
    printf '{"brains":[{"brainId":"brain-1","kind":"personal","role":"personal_agent"}]}\n'
  fi
}
wait_for_agent_brain_list personal personal_agent 1 5
if (( runtime_exec_calls < 3 )); then
  echo "convergence gate did not wait for the agent-visible Brain list" >&2
  exit 1
fi

runtime_exec_calls=0
runtime_exec() {
  runtime_exec_calls=$((runtime_exec_calls + 1))
  printf '{"brains":[]}\n'
}
if wait_for_agent_brain_list personal personal_agent 1 1 \
  >"$test_root/list.stdout" 2>"$test_root/list.stderr"; then
  echo "convergence gate unexpectedly accepted an empty Brain list" >&2
  exit 1
fi
grep -Fq "did not converge to 1 personal/personal_agent within 1s" "$test_root/list.stderr"
grep -Fq "expected 1 agent-visible Brains" "$test_root/list.stderr"

echo "devfinity Brain readiness tests passed"
