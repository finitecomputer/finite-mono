#!/usr/bin/env bash
set -euo pipefail

: "${LAGUNA_MODEL_PATH:?set LAGUNA_MODEL_PATH to the mounted FP8 checkpoint}"
: "${LAGUNA_DRAFT_PATH:?set LAGUNA_DRAFT_PATH to the mounted FP8 DFlash checkpoint}"

# The router needs one inbound and one upstream socket per active request.
# Keep the process limit comfortably above the 512-request admission ceiling.
ulimit -n 65536

export PYTHONNOUSERSITE=1
export VLLM_BLOCKSCALE_FP8_GEMM_FLASHINFER=0
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
  CUDA_VISIBLE_DEVICES="$gpus" /usr/local/bin/vllm serve "$LAGUNA_MODEL_PATH" \
    --host 127.0.0.1 \
    --port "$port" \
    --served-model-name laguna-s-2.1 glm-5-2 \
    --trust-remote-code \
    --tensor-parallel-size 2 \
    --disable-custom-all-reduce \
    --enforce-eager \
    --max-model-len 1048576 \
    --kv-cache-dtype bfloat16 \
    --gpu-memory-utilization 0.90 \
    --max-num-seqs 128 \
    --max-num-batched-tokens 16384 \
    --enable-prefix-caching \
    --enable-chunked-prefill \
    --enable-auto-tool-choice \
    --tool-call-parser poolside_v1 \
    --reasoning-parser poolside_v1 \
    --generation-config auto \
    --moe-backend triton \
    --speculative-config \
      "{\"model\":\"$LAGUNA_DRAFT_PATH\",\"num_speculative_tokens\":15,\"method\":\"dflash\"}" \
    >"/tmp/laguna-$port.log" 2>&1 &
  pids+=("$!")
}

start_replica "0,1" 8001
start_replica "2,3" 8002
start_replica "4,5" 8003
start_replica "6,7" 8004

for port in 8001 8002 8003 8004; do
  ready=0
  for _ in $(seq 1 1800); do
    if curl -sf "http://127.0.0.1:$port/health" >/dev/null; then
      ready=1
      break
    fi
    sleep 2
  done
  if [[ "$ready" != 1 ]]; then
    echo "Laguna replica on port $port did not become healthy" >&2
    exit 1
  fi
done

/usr/bin/python3 /usr/local/bin/laguna-s21-router.py &
pids+=("$!")

# Any unexpected child exit restarts the whole Tinfoil container. This is
# preferable to silently serving with reduced capacity indefinitely; the
# router keeps ordinary rolling model reloads available when performed by an
# operator outside this fail-fast launcher.
wait -n "${pids[@]}"
echo "A Laguna replica or its router exited unexpectedly" >&2
exit 1
