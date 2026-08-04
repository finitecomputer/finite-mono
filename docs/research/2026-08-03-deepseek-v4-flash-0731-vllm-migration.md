# DeepSeek-V4-Flash-0731 vLLM/Tinfoil migration research (2026-08-03)

Scope: primary-source verification for replacing the checked-in Tinfoil GLM 5.2
GPU model while retaining the vLLM/private-limiter topology. This is a
decision note, not authorization to build, measure, release, or relaunch an
enclave.

## Bottom line

The claim is real, but the names have two different meanings:

| Surface | Verified official identifier |
| --- | --- |
| DeepSeek hosted API model ID | `deepseek-v4-flash` |
| DeepSeek API model version (as of the July 31 update) | `DeepSeek-V4-Flash-0731` |
| Hugging Face self-hosted checkpoint | `deepseek-ai/DeepSeek-V4-Flash-0731` |

DeepSeek says the July 31 public-beta update keeps the same architecture and
size as V4-Flash Preview and is a re-post-training update; API callers still
select `deepseek-v4-flash`. [DeepSeek change log](https://api-docs.deepseek.com/updates/)
and [DeepSeek model/pricing contract](https://api-docs.deepseek.com/quick_start/pricing/)
are the authority for the hosted name/version mapping. The first-party HF
checkpoint is the relevant identifier for a self-hosted Tinfoil image. [HF
model card](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731)

This is not a drop-in replacement for the current GLM image. vLLM added base
DeepSeek-V4 support in v0.20.0, but the official vLLM recipe says the 0731
checkpoint's bundled DSpark speculative decoder requires vLLM 0.25.0 or later
(ROCm DSpark support requires 0.26.0). vLLM's 0.25.0 release also records the
new DSpark drafter/checkpoint support. [vLLM v0.25.0 release](https://github.com/vllm-project/vllm/releases/tag/v0.25.0).
The checked-in GLM 5.2 Tinfoil image is
the model-specific upstream v0.0.17 image, documented as patched vLLM 0.24.0;
there is no evidence that image can load this checkpoint unchanged. [vLLM
v0.20.0 release](https://github.com/vllm-project/vllm/releases/tag/v0.20.0),
[vLLM V4 recipe (updated 2026-07-31)](https://recipes.vllm.ai/deepseek-ai/DeepSeek-V4-Flash?features=tool_calling%2Creasoning&hardware=b300),
[candidate config](../../infra/tinfoil/confidential-kimi-k2-6/tinfoil-config.candidate.yml),
[upstream GLM Dockerfile at the staged commit](https://github.com/tinfoilsh/confidential-glm5-2/blob/84b2e805cd0ce59bc7170c3238294fa7762205e8/Dockerfile)

## Verified model facts

- **Architecture:** `DeepseekV4ForCausalLM`, `model_type: deepseek_v4`; a sparse
  MoE with 43 hidden layers, 4096 hidden size, 256 routed experts plus one
  shared expert, and six routed experts activated per token. The architecture
  uses the V4 hybrid attention/mHC design. [HF `config.json`](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731/blob/main/config.json),
  [HF model card](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731)
- **Parameters:** 284B total and 13B active, per DeepSeek's official release
  text. [DeepSeek change log](https://api-docs.deepseek.com/updates/)
- **Weights:** native mixed precision: MoE expert weights are FP4 and the
  remaining weights are FP8. The HF config records `expert_dtype: fp4` and
  dynamic E4M3 FP8 quantization with UE8M0 scales and 128x128 weight blocks.
  [HF `config.json`](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731/blob/main/config.json),
  [vLLM V4 recipe](https://recipes.vllm.ai/deepseek-ai/DeepSeek-V4-Flash?features=tool_calling%2Creasoning&hardware=b300)
- **Context:** `max_position_embeddings` is 1,048,576 (1M tokens). DeepSeek's
  hosted contract lists 1M context and a 384K maximum output. [HF
  `config.json`](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731/blob/main/config.json),
  [DeepSeek model/pricing contract](https://api-docs.deepseek.com/quick_start/pricing/)
- **Reasoning/encoding:** the checkpoint has no Jinja chat template; its
  `encoding/` helpers encode OpenAI-compatible messages. The 0731 card exposes
  `reasoning_effort` levels `low`, `high`, and `max`; DeepSeek recommends
  `temperature=1.0`, `top_p=0.95` for agentic workloads and `top_p=1.0`
  otherwise. [HF model card](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731)
- **Speculative module:** the checkpoint config contains DSpark fields
  (`dspark_block_size`, target layers 40/41/42, Markov rank 256). This is part
  of the 0731 checkpoint structure, not an assumption that the current GLM MTP
  settings can be reused. [HF `config.json`](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731/blob/main/config.json)
- **Hosted API surface:** DeepSeek lists Chat Completions, Anthropic, tools,
  and Responses API support for `deepseek-v4-flash`; Responses support is a
  hosted-service claim and does not prove local vLLM/limiter compatibility.
  [DeepSeek model/pricing contract](https://api-docs.deepseek.com/quick_start/pricing/)

## vLLM support and official launch shape

The first-party 0731 card gives this single-node example (4x GB300):

```text
vllm serve deepseek-ai/DeepSeek-V4-Flash-0731 \
  --trust-remote-code --kv-cache-dtype fp8 --block-size 256 \
  --data-parallel-size 4 --enable-expert-parallel \
  --moe-backend deep_gemm_mega_moe \
  --attention-config '{"use_fp4_indexer_cache": true}' \
  --speculative-config '{"method":"dspark","num_speculative_tokens":7,"draft_sample_method":"greedy"}'
```

[HF 0731 model card](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731)
is the source for this command. The vLLM recipe additionally shows the
DeepSeek-V4 tokenizer/tool/reasoning parser flags:
`--tokenizer-mode deepseek_v4`, `--tool-call-parser deepseek_v4`,
`--enable-auto-tool-choice`, and `--reasoning-parser deepseek_v4`. It states
that the fused 0731 checkpoint is about 167 GB on disk and requires vLLM
0.25.0 for DSpark (0.26.0 for ROCm DSpark). [vLLM V4 recipe](https://recipes.vllm.ai/deepseek-ai/DeepSeek-V4-Flash?features=tool_calling%2Creasoning&hardware=b300)

The recipe's 1M model context is not a requirement to expose 1M in the
service. It specifically calls for `--max-model-len >= 393216` (384K) for
Think Max. The current candidate already uses `--max-model-len 393216`, but
that is only a verified configuration fact, not proof that the new model will
fit or meet latency targets on the existing eight-GPU host.

## Existing checkout and migration implications

The candidate Tinfoil config has 8 GPUs/512 GiB, serves private vLLM on
`glm-5-2:8001`, sends public traffic through `finite-private-limiter:8002`,
and uses GLM-specific parsers, sparse-MLA attention, DCP, `runai_streamer`, and
MTP flags. [candidate config](../../infra/tinfoil/confidential-kimi-k2-6/tinfoil-config.candidate.yml)

For a V4-Flash-0731 trial, the authoritative changes would be a new measured
model image and HF revision, a new served model name, and V4-specific vLLM
runtime/parser flags. The following are **not verified and must not be carried
forward by assumption**:

1. The current `confidential-glm5-2` image loading V4 weights.
2. `runai_streamer` loading the FP4+FP8/DSpark checkpoint.
3. GLM parser names (`glm47`, `glm45`), GLM sparse-MLA/DCP flags, or GLM MTP
   being valid for V4.
4. vLLM's local `/v1/responses` behavior matching DeepSeek's hosted Responses
   API or the existing limiter's accounting/streaming contract.
5. Eight-GPU capacity, `--max-num-seqs 32`, or 393216-token operation being
   safe without a measured V4 load/startup test.

The safe planning boundary is therefore: use a separate digest-pinned vLLM
>=0.25.0 image with an immutable HF revision, preserve the private-vLLM →
limiter → shim and sealed-secret topology, then prove model load, health,
chat/streaming, tool calls, Responses, reasoning modes, accounting settlement,
startup duration, and bounded concurrency before any production decision. A
Finite-specific derived image is unnecessary unless the official image fails
a demonstrated requirement; Tinfoil's existing DeepSeek-V4-Pro enclave is
evidence for using the official digest-pinned vLLM image directly. [Tinfoil
DeepSeek-V4-Pro config](https://github.com/tinfoilsh/confidential-deepseek-v4-pro/blob/main/tinfoil-config.yml)

## FP8 and DSpark evidence (2026-08-03)

There is no published first-party `DeepSeek-V4-Flash-0731-FP8` instruct
checkpoint. The official 0731 config stores MoE experts in FP4 and dynamically
quantizes the other weights to E4M3 FP8. DeepSeek publishes a full-FP8 **base**
checkpoint, but not a full-FP8 version of the 0731 re-post-trained instruct
model. The 0731 reference converter can expand the released FP4 expert values
and cast them to FP8; that produces a custom converted artifact and cannot
restore precision absent from the published FP4 values. It is not an official
vLLM-validated 0731 FP8 checkpoint. [0731 model card](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731),
[0731 config](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731/raw/main/config.json),
[reference inference instructions](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731/blob/main/inference/README.md),
[reference converter](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731/raw/main/inference/convert.py),
[official FP8 base checkpoint](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-Base)

DSpark has strong but bounded performance evidence. DeepSeek's paper evaluates
V4-Flash **Preview**, not the 0731 checkpoint, and reports 60--85% faster
per-user generation at matched aggregate throughput versus its MTP-1 production
baseline. The paper reports no 0731-on-H200 acceptance or speed result. Its
quality contract is stronger: target-model rejection sampling preserves the
target distribution exactly, and DSpark's causal early-stop scheduling is
designed to retain that property. Approximate `synthetic` rejection sampling
must not be enabled when this losslessness matters. [DSpark paper](https://arxiv.org/abs/2607.05147),
[DeepSpec repository](https://github.com/deepseek-ai/DeepSpec),
[vLLM 0.25 speculative configuration](https://github.com/vllm-project/vllm/blob/v0.25.0/vllm/config/speculative.py)

The 0731 checkpoint includes the DSpark module, and its official vLLM command
enables it with one engine argument and no separate draft model:
`--speculative-config '{"method":"dspark","num_speculative_tokens":7,"draft_sample_method":"greedy"}'`.
That makes it operationally simple and removable, but the published speedup is
supporting evidence rather than proof for Finite's exact Tinfoil/H200 serving
shape. A conservative release should keep the KV cache explicitly FP8, prepare
measured DSpark-on and DSpark-off configurations, and make the production
choice from short H200 protocol, output-equivalence, latency, and concurrency
canaries. [0731 model card](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731),
[vLLM V4 recipe](https://recipes.vllm.ai/deepseek-ai/DeepSeek-V4-Flash?features=tool_calling%2Creasoning&hardware=b300),
[vLLM 0.25 release](https://github.com/vllm-project/vllm/releases/tag/v0.25.0)

## Runtime version decision (2026-08-03)

Upstream's latest stable release is vLLM 0.26.0. The official DeepSeek V4
recipe sets 0.25.0 as the minimum for the 0731 checkpoint with DSpark and marks
H200 as verified; 0.26.0 adds further DeepSeek V4 routing and end-to-end
performance work. The planned H200 candidate therefore uses a digest-pinned
vLLM 0.26.0 image, with an exact `vllm --version` check in CI and the running
container's startup evidence. A newer release is not adopted automatically
during the maintenance change, and a 0.25.0 fallback requires an explicit
decision rather than a silent downgrade. [vLLM 0.26.0
release](https://github.com/vllm-project/vllm/releases/tag/v0.26.0), [official
DeepSeek V4 recipe](https://github.com/vllm-project/recipes/blob/main/models/deepseek-ai/DeepSeek-V4-Flash.yaml)

The recipe's `deep_gemm_mega_moe` and FP4 indexer-cache flags are Blackwell
hardware overrides. They are not part of its H200/Hopper recipe and must not
be copied from the GB300 example into the planned eight-H200 configuration
without separate evidence. This distinction changes the planned launch flags,
not the decision to keep the official mixed checkpoint or FP8 KV cache.

## Pre-window preparation snapshot (2026-08-03)

Read-only Tinfoil inventory immediately before artifact preparation reported
the existing `kimi-k2-6` enclave `ready` on
`v2026-07-02-glm-5-2-limiter-routing-1`, with 8 H200 GPUs, 32 CPUs, 512 GiB
memory, TDX confidential compute, and the expected three secret names. The
satellite's `origin/main` was `6654fe22259b0cbc508821be77c35ca13199863d`,
which contains the live tagged config unchanged.

Tinfoil modelwrap job `buzbcevsfmbhmruz` completed on the default
`control.inf9.tinfoil.sh` build host for the exact public checkpoint and
revision used everywhere in this plan. It returned root hash
`9dd15749a2f9c554cefb41b9bb202c2994d64519b4efbd42af68b51e010d5e21`,
MPK offset `166898688000`, and the complete MPK identifier now pinned by both
candidate configs. The wrap did not create a satellite release or change the
running enclave.

A read-only one-hour Tinfoil metrics snapshot immediately after the wrap
showed the live GLM enclave at 94% GPU-memory utilization throughout, with
intermittent production compute bursts and a 100% observed GPU-utilization
peak. The final several pre-snapshot intervals averaged roughly 87–88% GPU
utilization before returning idle. Tonight's controlled GLM and DeepSeek
comparison must therefore preserve the same bounded workload and account for
ambient traffic; these host samples alone are not a throughput benchmark.

Both DeepSeek candidates were subsequently measured by the satellite release
workflow. DSpark-off tag
`v2026-08-03-deepseek-v4-flash-0731-dspark-off-1` has Tinfoil config hash
`3351fea60f7d1276d3e6c8b3192ab38ae4d6cee07db6995fcd504b922ccafa5e`;
DSpark-on tag `v2026-08-03-deepseek-v4-flash-0731-dspark-on-1` has config hash
`6fdcc5841d394bf2979df6c0b0a0c2c39b792a2b2f2a1bd2df59b60d65d750c8`.
Each release contains `tinfoil-deployment.json` and `tinfoil.hash`; decoding
the embedded config reproduced its checked candidate exactly.

The controlled GLM baseline used the same 64-token streaming workload planned
for the cutover. Concurrency 1 delivered 55.521 aggregate generation tok/s
with 0.154s TTFB and 1.133s completion. Concurrency 32 delivered 320.787
aggregate tok/s, 2.121s p50 / 5.161s p99 TTFB, and 3.923s p50 / 6.326s p95
completion. Post-load `/health` remained deep-ready, and a read-only Core query
proved all 35 preparation requests made with the temporary key were settled,
with zero non-settled reservations.
