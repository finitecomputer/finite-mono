# GLM-5.3-Flash on eight Tinfoil H200s: initial assessment

Date: 2026-08-27

Status: **lab candidate only**. No production container, model default, durable
agent configuration, route, limiter, or persisted chat state was changed.

## Recommendation

GLM-5.3-Flash is unusually promising for Finite's workload: it is a permissively
licensed, natively multimodal 320B-total/18B-active MoE with a configured 1M-token
context, native tool calling, and much lower raw weight memory than the prior GLM
5.2 family. There is also an official SGLang recipe marked verified on exactly
eight H200s. It should be the next isolated eight-H200 lab candidate.

It should **not** replace DeepSeek V4 Flash 0731 as the main model yet. The model
was announced on 2026-08-26, its untagged Hugging Face `main` changed repeatedly
during this investigation, the official H200 evidence is accuracy-only, and the
standard vLLM release does not yet contain support. No first-party H200 throughput,
latency, concurrency, cold-start, near-1M-context, or sustained-load result exists.
Those are material gaps against Finite's already measured production baseline.
[Z.ai announcement](https://z.ai/blog/glm-5.3-flash),
[official model repository](https://huggingface.co/zai-org/GLM-5.3-Flash),
[SGLang GLM-5.3-Flash cookbook](https://docs.sglang.io/cookbook/autoregressive/GLM/GLM-5.3-Flash),
[open vLLM support PR](https://github.com/vllm-project/vllm/pull/53906).

## Fixed identity for a lab trial

- Product/API name: `GLM-5.3-Flash` / `glm-5.3-flash`. Z.ai announced it on
  2026-08-26. The open checkpoint is a newly trained base rather than a
  post-training revision of GLM-5.2.
  [Official guide](https://docs.z.ai/guides/vlm/glm-5.3-flash),
  [official announcement](https://z.ai/blog/glm-5.3-flash).
- Candidate weights: `zai-org/GLM-5.3-Flash`, revision
  `04c4e9e95c5da8862dced7e5056455116f83a7e0` as observed on 2026-08-27.
  The repository has no release tag, so this SHA is part of the candidate
  identity, not a replaceable detail.
  [Hugging Face model API](https://huggingface.co/api/models/zai-org/GLM-5.3-Flash),
  [pinned config](https://huggingface.co/zai-org/GLM-5.3-Flash/blob/04c4e9e95c5da8862dced7e5056455116f83a7e0/config.json).
- The default checkpoint is mixed precision, not pure FP8: approximately
  314.397B FP8 E4M3 parameters, 6.926B BF16 parameters, and a small F32 set.
  Its 62 safetensor shards total 328,337,455,672 bytes (305.8 GiB). The separate
  official BF16 checkpoint totals 642,652,070,880 bytes (598.5 GiB). Start with
  the native checkpoint; do not introduce an unofficial FP4/NVFP4 conversion
  into the first correctness trial.
  [FP8 repository inventory](https://huggingface.co/api/models/zai-org/GLM-5.3-Flash),
  [FP8 shard index](https://huggingface.co/zai-org/GLM-5.3-Flash/blob/04c4e9e95c5da8862dced7e5056455116f83a7e0/model.safetensors.index.json),
  [BF16 repository](https://huggingface.co/zai-org/GLM-5.3-Flash-BF16).

The checkpoint is MIT licensed: use, modification, redistribution, sublicensing,
and sale are allowed if the copyright and license notice are preserved; the
license disclaims warranty. No separate acceptable-use addendum appears in the
weight repository. Z.ai's hosted API terms are materially more restrictive and
should not be treated as the weight license.
[Pinned weight license](https://huggingface.co/zai-org/GLM-5.3-Flash/blob/04c4e9e95c5da8862dced7e5056455116f83a7e0/LICENSE),
[Z.ai hosted-service terms](https://docs.z.ai/legal-agreement/terms-of-use).

## Architecture and product capabilities

The official rounded size is 320B total / 18B active; the tensor inventory is
321.323B parameters. The text model has 45 layers: 34 KDA linear-attention layers
and 11 DeepSeek sparse-attention layers. It uses mHC, 288 routed experts plus one
shared expert, eight routed experts per token, three initial dense layers, and one
NextN/MTP draft layer. The configured maximum position count is exactly 1,048,576.
The vision tower has 24 layers and supports image and video input. These are
checkpoint properties, not proof of useful quality or stable serving across a
full 1M-token request.
[Pinned config](https://huggingface.co/zai-org/GLM-5.3-Flash/blob/04c4e9e95c5da8862dced7e5056455116f83a7e0/config.json),
[model card](https://huggingface.co/zai-org/GLM-5.3-Flash).

Z.ai documents text, image, video, and file input; function calling and streamed
tool calls; context caching; reasoning-effort control; and JSON-object output.
The chat template accepts `low`, `high`, and `max` reasoning effort and falls
back to `max`. The API's structured-output mode is JSON-object generation followed
by client-side schema validation, not a promise that the hosted API enforces an
arbitrary supplied JSON Schema. Function selection is documented as automatic,
so Finite must separately test forced/required-tool semantics if any consumer
depends on them.
[Model guide](https://docs.z.ai/guides/vlm/glm-5.3-flash),
[function-calling guide](https://docs.z.ai/guides/capabilities/function-calling),
[structured-output guide](https://docs.z.ai/guides/capabilities/struct-output),
[pinned chat template](https://huggingface.co/zai-org/GLM-5.3-Flash/blob/04c4e9e95c5da8862dced7e5056455116f83a7e0/tokenizer_config.json).

There is a protocol ambiguity to resolve experimentally. The model card says
`clear_thinking` defaults to `false` and tells chat clients to set it to `true`;
Z.ai's model/API material and SGLang's request-level `thinking` switch describe
different behavior, while the vLLM recipe says thinking is always opened by the
generation prompt. Finite must pin one explicit history policy and verify
multi-turn tool loops rather than inheriting a default.
[Model card note](https://huggingface.co/zai-org/GLM-5.3-Flash),
[SGLang reasoning behavior](https://docs.sglang.io/cookbook/autoregressive/GLM/GLM-5.3-Flash),
[vLLM recipe](https://recipes.vllm.ai/zai-org/GLM-5.3-Flash).

## Eight-H200 fit and serving maturity

| Concern | Current Finite Private production | GLM-5.3-Flash evidence today |
| --- | --- | --- |
| Model | DeepSeek V4 Flash 0731 | Z.ai GLM-5.3-Flash native mixed FP8 |
| Hardware | One 8xH200 Tinfoil allocation | Official SGLang 8xH200 recipe |
| Shape | vLLM DP8 + EP, TP1 | SGLang TP8 + EP8 |
| KV cache | FP8 `fp8_ds_mla` | BF16 on Hopper; FP8 KV is disabled in the official H200 recipe |
| Service context | 393,216, proven near limit | 1,048,576 configured; not proven near limit on H200 |
| Measured speed | 8,373 output tok/s at 1,024 clients; about 55 tok/s single session | No official H200 speed measurement |
| H200 correctness | Finite protocol gates and soak passed | SGLang reports full-GSM8K accuracy only: 97.04% low-latency, 97.35% recommended high-throughput |
| Runtime maturity | Pinned custom vLLM 0.25.1 image, production measured | Dedicated SGLang image is verified; vLLM standard integration is still open |

The current baseline and its measurement method are preserved in
[the eight-H200 optimization note](2026-08-07-deepseek-v4-eight-h200-optimization.md).
The GLM side of the table comes from SGLang's official cookbook, whose verified
H200 commands use the native FP8 checkpoint, TP8/EP8, BF16 KV, TileLang sparse
attention, DeepGEMM MoE, and the existing `glm45` reasoning / `glm47` tool
parsers. Its own notes say the H200 results are accuracy-only, not speed results.
[SGLang H200 recipe and evidence](https://docs.sglang.io/cookbook/autoregressive/GLM/GLM-5.3-Flash).

Raw capacity is favorable but is not a concurrency estimate. The native weights
are about 305.8 GiB across eight 141 GB H200s, versus 598.5 GiB for BF16. The
hybrid architecture also needs a separate KDA state pool in addition to paged KV;
SGLang warns that either pool can cap concurrency and that its generic memory
split must be workload-tuned. A TP8 native-weight lab launch is therefore the
lowest-risk first topology, but startup-reported pools and actual admission under
Finite traffic remain the authority.
[SGLang memory guidance](https://docs.sglang.io/cookbook/autoregressive/GLM/GLM-5.3-Flash),
[official weight index](https://huggingface.co/zai-org/GLM-5.3-Flash/blob/04c4e9e95c5da8862dced7e5056455116f83a7e0/model.safetensors.index.json).

### Engine decision for the lab

- **SGLang is the evidence-leading first trial.** It publishes a dedicated
  `lmsysorg/sglang:glm-5.3-flash` image and marks the exact 8xH200 TP8/EP8
  commands verified. Pin the resulting image digest; never deploy the floating
  tag. [Official cookbook](https://docs.sglang.io/cookbook/autoregressive/GLM/GLM-5.3-Flash).
- **vLLM is not a production candidate today.** vLLM 0.28.0 was the latest
  release during this investigation, while GLM support PR #53906 remained open
  with an empty test plan. The official recipe requires a dedicated image/nightly
  and calls 0.29.0 the minimum future version; it inconsistently names FlashInfer
  0.6.17 and 0.6.18 as the minimum. This is useful for a later A/B, not the first
  production path. [vLLM release](https://github.com/vllm-project/vllm/releases/tag/v0.28.0),
  [support PR](https://github.com/vllm-project/vllm/pull/53906),
  [recipe source](https://github.com/vllm-project/recipes/blob/main/models/zai-org/GLM-5.3-Flash.yaml).
- **TGI is not a serious candidate.** GLM5 Next is absent from TGI's optimized
  model list; its generic Transformers fallback loses tensor-parallel sharding
  and Flash Attention, and Hugging Face says TGI is in maintenance mode while
  recommending vLLM and SGLang. [TGI supported models](https://huggingface.co/docs/text-generation-inference/supported_models),
  [non-core fallback limitations](https://huggingface.co/docs/text-generation-inference/basic_tutorials/non_core_models),
  [TGI maintenance notice](https://github.com/huggingface/text-generation-inference).

SGLang's `--served-model-name` is a single string, unlike the current vLLM
configuration's multiple accepted names. The production service currently accepts
the canonical `deepseek-v4-flash-0731` plus the mixed-version `glm-5-2` alias.
Moving to SGLang therefore requires an explicit limiter/request-rewrite contract
or a coordinated client migration; silently dropping either reader would break
mixed-version chat.
[SGLang server argument type](https://github.com/sgl-project/sglang/blob/d1f14431fdf036b386bec347461df004c99ed88c/python/sglang/srt/server_args.py#L1349-L1352),
[current candidate aliases](../../infra/tinfoil/confidential-kimi-k2-6/tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml).

## Concrete eight-H200 recipes found

There are two official, single-node 8xH200 SGLang recipes. Both use the native
FP8 checkpoint with TP8/EP8, BF16 KV, TileLang for DSA prefill and decode, and
DeepGEMM for MoE. The low-latency arm adds adaptive EAGLE MTP 5/1/6 and
`--mem-fraction-static 0.75`; the high-throughput arm deliberately omits both
speculative decoding and an explicit static-memory fraction. Neither command
sets a context cap, so the checkpoint's 1,048,576-token maximum remains the
configured ceiling. Both leave the KDA state split at SGLang's generic
`--mamba-full-memory-ratio 0.9` default, which the cookbook explicitly says to
tune from the KV and KDA pool sizes printed at boot.
[Pinned SGLang recipe source](https://github.com/sgl-project/sglang/blob/6ff2a20ccf64b64a6bb6d9a54c7b0e605f673da2/docs/src/snippets/configs/zai-org/glm-5.3-flash.jsx),
[memory and backend guidance](https://docs.sglang.io/cookbook/autoregressive/GLM/GLM-5.3-Flash).

| Official arm | Exact H200 serving shape | Verification level |
| --- | --- | --- |
| Low latency | TP8, EP8, `mem-fraction-static=0.75`, BF16 KV, TileLang DSA, DeepGEMM, adaptive EAGLE with 5 steps / top-k 1 / 6 draft tokens | Marked Verified. Full GSM8K: 97.04%. Accuracy only; no H200 speed, context-limit, multimodal, tool-loop, concurrency, or soak result. |
| High throughput | TP8, EP8, BF16 KV, TileLang DSA, DeepGEMM, speculative decoding off | Marked Verified and recommended for sustained batches. Full GSM8K: 97.35% for the recommended selection, 97.19-97.57% across measured selections. Accuracy only. |

The H200 verification record names SGLang source `f040cc72e6`, but does **not**
record an exact Hugging Face revision. Its GSM8K command also left thinking off,
so those scores are not directly comparable to the thinking-enabled GB300 rows.
The current candidate checkpoint SHA in this note is therefore a new supply-chain
identity that must be re-gated; it is not byte-for-byte covered by the published
H200 result.
[Pinned SGLang benchmark record](https://github.com/sgl-project/sglang/blob/6ff2a20ccf64b64a6bb6d9a54c7b0e605f673da2/docs/src/snippets/configs/zai-org/glm-5.3-flash-benchmarks.jsx).

### Exact SGLang launch commands

Low latency / interactive agent traffic:

```bash
sglang serve \
  --model-path zai-org/GLM-5.3-Flash \
  --tp-size 8 \
  --ep-size 8 \
  --mem-fraction-static 0.75 \
  --dsa-prefill-backend tilelang \
  --dsa-decode-backend tilelang \
  --kv-cache-dtype bfloat16 \
  --moe-runner-backend deep_gemm \
  --speculative-algorithm EAGLE \
  --speculative-num-steps 5 \
  --speculative-eagle-topk 1 \
  --speculative-num-draft-tokens 6 \
  --speculative-adaptive \
  --reasoning-parser glm45 \
  --tool-call-parser glm47 \
  --host 0.0.0.0 \
  --port 30000
```

High throughput / correctness-first baseline:

```bash
sglang serve \
  --model-path zai-org/GLM-5.3-Flash \
  --tp-size 8 \
  --ep-size 8 \
  --dsa-prefill-backend tilelang \
  --dsa-decode-backend tilelang \
  --kv-cache-dtype bfloat16 \
  --moe-runner-backend deep_gemm \
  --reasoning-parser glm45 \
  --tool-call-parser glm47 \
  --host 0.0.0.0 \
  --port 30000
```

Start Finite's trial with the second command. It removes MTP as a correctness
variable; the low-latency arm can then be tested as a one-variable change.
The published CUDA image is `lmsysorg/sglang:glm-5.3-flash`. On 2026-08-27 the
registry resolved that mutable tag to OCI index
`sha256:e6f5482505e7502f791fe4615ad1fbec118cbbd6b44e98f2479b16b98b985ad6`
and its Linux/amd64 manifest to
`sha256:0836f0160fa785e424e68d13ef88ddd548f87e6e11ad9f0e4de982e4f9188aaf`.
The image's provenance labels report the build commit as `unknown`, so the
digest is necessary but does not independently establish source provenance.
Re-resolve the tag immediately before the lab, approve one digest, and preserve
the inspected image metadata with the experiment.
[Official Docker Hub tag record](https://hub.docker.com/v2/repositories/lmsysorg/sglang/tags/glm-5.3-flash).

### Tinfoil-shaped adaptation

No first-party Tinfoil GLM-5.3-Flash recipe exists. The official SGLang command
does map cleanly onto a Tinfoil eight-GPU container, but this is a proposed lab
adaptation, not an upstream-verified Tinfoil configuration:

```yaml
gpus: 8
models:
  - name: "glm-5-3-flash"
    repo: "zai-org/GLM-5.3-Flash@04c4e9e95c5da8862dced7e5056455116f83a7e0"
    mpk: "<generated-by-tinfoil-models-tab>"

containers:
  - name: "glm-5-3-flash"
    image: "lmsysorg/sglang:glm-5.3-flash@sha256:0836f0160fa785e424e68d13ef88ddd548f87e6e11ad9f0e4de982e4f9188aaf"
    runtime: nvidia
    gpus: all
    ipc: host
    command:
      - sglang
      - serve
      - --model-path
      - /tinfoil/mpk/mpk-<root-hash-from-generated-mpk>
      - --served-model-name
      - glm-5.3-flash
      - --tp-size
      - "8"
      - --ep-size
      - "8"
      - --dsa-prefill-backend
      - tilelang
      - --dsa-decode-backend
      - tilelang
      - --kv-cache-dtype
      - bfloat16
      - --moe-runner-backend
      - deep_gemm
      - --reasoning-parser
      - glm45
      - --tool-call-parser
      - glm47
      - --host
      - 0.0.0.0
      - --port
      - "8001"
```

The MPK value and root-hash path cannot be filled in until Tinfoil wraps the
pinned checkpoint. Tinfoil requires a digest-pinned image, top-level `gpus: 8`,
`runtime: nvidia`, container `gpus: all`, and normally `ipc: host`; it mounts the
verified model read-only at `/tinfoil/mpk/mpk-<root_hash>`. The rest of the
existing limiter, networks, shim, health checks, secrets, and 45-minute startup
allowance can be retained for a model-only lab comparison, but model-name
normalization remains required because SGLang accepts only one served name.
[Tinfoil model wrapping and mount contract](https://docs.tinfoil.sh/containers/models),
[Tinfoil GPU and image configuration](https://docs.tinfoil.sh/containers/configuration).

### vLLM recipe status

vLLM now has an official model recipe, but not an exact monolithic 8xH200
recipe. Its explicit examples are TP4 on GB200 and a single-node TP4+TP4
prefill/decode split; the recipe says Hopper must use BF16 KV. It requires the
mutable dedicated image `vllm/vllm-openai:glm53-flash` until support lands,
labels vLLM 0.29.0 as the minimum future version, and publishes no immutable
image digest in the recipe. The registry tag resolved on 2026-08-27 to index
`sha256:2c6da6c6f16ed15c91e412d896dba13701f25fe1861eaec9ddaa4db34d1d21c4`
and Linux/amd64 manifest
`sha256:2e771fa615452282cc331eb418b3ef21636fce355bea0491fca89e6d362ab703`;
that is supply-chain identity, not H200 verification. The implementation PR
remains open and contains no test plan or test result. This is supporting
evidence for a later engine A/B, not a reproducible eight-H200 starting point.
[Official vLLM recipe](https://recipes.vllm.ai/zai-org/GLM-5.3-Flash),
[recipe source](https://github.com/vllm-project/recipes/blob/7997f1d1bf1b7785a0367f19d2614cc3043c5948/models/zai-org/GLM-5.3-Flash.yaml),
[open support PR](https://github.com/vllm-project/vllm/pull/53906),
[official Docker Hub tag record](https://hub.docker.com/v2/repositories/vllm/vllm-openai/tags/glm53-flash).

## Upstream benchmark claims: useful signal, not a promotion gate

Z.ai reports GLM-5.3-Flash scores of 84.3 on Terminal Bench 2.1, 63.4 on
DeepSWE v1.1, 26.3 on Agents' Last Exam, 48.8 on AutomationBench, 55.3 on HLE
with tools, and 1773 on GDPval-AA v2. Every displayed comparison against GLM-5.2
improves, while comparisons with closed models are mixed. The chart is a vendor
self-report, not a reproducible Finite quality measurement.
[Official benchmark chart](https://github.com/zai-org/GLM-5/blob/main/resources/bench_53.png),
[official model card footnotes](https://huggingface.co/zai-org/GLM-5.3-Flash).

The footnotes materially limit cross-model interpretation: HLE used up to 163,840
generated tokens, 300K managed context, and an LLM judge; NL2Repo used 1M context,
64K generation, and rule/LLM anti-hacking checks; DeepSWE and Terminal Bench had
six-hour limits; Toolathlon is pass@1 averaged over three runs; and the Agents'
Last Exam footnote is blank. None establishes interactive latency, long-context
fidelity, tool-loop durability, or safety for Finite.
[Official model card footnotes](https://huggingface.co/zai-org/GLM-5.3-Flash).

## Security, privacy, and disclosure gaps

Self-hosting the MIT weights inside a measured Tinfoil enclave avoids sending
user prompts to Z.ai. Tinfoil's confidentiality claim still depends on clients
verifying the measured config/image and establishing an encrypted connection to
the attested enclave. The model change therefore needs a public, digest-pinned
satellite config and a released measurement; a plain HTTPS health check is not
privacy proof. Debug-mode Tinfoil containers deliberately fail attestation.
[Tinfoil verification architecture](https://docs.tinfoil.sh/verification/verification-in-tinfoil),
[Tinfoil container configuration](https://docs.tinfoil.sh/containers/configuration),
[Tinfoil container limitations](https://docs.tinfoil.sh/containers/overview).

No Flash-specific technical report, system card, safety card, red-team results,
data cutoff, corpus provenance detail, bias evaluation, hallucination study,
copyright/privacy analysis, or cyber-safety evaluation was found. The model card
cites the GLM-5 report submitted in February 2026, which predates this newly
trained model and does not document this architecture. Tool ability must not be
confused with runtime authority: tool names/arguments remain untrusted model
output and Finite's existing authorization and sandbox boundaries must remain
authoritative.
[GLM-5 report history](https://arxiv.org/abs/2602.15763),
[GLM-5.3-Flash model card](https://huggingface.co/zai-org/GLM-5.3-Flash).

## New-user default is not an existing-user cutover

Changing the runtime default would seed GLM only into new Hermes configs.
Existing agent model/provider state is durable and intentionally preserved after
first boot; the only existing narrow migration recognizes the exact legacy
`glm-5-2` Finite Private shape and moves it to the current canonical model.
Consequently, a new default alone creates a mixed population rather than migrating
existing DeepSeek agents. Before promotion, choose and prove one explicit contract:

1. new users get GLM while existing users stay on DeepSeek indefinitely; or
2. a narrowly matched, rollback-safe durable-config migration moves eligible
   existing users, preserving deliberate custom selections and mixed versions.

The relevant writers and preservation rules are
[`run_hermes_gateway.sh`](../../finitechat/containers/agent/run_hermes_gateway.sh)
and [`reconcile_hermes_config.py`](../../finitechat/containers/agent/reconcile_hermes_config.py).
Neither should be changed as part of the initial model lab.

## Required staged gates

1. **Freeze supply chain:** pin the model SHA, verify every shard/checksum, build
   and pin an SGLang image digest, create a Tinfoil model pack/MPK, and preserve
   the exact launch command. Re-check upstream revisions immediately before the
   trial because the repository is changing quickly.
2. **Isolated boot and memory proof:** use the disposable eight-H200 lab, TP8/EP8,
   native mixed-FP8 weights, and BF16 KV. Record startup time, peak host/GPU
   memory, KDA and KV pool sizes, graph capture, kernels, and every parsed option.
3. **Protocol proof:** gate reasoning separation, all three effort levels,
   explicit history/`clear_thinking` behavior, streaming and non-streaming tool
   calls, parallel and multi-turn tool results, JSON-object/schema validation,
   current model aliases, usage accounting, cancellation, and malformed input.
   Test text first, then image/video as separate capability gates.
4. **Quality proof:** run the existing Finite scored reasoning/tool suite against
   the self-hosted candidate and current production with blinded review. Add
   long-horizon coding, prompt-injection/tool-argument abuse, and chat-history
   regression cases. Upstream benchmark wins do not substitute for this gate.
5. **Capacity proof:** reuse unique prompts and the existing 64/256/512/1,024
   client shapes; measure TTFT, TPOT, output throughput, errors, and single-user
   speed. Test MTP off first, then adaptive MTP as a one-variable experiment.
6. **Context and stability:** prove ordinary chat, expected long context, and a
   near-limit request without assuming the full 1M ceiling is economically
   useful. Run the normal 35-minute stability gate, warm every admitted shape,
   and re-run protocol gates after soak.
7. **Compatibility and recovery:** prove limiter rewrite/served names, new-user
   enrollment through chat readiness, existing durable-agent behavior, mixed
   versions, exact DeepSeek rollback authority, and the multi-GPU downtime window.
   Tinfoil documents that multi-GPU updates are not zero-downtime.
   [Tinfoil update behavior](https://docs.tinfoil.sh/containers/updates).
8. **Only then consider promotion:** run `scripts/finite-status` before and after
   any authorized rollout and preserve the current DeepSeek release/config as the
   rollback boundary. Production mutation requires separate explicit approval.

## Decision boundary

The next action is a reproducible lab branch/config and measurement run, not a
change to the main model. Promotion becomes a reasonable decision only after the
candidate matches Finite's protocol and quality contract, demonstrates an
acceptable H200 latency/throughput envelope, and has a clear answer for aliases,
existing durable agents, new-user readiness, downtime, and rollback.
