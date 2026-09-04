#!/bin/sh
set -eu

: "${VLLM_API_KEY:?VLLM_API_KEY is required for private SGLang authentication}"

exec sglang serve --api-key "$VLLM_API_KEY" "$@"
