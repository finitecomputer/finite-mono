# Inkling Small on eight H200s

Date: 2026-08-07

Status: isolated lab study complete but without a deployable winner. Production
on `control.inf9.tinfoil.sh` was not changed. Tests used the disposable,
debug-enabled `gpu-lab` container on `control.inf12.tinfoil.sh` with no
production secrets or traffic. The host authorization expired before the final
SGLang correctness test could run.

## Fixed identity and intended topology

- Model: `thinkingmachines/Inkling-Small-NVFP4`.
- Revision: `b6a99534467840620d411e4cd4ad5819b2610d9c`.
- Checkpoint verification: 21 remote model files and all checksums passed.
- Model size: about 160 GiB locally, including the optional MTP checkpoint.
- Architecture: 276B total parameters, 12B active, native multimodal input,
  and a 1,048,576-token service context.
- H200 numerical path: Marlin W4A16, with NVFP4 weights dequantized on Hopper
  rather than using Blackwell-native W4A4.
- Current candidate topology: one SGLang TP8 instance across all eight H200s.
  This replaces the investigated four-TP2 vLLM topology because every vLLM
  runtime tested failed output correctness.
- Baseline speculation: disabled. The bundled MTP heads are tested only after
  target-only output passes protocol and quality gates.

The candidate executable configuration is preserved in
[`inkling-small-sglang-launch.sh`](../../infra/tinfoil/confidential-kimi-k2-6/inkling-small-sglang-launch.sh).
The rejected vLLM investigation harness remains in
[`inkling-small-launch.sh`](../../infra/tinfoil/confidential-kimi-k2-6/inkling-small-launch.sh)
and [`inkling-small-router.py`](../../infra/tinfoil/confidential-kimi-k2-6/inkling-small-router.py)
for reproducibility only.
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
that mixed image is accepted.

Three clean image paths were then measured and preserved as immutable lab
releases:

1. `v2026-08-07-gpu-lab-inkling-1`, the July 15 dedicated Inkling image at
   amd64 digest `sha256:9b001250ef36000b7075327656485a0dfc248e9bc69c855283a8f0690d9b26ba`,
   did not recognize the current Small checkpoint's quantization and aborted.
2. `v2026-08-07-gpu-lab-inkling-2`, clean official vLLM 0.26.0 at digest
   `sha256:770fe02d83d8a1b6034719273fc02d52d8d90cbb2b2e0392580c990c27ff4ae0`,
   recognized `modelopt_fp4` but generated one varied token followed by token
   ID 1023 repeatedly. Forcing eager execution produced the same corruption,
   ruling out CUDA graph capture. The FlashInfer CUTLASS and CuteDSL MoE
   alternatives rejected this checkpoint/backend combination rather than
   running it incorrectly.
3. `v2026-08-07-gpu-lab-inkling-3`, the then-current official vLLM nightly at
   digest `sha256:6877023dee3a2456e00f468813607fd4ec21cd92c6386e5433e2f7422bf087a8`
   (vLLM commit `c810e5ee9`), loaded successfully with Marlin but generated only
   token ID 1023, rendered as repeated `they` text.

These failures were reproduced with the exact checkpoint after all 21 tracked
files and checksums passed verification. They are serving-runtime failures,
not evidence of a damaged download. A healthy HTTP endpoint is therefore not
an acceptable Inkling readiness gate.

The official SGLang source contains a separately verified H200/NVFP4 recipe:
TP8, FA4 attention, Marlin FP4 GEMM and MoE, unified radix cache, and 0.85 static
memory fraction. Its CUDA 13 image was pinned to the linux/amd64 manifest
`sha256:b90c0d760a65bc4dbbe4520bea966c437cc40391dcb7cca2a74922985dc1abeb`
and measured as lab release `v2026-08-07-gpu-lab-inkling-4`. The temporary host
authorization expired during deployment, before the model could be downloaded
and the raw-output gate run. This is the correct first recipe for the next lab,
but it is not a Finite-validated result yet.

## Candidate SGLang launch shape

The next correctness attempt must use one TP8 instance with:

```text
SGLANG_ENABLE_UNIFIED_RADIX_TREE=1
--tp 8
--quantization modelopt_fp4
--attention-backend fa4
--page-size 128
--fp4-gemm-backend marlin
--moe-runner-backend marlin
--enable-torch-symm-mem
--mamba-radix-cache-strategy extra_buffer
--mem-fraction-static 0.85
--swa-full-tokens-ratio 0.1
--mamba-full-memory-ratio 0.1
--enable-multimodal
--reasoning-parser inkling
--tool-call-parser inkling
```

Do not add MTP or DSpark speculation to the first run. The official recipe notes
that Inkling Small's MTP needs multi-layer EAGLE; an incomplete speculative
configuration can itself generate garbage. On H200, do not use MXFP8 KV cache;
the current official recipe limits that long-context option to Blackwell.

## Gates still required

- normal chat content, separated reasoning, and structured automatic tool call;
- raw token sanity check, including rejection of repeated-token collapse;
- single-stream 1,024-token latency and output rate;
- TP8 unique-prompt concurrency sweep and scheduler-limit sweep;
- near-limit context prefill;
- post-soak protocol recheck and server-log error scan; and
- optional MTP A/B only if target-only output is correct and acceptance plus a
  hosted-reference quality sample show no regression.

Primary sources:

- https://huggingface.co/thinkingmachines/Inkling-Small-NVFP4
- https://recipes.vllm.ai/thinkingmachines/Inkling-Small
- https://docs.sglang.io/cookbook/autoregressive/ThinkingMachines/Inkling-Small
- https://thinkingmachines.ai/news/introducing-inkling/

## Production boundary

This is candidate evidence, not rollout authority. Publishing a Finite image,
wrapping the checkpoint into an MPK, replacing candidate placeholders, adding
secrets or production traffic, or relaunching production requires its own
review and explicit authorization. The existing limiter/auth/accounting path
must remain the public authority.
