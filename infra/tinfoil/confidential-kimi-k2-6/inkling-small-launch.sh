#!/usr/bin/env bash
set -euo pipefail

: "${INKLING_MODEL_PATH:?set INKLING_MODEL_PATH to the mounted NVFP4 checkpoint}"

INKLING_REPLICA_COUNT="${INKLING_REPLICA_COUNT:-4}"
INKLING_MAX_ACTIVE_PER_REPLICA="${INKLING_MAX_ACTIVE_PER_REPLICA:-256}"
INKLING_MAX_NUM_SEQS="${INKLING_MAX_NUM_SEQS:-}"
INKLING_MAX_NUM_BATCHED_TOKENS="${INKLING_MAX_NUM_BATCHED_TOKENS:-}"
INKLING_GPU_MEMORY_UTILIZATION="${INKLING_GPU_MEMORY_UTILIZATION:-0.80}"
INKLING_SPECULATIVE_CONFIG="${INKLING_SPECULATIVE_CONFIG:-}"

if ((INKLING_REPLICA_COUNT < 1 || INKLING_REPLICA_COUNT > 4)); then
  echo "INKLING_REPLICA_COUNT must be between 1 and 4" >&2
  exit 2
fi

ulimit -n 65536
export VLLM_USE_V2_MODEL_RUNNER=1
export FLASH_ATTENTION_CUTE_DSL_CACHE_ENABLED=1
export PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True

pids=()
cleanup() {
  if ((${#pids[@]})); then
    kill -TERM "${pids[@]}" 2>/dev/null || true
    wait "${pids[@]}" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

start_replica() {
  local gpus="$1"
  local port="$2"
  local args=(
    serve "$INKLING_MODEL_PATH"
    --host 127.0.0.1
    --port "$port"
    --served-model-name inkling-small glm-5-2
    --tokenizer-mode inkling
    --reasoning-parser inkling
    --tool-call-parser inkling
    --enable-auto-tool-choice
    --tensor-parallel-size 2
    --kernel-config.enable_flashinfer_autotune=False
    --trust-remote-code
    --max-model-len 1048576
    --gpu-memory-utilization "$INKLING_GPU_MEMORY_UTILIZATION"
    --enable-prefix-caching
  )

  if [[ -n "$INKLING_MAX_NUM_SEQS" ]]; then
    args+=(--max-num-seqs "$INKLING_MAX_NUM_SEQS")
  fi
  if [[ -n "$INKLING_MAX_NUM_BATCHED_TOKENS" ]]; then
    args+=(--max-num-batched-tokens "$INKLING_MAX_NUM_BATCHED_TOKENS")
  fi
  if [[ -n "$INKLING_SPECULATIVE_CONFIG" ]]; then
    args+=(--speculative-config "$INKLING_SPECULATIVE_CONFIG")
  fi

  CUDA_VISIBLE_DEVICES="$gpus" /usr/local/bin/vllm "${args[@]}" \
    >"/models/inkling-$port.log" 2>&1 &
  pids+=("$!")
}

gpu_pairs=("0,1" "2,3" "4,5" "6,7")
ports=(8001 8002 8003 8004)

backend_urls=()
for ((index = 0; index < INKLING_REPLICA_COUNT; index++)); do
  start_replica "${gpu_pairs[$index]}" "${ports[$index]}"
  backend_urls+=("http://127.0.0.1:${ports[$index]}")
done

for ((index = 0; index < INKLING_REPLICA_COUNT; index++)); do
  ready=0
  for _ in $(seq 1 1800); do
    if curl -sf "http://127.0.0.1:${ports[$index]}/health" >/dev/null; then
      ready=1
      break
    fi
    if ! kill -0 "${pids[$index]}" 2>/dev/null; then
      echo "Inkling replica on port ${ports[$index]} exited during startup" >&2
      exit 1
    fi
    sleep 2
  done
  if [[ "$ready" != 1 ]]; then
    echo "Inkling replica on port ${ports[$index]} did not become healthy" >&2
    exit 1
  fi
done

joined_backends="$(IFS=,; echo "${backend_urls[*]}")"
INKLING_BACKENDS="$joined_backends" \
INKLING_MAX_ACTIVE_PER_BACKEND="$INKLING_MAX_ACTIVE_PER_REPLICA" \
  /usr/bin/python3 /models/inkling-small-router.py &
pids+=("$!")

wait -n "${pids[@]}"
echo "An Inkling replica or its router exited unexpectedly" >&2
exit 1
