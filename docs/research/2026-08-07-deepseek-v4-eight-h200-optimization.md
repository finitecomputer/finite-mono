# DeepSeek V4 Flash 0731: isolated eight-H200 optimization

Date: 2026-08-07

Status: measured candidate only. Production on `control.inf9.tinfoil.sh` was
not changed. All tests ran in the disposable `gpu-lab` container on
`control.inf12.tinfoil.sh` without production secrets or traffic.

## Outcome

The winning target-only recipe changes only two scheduler limits from the
retry-2 baseline:

| Setting | Baseline | Winner |
| --- | ---: | ---: |
| `--max-num-seqs` | 64 | **128** |
| `--max-num-batched-tokens` | 512 | **2048** |

The model revision, model MPK, image, weight formats, FP8 KV cache, DP8+EP
topology, parsers, context ceiling, graph mode, and sampling requests were
unchanged. This is therefore a throughput/admission change, not a model-quality
or numerical-format change.

At 1,024 simultaneous 128-token reasoning requests, the winner produced
8,373 output tokens/second with zero errors, versus 5,635 tokens/second for the
baseline. P95 time to first token fell from 14.16 seconds to 4.05 seconds.

## Fixed identity

- Host allocation: eight NVIDIA H200 GPUs.
- Model: `deepseek-ai/DeepSeek-V4-Flash-0731`.
- Revision: `7872f01b1d1fe23eabc4c98b48bffcef5a386062`.
- Local model verification: all 74 remote files and all checksums passed.
- Modelwrap MPK root:
  `9dd15749a2f9c554cefb41b9bb202c2994d64519b4efbd42af68b51e010d5e21`.
- Runtime image:
  `ghcr.io/finitecomputer/deepseek-v4-vllm:0.25.1-0731-reasoning.6@sha256:48716fa9c25605ab5fe00fd7eed4e792268aee6c9008616f7641d9bf622ff262`.
- Runtime: vLLM 0.25.1 with the guarded upstream 0731 reasoning mapping.
- Native model formats: FP4 experts and FP8 other weights/linear paths.
- KV cache: FP8 using DeepSeek's `fp8_ds_mla` layout.
- Parallel shape: data parallel 8 plus expert parallel; TP1 on each rank.
- MoE communication: PYNCCL with `AgRsAll2AllManager`.
- MoE kernel observed: MARLIN MXFP4.
- Attention/linear path observed: DeepGEMM FP8 with UE8M0 enabled.
- Maximum service context: 393,216 tokens.
- Decode graphs: `FULL_DECODE_ONLY`; torch compilation disabled by breakable
  CUDA graphs as reported by vLLM.
- Speculation: disabled. DSpark remained excluded.

The complete winning command is preserved in
[`deepseek-v4-lab-launch.sh`](../../infra/tinfoil/confidential-kimi-k2-6/deepseek-v4-lab-launch.sh),
and the Tinfoil-shaped candidate is
[`tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml`](../../infra/tinfoil/confidential-kimi-k2-6/tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml).

## Method

The streaming harness used unique SHA-256-prefixed prompts so prefix-cache hits
could not inflate the synthetic comparison. Every scored request enabled
thinking, selected `reasoning_effort=high`, ignored EOS, and generated an exact
128 tokens. It recorded server-reported completion tokens, aggregate output
rate, TTFT percentiles, end-to-end latency, and every transport or HTTP error.

The benchmark process raised its `RLIMIT_NOFILE` before high-concurrency runs.
An initial 1,024-client baseline attempt produced six client-side `EMFILE`
errors at the default soft limit of 1,024; this was a harness defect, not a
server failure, and is excluded. The corrected rerun completed 1,024/1,024.

Each configuration was launched from clean GPU memory. Exact process arguments
and vLLM's parsed non-default configuration were checked before measurement.
Only one serving variable was changed at a time:

1. exact retry-2 baseline: batch tokens 512, sequences 64;
2. batch-token candidate: batch tokens 2,048, sequences 64;
3. winning admission candidate: batch tokens 2,048, sequences 128.

Chat/reasoning/tool protocol gates ran before performance scoring. Large
concurrency shapes were exercised before their steady-state measurement so
lazy execution-path creation could not be scored as normal throughput.

## Results

All table rows used the same checkpoint, image, reasoning request, unique short
prompt family, 128 output tokens per request, and zero-error runs.

| Config | Clients | Output tok/s | P95 TTFT | P95 latency | Success |
| --- | ---: | ---: | ---: | ---: | ---: |
| baseline 512/64 | 256 | 4,512.46 | 2.309 s | 7.218 s | 256/256 |
| batch 2048, seqs 64 | 256 | 5,101.53 | 1.442 s | 6.385 s | 256/256 |
| **winner 2048/128** | 256 | **5,033.92** | **1.244 s** | **6.469 s** | **256/256** |
| baseline 512/64 | 512 | 5,054.02 | 4.400 s | 10.739 s | 512/512 |
| batch 2048, seqs 64 | 512 | 5,737.53 | 2.346 s | 8.791 s | 512/512 |
| **winner 2048/128** | 512 | **7,511.17** | **2.385 s** | **8.627 s** | **512/512** |
| baseline 512/64 | 1,024 | 5,634.71 | 14.160 s | 20.910 s | 1,024/1,024 |
| batch 2048, seqs 64 | 1,024 | 6,555.42 | 10.710 s | 17.438 s | 1,024/1,024 |
| **winner 2048/128** | 1,024 | **8,373.44** | **4.045 s** | **13.457 s** | **1,024/1,024** |

Relative to baseline, the winner improved output throughput by 11.6% at 256
clients and 48.6% at both 512 and 1,024 clients. At 1,024 clients it reduced
p95 TTFT by 71.4%.

The 128-sequence candidate repeated consistently:

- first 512-client pass: 7,512.95 output tok/s;
- second 512-client pass: 7,511.17 output tok/s;
- first 1,024-client pass: 8,623.97 output tok/s;
- second 1,024-client pass: 8,373.44 output tok/s.

## Promotion gates completed in the lab

### Protocol

The pre-load and post-soak gates both passed:

- non-thinking arithmetic returned 42 without a raw reasoning marker;
- thinking returned non-empty structured `message.reasoning`;
- final content did not contain `<think>`;
- automatic tool choice emitted a structured `get_weather` call; and
- no protocol-gate failure was recorded.

The exact gate is
[`deepseek-v4-protocol-gate.py`](../../infra/tinfoil/confidential-kimi-k2-6/deepseek-v4-protocol-gate.py).

### Near-limit context

A request with 380,009 prompt tokens, 96.6% of the 393,216-token service
ceiling, returned `OK` successfully in 37.209 seconds. This proves the winner
retains the advertised per-request context ceiling with chunked prefill.

The exact gate is
[`deepseek-v4-context-gate.py`](../../infra/tinfoil/confidential-kimi-k2-6/deepseek-v4-context-gate.py).

### Sustained output soak

The final soak ran 1,024 simultaneous high-reasoning requests with exactly
1,024 output tokens each:

- 1,024/1,024 successful;
- zero errors;
- 1,048,576 output tokens;
- 97.483 seconds wall time;
- 10,756.55 aggregate output tok/s;
- 3.944 seconds p95 TTFT; and
- 77.681 seconds p95 request latency.

After the soak the health endpoint and all protocol gates still passed, and the
server log contained zero `ERROR`, `Traceback`, CUDA-error, or out-of-memory
matches.

This 97-second load soak is not a substitute for the existing 35-minute
production stability gate in the retry-2 runbook. It is strong evidence for
the scheduler comparison only.

## Memory and capacity tradeoff

The larger batch-token ceiling reserved more non-KV memory:

| Config | Available KV memory/rank | KV tokens/rank | Full 393,216-token contexts/rank |
| --- | ---: | ---: | ---: |
| baseline 512/64 | 105.19 GiB | 24,536,588 | 62.4 |
| winner 2048/128 | 103.97 GiB | 18,405,902 | 46.8 |

The winner retains the 393,216-token limit for an individual request and can
admit up to 128 short/ordinary sequences per rank, or 1,024 cluster-wide. It
cannot hold 128 maximum-length sequences on every rank simultaneously; KV
pressure would require preemption/recomputation. For a workload dominated by
hundreds of concurrent 393K-token sessions, keep a separate long-context lane
or prefer the baseline's larger KV reserve. For ordinary mixed chat traffic,
the measured latency and throughput win is decisive.

Graph capture used 0.37 GiB for the winner versus 0.25 GiB at 64 sequences.
Observed steady GPU memory was approximately 139.4--140.0 GiB of 143.8 GiB,
without OOM during the soak.

## Readiness and warm-up

The 64-sequence experiments exposed a large lazy-path penalty after nominal
readiness: an unseen 64-client shape reached roughly 35 seconds TTFT, and later
unseen shapes produced p95 spikes above 40 seconds. Repeating the same shapes
removed the penalty. With 128 sequences, startup captured decode graphs through
size 256 and the first measured shapes did not reproduce that severe spike,
but production must still warm all DP ranks after `/health` becomes ready.

Use the checked-in benchmark harness with a high file-descriptor limit and run
at least 64, 256, 512, and 1,024 global request shapes before admitting normal
traffic. Treat readiness plus warm-up completion—not `/health` alone—as chat
readiness.

## What was rejected or deferred

- Batch tokens 512 remained correct but left 13--16% throughput on the table
  even before increasing live sequences.
- Batch tokens 2,048 with only 64 sequences improved the baseline but retained
  a large queue at 1,024 clients.
- DSpark was not retried. The prior attempt produced corrupt output and about
  1% acceptance; speculation cannot be called quality-neutral.
- Alternative all-to-all backends were deferred. The scheduler-only winner was
  large, quality-neutral, and used the official/default H200 path; changing MoE
  communication would add startup and correctness risk without first exhausting
  the safer admission controls.
- Prefill/decode disaggregation was deferred. It changes topology and routing
  and needs a dedicated workload study rather than a short maintenance-window
  A/B.

## Production boundary

No production container, release, route, secret, limiter, product default, or
persisted chat state was changed. Promotion still requires explicit operator
authorization and every gate in
[`finite-private-deepseek-v4-flash-0731-retry-2.md`](../../infra/runbooks/finite-private-deepseek-v4-flash-0731-retry-2.md),
including the 35-minute stability interval, real Finite route/settlement proof,
hosted-reference quality comparison, mixed protocol canaries, measured release
identity, and exact GLM rollback authority.
