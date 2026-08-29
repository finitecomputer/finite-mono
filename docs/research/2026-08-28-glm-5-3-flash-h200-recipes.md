# GLM-5.3-Flash on 8xH200: who is running it, and what to change

Date: 2026-08-28

Status: research for the live `finite-private` container. No production
mutation in this note.

## Short answer

Yes. The exact shape we are running (native FP8 checkpoint, one 8xH200 node,
TP8/EP8, BF16 KV) is the official SGLang Hopper recipe, verified by LMSYS on
8xH200. vLLM publishes an H200 TP=8 FP8 recipe too. Nobody has published a
public 120-user tok/s number for this model on H200. The useful speed evidence
is LMSYS's own 8xH200 A/B from 2026-08-28, and it says our TileLang pin is the
wrong Hopper default.

FP8 KV cache is not available on H200 for this model. Weights are FP8; the
KV pool must stay BF16. That is a kernel gap, not a flag we missed.

## Who runs this shape

SGLang's GLM-5.3-Flash cookbook is the primary source. It has verified 8xH200
cells for both a low-latency arm (adaptive MTP 5/1/6) and a high-throughput arm
(speculative decoding off). Both use `zai-org/GLM-5.3-Flash` (native mixed FP8),
TP8/EP8, BF16 KV, DeepGEMM MoE, `glm45` / `glm47` parsers, and the
`lmsysorg/sglang:glm-5.3-flash` image. GSM8K on 8xH200: 97.04% low-latency,
97.35% recommended high-throughput. Those scores are accuracy, not speed.
[Cookbook](https://docs.sglang.io/cookbook/autoregressive/GLM/GLM-5.3-Flash),
[verification record, source `f040cc72e6`](https://github.com/sgl-project/sglang/pull/36660).

vLLM's recipe page also lists **H200 · TP=8 · FP8** for the same checkpoint,
with an explicit note that Hopper cannot use FP8 KV and must run BF16 KV. The
vLLM standard tree still had an open support PR when we cut our image; the
recipe uses a dedicated `vllm/vllm-openai:glm53-flash` image.
[vLLM recipe](https://recipes.vllm.ai/zai-org/GLM-5.3-Flash).

Z.ai's hosted API is the same weights served as a product, not an 8xH200
self-host recipe. Hugging Face lists SGLang, vLLM, TokenSpeed, Transformers,
KTransformers, and Unsloth as local serving options; only SGLang and vLLM
publish an 8xH200 command.
[Model card](https://huggingface.co/zai-org/GLM-5.3-Flash).

I found no third-party write-up that says "we serve GLM-5.3-Flash FP8 on 8xH200
and here is tok/s at 32 / 64 / 120." Finite's measurements from this evening
are the first H200 speed numbers I can cite that are not LMSYS's internal A/B.

## What we are running vs that recipe

Live command (release `v2026-08-28-glm-5-3-flash-2`) is the cookbook
high-throughput arm plus `--context-length 393216`:

```
--tp-size 8 --ep-size 8
--dsa-prefill-backend tilelang --dsa-decode-backend tilelang
--kv-cache-dtype bfloat16
--moe-runner-backend deep_gemm
--reasoning-parser glm45 --tool-call-parser glm47
```

That was the right first cut: the Finite runbook forbade speculative decoding
until measured, and the high-throughput cell is the verified no-MTP baseline.
Two things have moved under us since that pin.

### 1. Stop pinning TileLang on H200 (measured, same hardware)

SGLang PR #36895 (closed 2026-08-29, merge `70225f32`) measured on **8xH200,
TP8/EP8, real weights, `lmsysorg/sglang:glm-5.3-flash`**. Pinning
`--dsa-prefill-backend tilelang --dsa-decode-backend tilelang` overrides
auto-detection. On SM90 + BF16 KV, auto selects `flashmla_sparse` prefill +
`fa3` decode, and that pair is faster:

| 8xH200, 24k shared-prefix 3-turn, 16k chunked prefill | c=32 | c=128 |
| --- | --- | --- |
| tilelang (what we pin) | 541.1 tok/s | 299.8 tok/s |
| auto (`flashmla_sparse` / `fa3`) | 589.9 (**+9.0%**) | 346.5 (**+15.6%**) |
| TTFT p95 | 20.98s vs 24.04s (**−12.7%**) | 142.3s vs 162.4s (−12.4%) |

GSM8K A/B on the same box: 92.2% tilelang vs 91.9% auto, not significant.
DeepGEMM stays; swapping MoE to Triton was 6–12% slower.
[PR #36895](https://github.com/sgl-project/sglang/pull/36895).

This is the one small flag change with first-party 8xH200 evidence. Our 32-way
thinking-on aggregate tonight was 218 tok/s with 33s TTFT; this will not close
the 2,400 aggregate / 10s TTFT bars by itself, but it is the cheapest win and
it attacks TTFT, which is our actual miss.

Do not try `--kv-cache-dtype fp8` on this box. GLM-5.3-Flash's DSA indexer
(`index_kpool > 1`) has no CUDA BF16-query × FP8-KV path on Hopper. SGLang
issue #36830: same cluster, same image, GLM-5.2 FP8 KV works, 5.3 Flash does
not, and the loss is ~1.75–2.0× KV capacity. Blackwell gets FP8 KV + TRT-LLM
as the cookbook default; Hopper reseats to BF16 + the DSA pair above.
[Issue #36830](https://github.com/sgl-project/sglang/issues/36830),
[Blackwell default PR #36519](https://github.com/sgl-project/sglang/pull/36519).

### 2. Do not copy the low-latency MTP arm yet

The interactive cookbook command adds:

```
--mem-fraction-static 0.75
--speculative-algorithm EAGLE
--speculative-num-steps 5 --speculative-eagle-topk 1
--speculative-num-draft-tokens 6 --speculative-adaptive
```

That is verified for GSM8K on 8xH200, not for our protocol/tool/soak gates.
Cookbook copy: start there for chat/agent, measure high-throughput when the
batch is the point. MTP helps decode; it does not fix a 33s time-to-first-token
under 32-way thinking, and it is a second correctness variable. Keep it as the
next one-variable A/B after the DSA swap.

Do not add `--disable-shared-experts-fusion` on this image. Cookbook restored
it in PR #36519 after fusion silently degenerated answers, then dropped it
again in PR #36544 once the fusion-gate fix made EP>1 build the unfused path
with no flag. Live `flash-2` (this image, no flag) already returns real
answers and stops. Forcing the flag on `glm5_next` has a recorded startup
crash with some MoE runners (SGLang issue #36830's neighbor #36711). Leave
the serving command matching the live-proven fusion path.
[PR #36519](https://github.com/sgl-project/sglang/pull/36519),
[PR #36544](https://github.com/sgl-project/sglang/pull/36544).

### 3. Memory split is a real lever we cannot copy as a flag

`--mamba-full-memory-ratio` default `0.9` under-provisions the KV pool on the
H200 BF16 cell badly enough to cost **−45% throughput at c=128** in LMSYS's
follow-up. The measured optimum across configs spans 0.255 to 11.4. There is
no single published value. Tune from the two pool sizes printed at boot and
the average request length, or pin `--max-mamba-cache-size`. We do not have
those boot numbers off the Tinfoil box yet (limiter `/metrics` is not SGLang's).
[Cookbook memory section](https://docs.sglang.io/cookbook/autoregressive/GLM/GLM-5.3-Flash),
[PR #36895 follow-up](https://github.com/sgl-project/sglang/pull/36895).

Chunked-prefill size is the other scheduler knob. On GLM-5.2, raising
`--chunked-prefill-size` from 2048 to 32768 cut TTFT 39–59% on 8xH200 at
8k-in/1k-out. The 5.3 Flash A/B above used 16k. Worth a later trial; not the
first delta.
[GLM-5.2 cookbook](https://docs.sglang.io/cookbook/autoregressive/GLM/GLM-5.2).

## Default thinking: high, not max

Z.ai and the weight card agree thinking cannot be turned off for GLM-5.3-Flash,
and `reasoning_effort` is `low` | `high` | `max` with **default `max`** if the
field is omitted. Z.ai recommends `max` for coding. The user request is
Finite's default `high`.
[Z.ai thinking guide](https://docs.z.ai/guides/capabilities/thinking),
[Z.ai GLM-5.3-Flash guide](https://docs.z.ai/guides/vlm/glm-5.3-flash),
[model card](https://huggingface.co/zai-org/GLM-5.3-Flash).

Open-weight SGLang still accepts `chat_template_kwargs.enable_thinking=false`
(our protocol gate uses that). The checkpoint default for an omitted
`reasoning_effort` is still `max`. A limiter fill-if-absent of
`reasoning_effort=high` plus `enable_thinking=true` is the right place: every
client that hits Finite Private gets it, including old `deepseek-v4-flash-0731`
aliases that now land on GLM. Explicit client values win. This is also a speed
change versus today's implicit `max`.

Hermes/runtime defaults stay untouched (cutover runbook: do not migrate durable
Runtime configuration in the model window).

## Recommended order

1. DSA pin → `flashmla_sparse` / `fa3`. Same GLM image, new measured Tinfoil
   tag, one replace. Re-run 1/32-way. Do not add `--disable-shared-experts-fusion`.
2. Limiter default `reasoning_effort=high` when omitted.
3. After boot logs exist: recompute `--mamba-full-memory-ratio`.
4. Separate window: adaptive MTP 5/1/6, one variable, protocol gate first.
5. Do not do PD disaggregation, DFlash2, or FP8 KV on this host.

Item 1 will not make the 120-user gate pass. It is the recipe LMSYS is now
telling every 8xH200 operator to run.
