#!/usr/bin/env bash
set -euo pipefail

# Reproducible temporary-rack launcher for one-variable DeepSeek V4 tests.
# The defaults exactly match the measured retry-2 candidate; override only the
# variable named by the current experiment.
MODEL_PATH="${MODEL_PATH:-/models/deepseek-v4-flash-0731}"
MAX_BATCHED_TOKENS="${MAX_BATCHED_TOKENS:-2048}"
MAX_NUM_SEQS="${MAX_NUM_SEQS:-128}"
LAB_LOG="${LAB_LOG:-/models/deepseek-lab.log}"

ulimit -n 65536
export PYTHONNOUSERSITE=1
export VLLM_ENGINE_READY_TIMEOUT_S=3600
export VLLM_MEMORY_PROFILER_ESTIMATE_CUDAGRAPHS=0
export PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True

exec /usr/local/bin/vllm serve "${MODEL_PATH}" \
    --host 0.0.0.0 \
    --port 8000 \
    --served-model-name deepseek-v4-flash-0731 glm-5-2 \
    --trust-remote-code \
    --kv-cache-dtype fp8 \
    --block-size 256 \
    --data-parallel-size 8 \
    --enable-expert-parallel \
    --tokenizer-mode deepseek_v4 \
    --tool-call-parser deepseek_v4 \
    --enable-auto-tool-choice \
    --reasoning-parser deepseek_v4 \
    --default-chat-template-kwargs '{"enable_thinking":true}' \
    --enable-prompt-tokens-details \
    --scheduling-policy priority \
    --max-model-len 393216 \
    --max-num-seqs "${MAX_NUM_SEQS}" \
    --max-num-batched-tokens "${MAX_BATCHED_TOKENS}" \
    --gpu-memory-utilization 0.95 \
    --no-enable-flashinfer-autotune \
    --compilation-config '{"mode":0,"cudagraph_mode":"FULL_DECODE_ONLY"}' \
    >"${LAB_LOG}" 2>&1
