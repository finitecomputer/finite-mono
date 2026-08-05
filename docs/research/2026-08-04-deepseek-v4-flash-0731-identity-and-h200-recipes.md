# DeepSeek-V4-Flash-0731 identity and eight-H200 serving evidence

Date: 2026-08-04

Scope: read-only review of the preserved Finite candidates against primary
DeepSeek, Hugging Face, vLLM, NVIDIA, and Tinfoil sources. This note does not
authorize or perform a production change.

## Bottom line

Finite wrapped and served the **correct latest official 0731 checkpoint**:
`deepseek-ai/DeepSeek-V4-Flash-0731` at revision
`7872f01b1d1fe23eabc4c98b48bffcef5a386062`. That revision is still the Hugging
Face repository head. Its only change from the original release commit was a
model-card addition; the model weights, config, tokenizer, and encoding were
unchanged. [Official HF revision](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731/commit/7872f01b1d1fe23eabc4c98b48bffcef5a386062)

The concern about output quality is nevertheless justified. Finite did **not**
serve the checkpoint with fully correct 0731 reasoning-effort behavior:

- The pinned vLLM 0.26.0 frontend defaults requests that omit all reasoning
  controls to **non-thinking chat mode**. Finite's Hermes configuration did
  request top-level `reasoning_effort=high`; vLLM converted that field into
  thinking mode, so the normal Hermes path was not simply non-thinking.
- `--reasoning-parser deepseek_v4` only parses reasoning that the model emits;
  it does not enable thinking.
- vLLM 0.26.0 predates the complete 0731 `low` / `high` / `max` prompt mapping.
  In that release, Finite's requested `high` enabled thinking but received no
  0731 high-effort prefix—effectively the official low prompt—and `max`
  received the text that 0731 defines as `high`. The corrected mappings landed
  on vLLM main on 2026-08-04, after v0.26.0 and after the Finite deployment.
- DeepSeek's published 0731 agent benchmark results used **max reasoning** with
  `temperature=1.0` and `top_p=0.95`. Finite/Hermes requested high, not max,
  and the v0.26 mapping reduced that request to the low prompt, so its outputs
  are not a fair reproduction of those results.

[vLLM 0.26 OpenAI request mapping](https://github.com/vllm-project/vllm/blob/v0.26.0/vllm/entrypoints/openai/chat_completion/protocol.py),
[vLLM 0.26 tokenizer behavior](https://github.com/vllm-project/vllm/blob/v0.26.0/vllm/tokenizers/deepseek_v4.py),
[vLLM 0.26 effort encoder](https://github.com/vllm-project/vllm/blob/v0.26.0/vllm/tokenizers/deepseek_v4_encoding.py),
[0731 reasoning fix on vLLM main](https://github.com/vllm-project/vllm/commit/77434861904a9f01ea4818fe9f0c7b2a5c05686e),
[official 0731 model card](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731)

This was not a case of applying an overly aggressive all-FP8 weight
quantization. The official 0731 checkpoint is natively mixed precision: routed
MoE experts are FP4 and the remaining quantized weights use dynamic E4M3 FP8
with UE8M0 scales. Finite used that official checkpoint unchanged and separately
selected an FP8 KV cache. [Revision-pinned config](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731/raw/7872f01b1d1fe23eabc4c98b48bffcef5a386062/config.json)

## Exact checkpoint identity

| Item | Official source | Finite candidate | Result |
| --- | --- | --- | --- |
| Repository | `deepseek-ai/DeepSeek-V4-Flash-0731` | Same | Match |
| HF revision | `7872f01b1d1fe23eabc4c98b48bffcef5a386062` | Same | Match; still current head |
| Architecture | `DeepseekV4ForCausalLM`, `model_type=deepseek_v4` | Loaded through vLLM's DeepSeek V4 implementation | Match |
| Weight files | 48 safetensor shards; repository storage about 166.89 GB decimal | Tinfoil MPK root `9dd15749...`, wrapped size/offset `166898688000` | The MPK was produced from the exact pinned HF revision |
| Native weight format | FP4 routed experts plus dynamic E4M3 FP8 elsewhere | No custom conversion; official checkpoint mounted directly | Match |
| KV cache | Serving choice, not a checkpoint identity property | Explicit `--kv-cache-dtype fp8` | Intended and supported |
| DSpark module | Bundled in 0731 config | Present in the wrapped model; enabled only in the rejected DSpark candidate | Match |

The HF Git revision, the individual large-file object IDs, and the Tinfoil MPK
root are different identifiers for different layers of the artifact. They are
not expected to be numerically equal. The important chain is that Tinfoil
modelwrap resolved the immutable HF revision and the served path used the root
from the resulting MPK.

The first-attempt target-only candidate, preserved on branch
`ops/deepseek-v4-flash-0731-cutover`, pinned:

```text
deepseek-ai/DeepSeek-V4-Flash-0731@7872f01b1d1fe23eabc4c98b48bffcef5a386062
mpk root 9dd15749a2f9c554cefb41b9bb202c2994d64519b4efbd42af68b51e010d5e21
vllm/vllm-openai:v0.26.0@sha256:770fe65b2c73ee74a5c42165cf3433de4048cc2cd9c57a937ca4e35aba5aa87b
```

Retry 2 keeps the checkpoint and MPK but replaces that runtime/topology, as
recorded in
[`finite-private-deepseek-v4-flash-0731-retry-2.md`](../../infra/runbooks/finite-private-deepseek-v4-flash-0731-retry-2.md).

## Why the output could look worse than the latest model

### 1. Thinking defaults and Hermes' requested effort differed

The 0731 checkpoint has no Jinja chat template. It ships a dedicated encoder
which chooses between chat/non-thinking and thinking modes and applies the
effort prefix. vLLM activates this encoder with `--tokenizer-mode deepseek_v4`.
[Official encoding guide](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731/raw/7872f01b1d1fe23eabc4c98b48bffcef5a386062/encoding/README.md)

In vLLM 0.26.0, the tokenizer wrapper defaults a request with no reasoning
controls to chat mode:

```python
thinking = kwargs.get("thinking", False)
enable_thinking = kwargs.get("enable_thinking", False)
thinking_mode = "thinking" if thinking or enable_thinking else "chat"
```

Therefore a plain OpenAI request with neither `chat_template_kwargs` nor a
top-level `reasoning_effort` is rendered as non-thinking. The checked Finite
smoke/load canaries fell into that category. Hermes was different: its local
configuration requested `reasoning_effort: high`, its custom-provider adapter
sent that as a top-level OpenAI field, and vLLM 0.26's request protocol added
`enable_thinking=true` before calling this tokenizer. Hermes therefore did
enter thinking mode; its quality mismatch came from the incomplete effort
mapping below, not from thinking being entirely disabled.

DeepSeek's hosted API now documents thinking as enabled by default, normally at
effort `high`; vLLM 0.26.0 did not reproduce that hosted default.
[DeepSeek thinking-mode contract](https://api-docs.deepseek.com/guides/thinking_mode)

### 2. vLLM 0.26.0 did not contain the final 0731 effort prompts

The checkpoint's own encoder defines:

- `low`: no prefix;
- `high`: the `Absolute maximum...` prefix;
- `max`: the new `Beyond maximum...` prefix.

vLLM 0.26.0 had only the older `Absolute maximum...` prefix and inserted it
only for its internal `max` mapping. The upstream correction added all three
0731 mappings and changed omitted thinking/effort to thinking enabled at high.
It landed on main at commit `7743486` on 2026-08-04 and is absent from the
v0.26.0 tag. [Upstream fix and tests](https://github.com/vllm-project/vllm/pull/50580)

This is the strongest explanation for the perceived gap. DeepSeek reports its
agent benchmark numbers using the new `max` prompt, while Finite's normal
Hermes request asked for `high` and vLLM 0.26 rendered that with the official
low/no-prefix prompt. Even explicitly requesting `max` on v0.26 only produced
the official 0731 high prefix.

### 3. The sampling/evaluation comparison was not apples-to-apples

DeepSeek's card says the published code-agent results used its minimal harness,
max reasoning, `temperature=1.0`, and `top_p=0.95`. It recommends
`temperature=1.0`, `top_p=0.95` for agentic scenarios, and `top_p=1.0`
otherwise. Short deterministic smoke prompts are useful for protocol safety,
but they do not validate parity with those quality results.

The next attempt should include a small fixed, scored quality set in all three
modes and compare:

1. official DeepSeek hosted `deepseek-v4-flash`;
2. self-hosted target-only 0731;
3. self-hosted DSpark 0731, only after exact-output/distribution checks pass;
4. the current GLM production baseline.

Use the same system prompt, tools, prompt history, effort, sampling, and output
budget in every lane.

### 4. DSpark was a runtime correctness failure, not evidence against the weights

Finite's DSpark-on vLLM 0.26.0/H200 candidate produced corrupt output and only
about 1% speculative acceptance after startup was made to work through NCCL.
Removing DSpark while retaining the same target weights restored coherent
generation. This isolates the observed corruption to the speculative runtime
path, not the 0731 target checkpoint.

The official DSpark launch is seven speculative tokens with greedy drafting:

```text
--speculative-config '{"method":"dspark","num_speculative_tokens":7,"draft_sample_method":"greedy"}'
```

[DeepSeek's official vLLM command](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash-0731#how-to-run-with-vllm),
[vLLM DSpark implementation PR](https://github.com/vllm-project/vllm/pull/46995)

## Published eight-H200 recipes

### Closest monolithic recipe: vLLM single-node TP/EP

The official vLLM recipe marks H200 as `verified`, selects 0731 as its default
variant, requires at least vLLM 0.25.0 for the fused checkpoint, and defines
single-node tensor plus expert parallelism as its default strategy. Its common
arguments are:

```text
--trust-remote-code
--kv-cache-dtype fp8
--block-size 256
--tensor-parallel-size 8
--enable-expert-parallel
--tokenizer-mode deepseek_v4
--tool-call-parser deepseek_v4
--enable-auto-tool-choice
--reasoning-parser deepseek_v4
```

On Hopper, the recipe disables FlashInfer autotuning for the TP-only latency
shape. The Blackwell-only `deep_gemm_mega_moe` and FP4 indexer-cache overrides
must not be copied to H200. Finite's TP8+EP topology was therefore broadly
aligned with the recipe.

However, the official recipe was changed on 2026-08-01 to pin the 0731 variant
to `vllm/vllm-openai:v0.25.0` because v0.26.0/nightly crashed within about 30
minutes with top-k and sparse-prefill assertions. Finite chose v0.26.0 anyway.
That upstream report is consistent with treating the runtime—not the
checkpoint—as the suspect.

[Current official recipe](https://github.com/vllm-project/recipes/blob/main/models/deepseek-ai/DeepSeek-V4-Flash.yaml),
[v0.26 rollback/pin PR](https://github.com/vllm-project/recipes/pull/723),
[exact pin change](https://github.com/vllm-project/recipes/commit/f53c5efd80be24c7eb365d08a2ededc2e86b17c8)

This recipe is first-party and says H200 is verified, but it does **not** publish
an eight-H200 0731 throughput, latency, acceptance-rate, or quality result.
It is configuration evidence, not a benchmark.

### Most explicit 8×H200 recipe: single-node disaggregated prefill/decode

The same vLLM recipe contains two explicit eight-H200 deployment shapes:

- four H200s for prefill, DP4+EP, `max-num-seqs=8`,
  `max-num-batched-tokens=16384`, eager execution;
- four H200s for decode, DP4+EP, `max-num-seqs=512`,
  `max-num-batched-tokens=512`, `FULL_DECODE_ONLY` CUDA graphs;
- FP8 KV, block size 256, and the hybrid KV-cache manager enabled on both;
- either Mooncake/RDMA or NIXL for intra-node KV transfer, fronted by vLLM
  Router.

This is a substantial topology change from Finite's one-container TP8 service,
so it is a later optimization path rather than the lowest-risk retry.

There are two copy/paste traps in the raw guide:

1. its H200 command blocks hard-code the preview repo
   `deepseek-ai/DeepSeek-V4-Flash`, even though the recipe's selected default
   variant is 0731;
2. those blocks omit DSpark and the tool parser flags.

For 0731, explicitly substitute `deepseek-ai/DeepSeek-V4-Flash-0731`, retain the
DeepSeek tokenizer/reasoning/tool parser flags, and make DSpark an explicit,
separately tested choice. Do not assume the raw guide performed those variant
substitutions.

[Official recipe, H200 PD sections](https://github.com/vllm-project/recipes/blob/main/models/deepseek-ai/DeepSeek-V4-Flash.yaml#L441),
[vLLM Router](https://github.com/vllm-project/router)

### Other first-party H200 evidence

NVIDIA Dynamo publishes DeepSeek V4 Flash configurations for four H200s
(aggregated DP4+TP1+EP) and 28 H200s (disaggregated 4P3D). These use the preview
checkpoint, MTP-1, and an NVIDIA runtime image; they are useful evidence for
Hopper parallelism and workload design, but are **not** an eight-H200 0731
recipe. [NVIDIA Dynamo DeepSeek V4 Flash recipe](https://docs.nvidia.com/dynamo/dev/recipes/deepseek-v4-flash)

Tinfoil's first-party DeepSeek V4 Pro enclave uses eight GPUs, vLLM, DP8+EP,
FP8 KV, DeepSeek V4 tokenizer/tool/reasoning parsers, and a private measured
deployment. It validates that the broad Tinfoil/vLLM/DeepSeek-V4 topology is
viable, but it serves V4 Pro rather than Flash-0731 and therefore cannot prove
the 0731 runtime or its DSpark path.
[Tinfoil DeepSeek V4 Pro config](https://github.com/tinfoilsh/confidential-deepseek-v4-pro/blob/main/tinfoil-config.yml)

## Recommended recipe for the next Finite attempt

The lowest-risk retry is a corrected monolithic eight-H200 candidate, not an
immediate move to PD disaggregation:

1. Keep the exact current checkpoint revision and existing Tinfoil MPK.
2. Keep the native FP4-expert/FP8-other weights and FP8 KV cache.
3. Use a digest-pinned vLLM build that contains the 0731 reasoning fix at
   `7743486` **and** has passed an isolated eight-H200 soak. Do not reuse stock
   v0.26.0. The conservative alternatives are a reviewed backport onto the
   official recipe's v0.25.x base or a later stable vLLM release after the fix
   lands and its H200 stability is demonstrated.
4. Start DP8+EP, target-only, with the same 393,216-token service ceiling. This
   follows Tinfoil's eight-GPU V4 Pro topology and the public H200 throughput
   lane while keeping the service monolithic. Keep Blackwell-only kernels off.
5. Make thinking behavior explicit at the client contract:
   top-level `reasoning_effort="high"` plus
   `chat_template_kwargs={"thinking": true}` by default, with top-level `max`
   for the quality comparison. Verify the rendered prompt byte-for-byte against
   the checkpoint's official encoder for low/high/max.
6. Validate target-only quality before enabling DSpark. If DSpark is retried,
   use the official greedy seven-token configuration and require coherent
   output, target-equivalence/distribution checks, and a meaningful acceptance
   rate before measuring speed.
7. Begin with concurrency 1/8/16/32. Add 64 only after a longer soak confirms no
   v0.26-style top-k or sparse-prefill crash.
8. Add long-context and tool-call canaries, not only short chat. In particular,
   preserve `reasoning_content` across tool-result turns and verify that raw
   DSML never leaks into `content` under concurrency.

## Evidence limits and open risks

- No first-party source found publishes a complete, measured
  **DeepSeek-V4-Flash-0731 + vLLM + 8×H200** throughput/quality benchmark.
- The vLLM recipe's H200 `verified` marker and explicit 4P/4D recipe are the
  strongest configuration evidence, but the guide mixes preview-model command
  text with a 0731 variant declaration.
- The quantitative DSpark vLLM evidence is on eight B300s and V4 Pro/preview,
  not 0731 on H200.
- Open/version-sensitive vLLM reports describe concurrent DSML corruption and
  long-context tool-call parsing failures. They are not proof of a failure in
  the preserved Finite target-only candidate, but they justify dedicated
  concurrency and long-context parser tests.

[Concurrent DSML report](https://github.com/vllm-project/vllm/issues/48089),
[long-context DSML report](https://github.com/vllm-project/vllm/issues/48931),
[DeepSeek V4 parser tracking](https://github.com/vllm-project/vllm/issues/41240)

## Decision

Do not replace or re-wrap the model: the checkpoint identity was correct.
Treat the next attempt as a **serving correctness upgrade** centered on the
0731 encoder/reasoning contract and an H200-stable vLLM pin. The model should
not be judged against DeepSeek's published 0731 quality until normal Finite
requests actually run thinking-high/max with the official prompt encoding.
