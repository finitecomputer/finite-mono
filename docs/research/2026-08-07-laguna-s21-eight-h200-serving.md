# Laguna S 2.1 FP8 on eight H200s

Date: 2026-08-07

Scope: isolated testing on temporary Tinfoil host
`control.inf12.tinfoil.sh`. Production host `control.inf9.tinfoil.sh` was not
changed. The temporary container was debug-enabled, held no production
secrets, and used public immutable model revisions.

## Result

The best verified shape is four independent TP2 replicas, one per H200 pair,
behind a least-active streaming router:

```text
public :8000 router
  -> GPUs 0,1 :8001 (TP2)
  -> GPUs 2,3 :8002 (TP2)
  -> GPUs 4,5 :8003 (TP2)
  -> GPUs 6,7 :8004 (TP2)
```

Each replica uses FP8 checkpoint weights, BF16 KV cache, DFlash-15, eager
execution, Triton MoE, a 1,048,576-token service ceiling, 128 scheduler
sequences, and 16,384 batched tokens. This combines Poolside/vLLM's H200 TP2
and DFlash guidance with the configuration that was stable in this Tinfoil
runtime. The launcher raises `RLIMIT_NOFILE` to 65,536: the inherited 1,024-FD
limit produced seven router connection failures in an otherwise successful
512-request test because each active request needs an inbound and upstream
socket.

Pinned artifacts:

- target: `poolside/Laguna-S-2.1-FP8` at
  `9e0b8ba630080b0e6f20a7b43294a9f2232fd247`;
- draft: `poolside/Laguna-S-2.1-DFlash-FP8` at
  `a16e2e9287093bf74d7ecd5b5bea732687e0268e`;
- runtime: Finite's vLLM 0.25.1 image digest
  `sha256:48716fa9c25605ab5fe00fd7eed4e792268aee6c9008616f7641d9bf622ff262`.

The target revision is the current FP8 repository head as of this test. Its
`config.json` declares native `max_position_embeddings=1048576`; the 1M
weights were trained through long-context extension and quantized at that
configuration. Poolside notes that quality can still decline at very long
context. The model's `generation_config.json` remains authoritative, including
the eval-certified `top_k=20` default.

The upstream tokenizer emitted Transformers' known Mistral-regex warning until
top-level `fix_mistral_regex: true` was added to `tokenizer_config.json`. The
lab retained the original as `tokenizer_config.pre-fix-mistral-regex.json`.
Production model wrapping must make this one-file correction reproducibly and
record the derived artifact hash; it must not mutate a read-only MPK at boot.
The observed SHA-256 changed from
`65606e1b77dd95b5a157860aa7141941a0181c9e4eb2b1384c8795f997a605ac`
to `fc9cfeb005182b742e00e2d2a4e3128b5d6ee26d59cae86c9c71b73a60fdf8ed`.

## Verified command shape

The executable launcher and router are:

- `infra/tinfoil/confidential-kimi-k2-6/laguna-s21-launch.sh`
- `infra/tinfoil/confidential-kimi-k2-6/laguna-s21-router.py`

Important flags per replica:

```text
--tensor-parallel-size 2
--max-model-len 1048576
--kv-cache-dtype bfloat16
--max-num-seqs 128
--max-num-batched-tokens 16384
--enable-prefix-caching
--enable-chunked-prefill
--disable-custom-all-reduce
--enforce-eager
--moe-backend triton
--speculative-config {model: DFlash-FP8, num_speculative_tokens: 15, method: dflash}
```

BF16 KV is intentional. Explicit FP8 KV caused vLLM 0.25.1 to warn that the
checkpoint did not provide a query scaling factor and that scale 1.0 could
hurt accuracy. The service therefore uses native FP8 weights without taking
an uncalibrated KV-quality risk. At the winning setting, each replica reported
about 1.51 million KV tokens, enough for one near-1M request but not two.
Admission control must account for prompt tokens, not only request count.

## Measurements

All throughput is aggregate output tokens/second. Short-request measurements
used 128 output tokens and non-thinking chat so they measure serving capacity,
not reasoning-token quality.

| Shape/workload | Concurrency | Output tok/s | p95 TTFT | p95 latency | Errors |
| --- | ---: | ---: | ---: | ---: | ---: |
| TP8 eager, no DFlash | 64 | 552 | 1.13s | 14.80s | 0 |
| One TP2, no DFlash | 64 | 495 | 2.18s | 16.52s | 0 |
| One TP2, DFlash-15, seq=64 | 64 | 1,306 | 1.40s | 6.25s | 0 |
| Four TP2, DFlash-15, seq=128, cache-cold first wave | 256 | 2,298 | 3.71s | 13.31s | 0 |
| Four TP2, DFlash-15, seq=128, warm unique-first-token | 512 | 6,711 | 0.86s | 9.33s | 0 |
| Four TP2, DFlash-15, warm repeated-structure ceiling | 512 | 8,273 | 0.52s | 6.85s | 0 |
| Final public-port router, four TP2, unique early marker | 512 | 6,249 | not streamed | 10.49s wall | 0 |

The 8,273 tok/s result is a warm lab ceiling because prompts retained reusable
structure. Use 2,298 tok/s as the cold-wave floor and 6,711 tok/s as the
steady saturated planning observation until a production prompt-trace replay
is available. The final routed test generated exactly 65,536 output tokens,
distributed exactly 128 requests to each replica, and returned 512/512
successes with zero router/backend failures. Warm every replica before adding
it to routing.

DFlash generated 61,860 draft tokens in the measured sample and accepted
9,374 (15.15% raw token acceptance, 2.27 accepted tokens per draft step). Even
with that modest raw acceptance, its parallel verification materially reduced
latency and increased throughput. `min_p` and `logit_bias` are incompatible
with vLLM speculative decoding in this version; Poolside's default `min_p` is
zero, so the checkpoint recipe is unaffected.

Long-context gates:

- 250,048-token chat prompt: correct output, 14.78s TTFT, no error;
- 1,000,042-token chat prompt: correct output, 155.50s TTFT, no error;
- all four final replicas advertised `max_model_len=1048576`;
- the router distributed a 64-request probe 17/16/16/16 with no backend
  failure.

Protocol smokes also passed normal chat, separated reasoning content, and a
structured `get_weather` tool call. Reasoning is off by checkpoint default;
clients requiring it must send
`chat_template_kwargs={"enable_thinking":true}`. The throughput table is not a
quality benchmark and must not be presented as one.

## Rejected configurations

- TP8 with compiled execution and CUDA graphs: illegal-access/CUBLAS failures.
- TP8 with custom all-reduce disabled but graphs retained: PYNCCL/graph memory
  failures.
- TP2 compiled with CUDA graphs explicitly disabled: BF16 CUBLAS failure during
  memory profiling. Eager mode is required for this runtime/driver combination.
- Explicit FP8 KV: uncalibrated scaling warning, rejected on quality grounds.
- One TP8 replica: stable only in eager mode and far less GPU-efficient than
  four TP2 replicas.

## Production boundary

This is deployable serving evidence, not an authorized production rollout.
Before release, build a dedicated immutable Laguna image containing the
launcher/router, wrap both pinned checkpoints (including the recorded tokenizer
correction), pin their MPKs and image digest in a target-only candidate, retain
the existing limiter/auth/accounting topology, and run the standard status,
quality, parser, long-context, overload, and rollback gates. Do not expose the
vLLM replicas directly or attach production secrets to a debug container.

Primary sources:

- https://huggingface.co/poolside/Laguna-S-2.1-FP8
- https://huggingface.co/poolside/Laguna-S-2.1-DFlash-FP8
- https://recipes.vllm.ai/poolside/Laguna-S-2.1
