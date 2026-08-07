# Inkling Small on eight H200s

Date: 2026-08-07

Status: active isolated lab study. Production on `control.inf9.tinfoil.sh` was
not changed. Tests use the disposable, debug-enabled `gpu-lab` container on
`control.inf12.tinfoil.sh` with no production secrets or traffic.

## Fixed identity and intended topology

- Model: `thinkingmachines/Inkling-Small-NVFP4`.
- Revision: `b6a99534467840620d411e4cd4ad5819b2610d9c`.
- Checkpoint verification: 21 remote model files and all checksums passed.
- Model size: about 160 GiB locally, including the optional MTP checkpoint.
- Architecture: 276B total parameters, 12B active, native multimodal input,
  and a 1,048,576-token service context.
- H200 numerical path: TP2 W4A16, with NVFP4 expert weights dequantized to
  BF16 on the fly. This is the H200 path documented by the vLLM recipe, not
  Blackwell-native W4A4.
- Intended cluster topology: four independent TP2 replicas on GPU pairs
  `0,1`, `2,3`, `4,5`, and `6,7`, behind the checked-in least-active streaming
  router.
- Baseline speculation: disabled. The bundled MTP heads are tested only after
  target-only output passes protocol and quality gates.

The executable configuration is preserved in
[`inkling-small-launch.sh`](../../infra/tinfoil/confidential-kimi-k2-6/inkling-small-launch.sh)
and
[`inkling-small-router.py`](../../infra/tinfoil/confidential-kimi-k2-6/inkling-small-router.py).
The deployment-shaped placeholder is
[`tinfoil-config.inkling-small-nvfp4.candidate.yml`](../../infra/tinfoil/confidential-kimi-k2-6/tinfoil-config.inkling-small-nvfp4.candidate.yml).

## Measured startup findings

One TP2 replica at the official/default `gpu_memory_utilization=0.90` loaded
the weights but overcommitted during final KV allocation. At `0.80`, vLLM
reported 30.92 GiB of available KV memory, 1,264,858 cache tokens, and 1.21x
maximum concurrency at the full 1,048,576-token request length. Weight loading
took 40.38 seconds after distributed initialization. This is the current safe
full-context memory setting.

The inherited DeepSeek image was not a valid Inkling runtime. Layering PyPI
vLLM 0.26.0 into it exposed these dependency defects in order:

1. `flashinfer-python` 0.6.14 versus `flashinfer-cubin` 0.6.13;
2. missing SciPy required by the multimodal vision tower;
3. Quack 0.5.0 versus CUTLASS DSL 4.6.0; and
4. corrupt inference: every generated output token was ID 1023.

The first three were sufficient to make the HTTP server healthy, but the raw
token gate proved the runtime numerically invalid. No throughput result from
that mixed image is accepted. The next lab step is the immutable official
Inkling CUDA 13 image, amd64 manifest digest
`sha256:9b001250ef36000b7075327656485a0dfc248e9bc69c855283a8f0690d9b26ba`.
A production image must be rebuilt and pinned by Finite with a complete,
internally consistent lock; it must not install packages at boot.

## Launch shape

Each replica uses:

```text
VLLM_USE_V2_MODEL_RUNNER=1
FLASH_ATTENTION_CUTE_DSL_CACHE_ENABLED=1
--tokenizer-mode inkling
--reasoning-parser inkling
--tool-call-parser inkling
--enable-auto-tool-choice
--tensor-parallel-size 2
--kernel-config.enable_flashinfer_autotune=False
--trust-remote-code
--max-model-len 1048576
--gpu-memory-utilization 0.80
--enable-prefix-caching
```

The scheduler defaults observed in vLLM 0.26.0 were 256 sequences and 8,192
batched tokens. These remain the initial candidate until correct output and a
repeatable concurrency sweep are recorded.

## Gates still required

- normal chat content, separated reasoning, and structured automatic tool call;
- raw token sanity check, including rejection of repeated-token collapse;
- single-stream 1,024-token latency and output rate;
- four-replica unique-prompt concurrency sweep;
- near-limit context prefill;
- post-soak protocol recheck and server-log error scan; and
- optional MTP A/B only if target-only output is correct and acceptance plus a
  hosted-reference quality sample show no regression.

Primary sources:

- https://huggingface.co/thinkingmachines/Inkling-Small-NVFP4
- https://recipes.vllm.ai/thinkingmachines/Inkling-Small
- https://thinkingmachines.ai/news/introducing-inkling/

## Production boundary

This is candidate evidence, not rollout authority. Publishing a Finite image,
wrapping the checkpoint into an MPK, replacing candidate placeholders, adding
secrets or production traffic, or relaunching production requires its own
review and explicit authorization. The existing limiter/auth/accounting path
must remain the public authority.
