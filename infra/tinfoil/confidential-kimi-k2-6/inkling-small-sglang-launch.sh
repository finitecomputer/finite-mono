#!/usr/bin/env bash
set -euo pipefail

# Exact upstream verified H200/NVFP4 baseline, with only the model path,
# bind address, port, and served name made deployment-configurable.
: "${INKLING_MODEL_PATH:?set INKLING_MODEL_PATH to the mounted NVFP4 checkpoint}"

INKLING_HOST="${INKLING_HOST:-0.0.0.0}"
INKLING_PORT="${INKLING_PORT:-8000}"
INKLING_SERVED_MODEL_NAME="${INKLING_SERVED_MODEL_NAME:-inkling-small}"
INKLING_MEM_FRACTION_STATIC="${INKLING_MEM_FRACTION_STATIC:-0.85}"

ulimit -n 65536
export SGLANG_ENABLE_UNIFIED_RADIX_TREE=1

exec sglang serve \
  --trust-remote-code \
  --model-path "$INKLING_MODEL_PATH" \
  --served-model-name "$INKLING_SERVED_MODEL_NAME" \
  --tp 8 \
  --quantization modelopt_fp4 \
  --attention-backend fa4 \
  --page-size 128 \
  --fp4-gemm-backend marlin \
  --moe-runner-backend marlin \
  --enable-torch-symm-mem \
  --mamba-radix-cache-strategy extra_buffer \
  --mem-fraction-static "$INKLING_MEM_FRACTION_STATIC" \
  --swa-full-tokens-ratio 0.1 \
  --mamba-full-memory-ratio 0.1 \
  --enable-multimodal \
  --reasoning-parser inkling \
  --tool-call-parser inkling \
  --host "$INKLING_HOST" \
  --port "$INKLING_PORT"
