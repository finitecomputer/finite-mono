# Eight-H200 serving campaign handoff

Date: 2026-08-07

Status: evidence backed up; no production mutation authorized by this document.

The isolated campaign used the temporary eight-H200 host
`control.inf12.tinfoil.sh`. It carried no production secrets or traffic.
Production remained on `control.inf9.tinfoil.sh`. The temporary host
authorization has expired.

## Durable artifact map

| Model | Durable branch | Evidence commit | Outcome |
| --- | --- | --- | --- |
| Laguna S 2.1 FP8 | `ops/laguna-s21-h200-20260807` | `56c52803c6b9416f6bd8b9b3efe3eb046c2198e6` | Valid four-TP2 recipe; deploy preparation still required |
| DeepSeek V4 Flash 0731 | `ops/deepseek-v4-h200-20260807` | `839d0f36e1a7c376be2671770af60c413f39520d` | Valid scheduler winner over the current production baseline |
| Inkling Small NVFP4 | `ops/inkling-small-h200-20260807` | `f34a65fe28c4c1d1d29a67c18eb79778303dc0c3` | vLLM rejected for corrupt output; SGLang candidate not locally validated before expiry |

The model-specific evidence remains authoritative:

- [Laguna S 2.1](2026-08-07-laguna-s21-eight-h200-serving.md)
- [DeepSeek V4 optimization](2026-08-07-deepseek-v4-eight-h200-optimization.md)
- [Inkling Small](2026-08-07-inkling-small-eight-h200-serving.md)

## Results that may be carried forward

### Laguna S 2.1

The valid topology is four independent TP2 replicas behind the checked-in
least-active router. It uses FP8 weights, BF16 KV, DFlash-15, eager execution,
Triton MoE, and the native 1,048,576-token service ceiling. The final public
router test completed 512/512 requests at 6,249 aggregate output tok/s with no
backend errors. A 1,000,042-token prompt also completed correctly.

This is serving evidence, not a release candidate: the two model MPKs, pinned
image, and reproducible tokenizer correction still need to be built and
measured.

### DeepSeek V4 Flash 0731

The winner changes exactly two scheduler settings from the production
baseline:

| Setting | Current production baseline | Lab winner |
| --- | ---: | ---: |
| `--max-num-seqs` | 64 | **128** |
| `--max-num-batched-tokens` | 512 | **2048** |

Everything else remains fixed: checkpoint, MPK, immutable runtime image,
FP4/FP8 weight formats, FP8 KV, DP8+EP topology, parsers, sampling behavior,
393,216-token ceiling, and no speculation.

At 1,024 simultaneous 128-token requests, the winner delivered 8,373 output
tok/s with zero errors versus 5,635 tok/s for the baseline. Its final
single-session result was 54.78 output tok/s with 0.214-second TTFT. Protocol,
380K-context, million-output-token soak, post-soak health, and log-error gates
passed. The scheduler study did not replace the production 35-minute stability
and real-route accounting gates.

Use the dedicated
[scheduler-promotion runbook](../../infra/runbooks/finite-private-deepseek-v4-flash-0731-scheduler-promotion.md)
for tonight's possible production update.

### Inkling Small

The checkpoint was pinned and all 21 tracked files/checksums verified. The
mixed runtime, clean vLLM 0.26.0, and current vLLM nightly all failed raw-token
correctness on H200, collapsing to token ID 1023. Their throughput is invalid.

The official SGLang TP8 H200 recipe and immutable CUDA 13 image manifest are
preserved, but the temporary host authorization expired before Finite could run
its correctness gate. Do not deploy Inkling until that gate passes on a fresh
isolated allocation.

## Immutable lab release evidence

The disposable satellite releases are retained in
`finitecomputer/confidential-kimi-k2-6`:

- `v2026-08-07-gpu-lab-debug-1`: no-model temporary lab base;
- `v2026-08-07-gpu-lab-inkling-1`: old dedicated Inkling image, checkpoint
  quantization not recognized;
- `v2026-08-07-gpu-lab-inkling-2`: clean vLLM 0.26.0, corrupt output;
- `v2026-08-07-gpu-lab-inkling-3`: current nightly, corrupt output; and
- `v2026-08-07-gpu-lab-inkling-4`: measured SGLang image, live model test not
  completed before host expiry.

These debug releases contain no production secrets and are investigation
evidence, not production release authority.

## Current production snapshot and tonight's boundary

At `2026-08-08T00:08Z`, Tinfoil reported `kimi-k2-6` ready on eight H200s at
`control.inf9.tinfoil.sh`, tag
`v2026-08-05-deepseek-v4-flash-0731-retry-2-3`. That tag's config is the
64/512 DeepSeek baseline. The snapshot proves control-plane state only; the
next maintenance window must repeat deep health, authenticated canary, Core
settlement, and status gates.

The optimized candidate config has SHA-256
`bc5a7606811e164a04c286b6bc6d59f46dda57adfd20bd46ac06fe340bf8a204`.
The currently running tag's source config has SHA-256
`89b31ee0ca2d0a580a4d4b8c4a52c3f88e2c76d4e213ede8c2a74aec385bbb9d`.
Their executable difference is only the two scheduler values above.

Creating a new measured satellite release is a separate preparation action.
Relaunching production requires fresh explicit approval for the exact new tag.
The immediate rollback for this scheduler-only update is the currently running
DeepSeek tag, not a new image or an inferred historical configuration.
