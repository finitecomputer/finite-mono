# Finite Private: DeepSeek-V4-Flash-0731 cutover

This is the planned-downtime procedure for replacing the current GLM 5.2
model in the existing Finite Private Tinfoil enclave with the official
`deepseek-ai/DeepSeek-V4-Flash-0731` checkpoint. The successful-path downtime
target is one hour. Preparation, image proof, and measured satellite releases
must finish before that hour starts.

This runbook cements the plan; it does not authorize creating releases or
relaunching production. Those mutations require fresh explicit approval. The
supporting model/runtime evidence is in
[the migration research note](../../docs/research/2026-08-03-deepseek-v4-flash-0731-vllm-migration.md).

## Fixed decisions

| Area | Decision |
| --- | --- |
| Model | Official `deepseek-ai/DeepSeek-V4-Flash-0731` checkpoint, pinned to HF revision `7872f01b1d1fe23eabc4c98b48bffcef5a386062` |
| Weight format | Use the official native FP4-expert/FP8-other checkpoint; do not substitute a custom all-FP8 conversion |
| KV cache | Explicitly FP8 |
| Runtime | vLLM 0.26.0, the latest stable release verified on 2026-08-03, pinned by x86_64 image digest; never a nightly or a floating `latest` tag |
| Context | Start at the service's existing 393,216-token ceiling, not the checkpoint's full 1M maximum |
| Speculation | DSpark with target-model verification; do not use approximate `synthetic` rejection sampling |
| Compatibility | Preserve the public endpoint, private-vLLM → limiter → shim topology, sealed secret names, Core accounting, and `glm-5-2` compatibility alias |
| Rollback | Keep both a measured DSpark-off DeepSeek tag and the current measured GLM tag ready before downtime |

The DeepSeek image replaces GLM-specific parser, sparse-attention, DCP, and
MTP arguments with the official DeepSeek V4 flags. Its starting shape includes
`--trust-remote-code`, `--kv-cache-dtype fp8`, `--block-size 256`, DeepSeek V4
tokenizer/tool/reasoning parsers, H200-supported kernel defaults, and the
official seven-token DSpark configuration. The official recipe's
`deep_gemm_mega_moe` and FP4 indexer-cache additions are Blackwell overrides;
do not carry them onto this H200 deployment by assumption. Parallelism and
scheduler limits must be recorded in the measured candidate; they are not
live-tuning knobs during the window.

## vLLM version gate

As of 2026-08-03, the latest stable upstream release is
[vLLM 0.26.0](https://github.com/vllm-project/vllm/releases/tag/v0.26.0).
The official
[DeepSeek V4 recipe](https://github.com/vllm-project/recipes/blob/main/models/deepseek-ai/DeepSeek-V4-Flash.yaml)
sets 0.25.0 as the minimum for the 0731 DSpark checkpoint and marks H200 as a
verified target. Version 0.26.0 adds further DeepSeek V4 performance work, so
it is tonight's candidate rather than merely satisfying the minimum.

At the time of verification, the official x86_64 image resolved as follows:

```text
vllm/vllm-openai:v0.26.0
index: sha256:ffb2d59b1c059a5bd8d781320c9f5189de8293693b7d95da54befddaa54abf52
linux/amd64: sha256:770fe65b2c73ee74a5c42165cf3433de4048cc2cd9c57a937ca4e35aba5aa87b
```

The Tinfoil config pins the official linux/amd64 image digest directly; no
Finite-specific vLLM fork is required. Docker/CI proof must record `vllm
--version` as exactly `0.26.0`. Before proof, re-read the upstream stable
release and recipe. A newer tag is not adopted automatically inside this
maintenance change. If 0.26.0 cannot load or pass the H200 preflight, stop and
record the evidence; using the recipe's 0.25.0 image is an explicit fallback
decision, not a silent downgrade.

Prep proof on 2026-08-03 pulled the exact linux/amd64 child digest above and
read `vllm=0.26.0`, `torch=2.11.0+cu130`, and CUDA build `13.0` from the image.
Static inspection of that same image also confirmed the DeepSeek V4 model,
DeepSeek V4 parsers, DSpark implementation, and every staged CLI argument.
The image contains `/usr/bin/curl`, so the retained container healthcheck is
available without a derived image.
The installed vLLM API server reads `VLLM_API_KEY` directly, so the existing
sealed internal-key path remains valid without exposing the model container.
The H200 launch and CUDA-driver compatibility checks remain maintenance-window
preflight gates; package inspection on a non-GPU workstation cannot prove
them.

## Success definition

The replacement is acceptable only when all of these are true:

1. Chat, streaming, tool calls, reasoning, Responses, auth rejection, Core
   reservation/settlement, and a real Hermes conversation pass.
2. Concurrency 1, 4, 8, 16, and 32 completes without HTTP failures, stuck
   reservations, or p99 time-to-first-byte reaching 90 seconds.
3. The service remains at least as capable as GLM under the current load. A
   measured 2× aggregate-throughput improvement is the target; 3× is upside.
   A healthy result below 2× is recorded honestly and requires an operator
   decision, but is not an automatic rollback by itself.
4. The exploratory 64, 128, and 256 tiers either pass or reveal a clean,
   recoverable saturation boundary. They do not become advertised capacity
   merely because a short test succeeds.

## Preconditions — finish before downtime

1. Pull and prove the pinned official vLLM 0.26.0 linux/amd64 image at the
   Docker/CI rung. Run `vllm --version` inside it and require exactly `0.26.0`.
   Record the image tag/digest and CUDA/runtime versions. Nothing is built on
   the production enclave. If a patch becomes necessary, stop: a derived image
   needs its own reviewed source, CI build, immutable digest, and proof.

   ```bash
   docker run --rm --platform linux/amd64 \
     --entrypoint python3 \
     vllm/vllm-openai@sha256:770fe65b2c73ee74a5c42165cf3433de4048cc2cd9c57a937ca4e35aba5aa87b \
     -c 'import importlib.metadata; print(importlib.metadata.version("vllm"))'
   ```
2. In **Tinfoil Containers → Models**, prepare the exact pinned Hugging Face
   revision. Copy the generated modelwrap MPK and its root hash into both
   checked-in DeepSeek candidate files, then require:

   ```bash
   python3 scripts/check_finite_private_deepseek_candidate.py --release-ready
   ```

   Prep completed this wrap as job `buzbcevsfmbhmruz` on
   `control.inf9.tinfoil.sh`. It resolved the pinned revision unchanged and
   produced root hash
   `9dd15749a2f9c554cefb41b9bb202c2994d64519b4efbd42af68b51e010d5e21`.

3. Produce two measured satellite releases from the same pins: DSpark on and
   DSpark off. Verify their `tinfoil-deployment.json` and `tinfoil.hash`
   artifacts. The only intended difference between them is speculative
   decoding.

   Prep completed and decoded both measurement artifacts back to YAML, then
   compared them structurally with the checked candidates:

   | Candidate | Measured tag | Commit | Tinfoil config hash |
   | --- | --- | --- | --- |
   | DSpark off | `v2026-08-03-deepseek-v4-flash-0731-dspark-off-1` | `50cd35c38c0248c426ba71379bdcae8b2818ae61` | `3351fea60f7d1276d3e6c8b3192ab38ae4d6cee07db6995fcd504b922ccafa5e` |
   | DSpark on | `v2026-08-03-deepseek-v4-flash-0731-dspark-on-1` | `104364589a7555d6eb505d9c12b490661e582182` | `6fdcc5841d394bf2979df6c0b0a0c2c39b792a2b2f2a1bd2df59b60d65d750c8` |

   The commits differ only by the two-line DSpark speculative configuration.
4. Capture a fresh read-only production inventory. At research time the
   known-good release was
   `v2026-07-02-glm-5-2-limiter-routing-1`; the value observed immediately
   before the window is the rollback authority.
5. Verify 8× H200 GPUs, 512 GiB host memory, TDX/CVM compatibility, model
   storage capacity, and sufficient time to fetch the pinned checkpoint before
   starting downtime.
6. Confirm the topology and secret names remain unchanged:
   `FINITE_USAGE_API_SERVICE_KEY`, `VLLM_INTERNAL_API_KEY`, and
   `VLLM_API_KEY`. Verify equality of the vLLM-facing key values only through
   the secret-management surface; never print them.
7. Confirm the limiter still validates through Core and remains the only
   public inference path. Do not bypass it for testing or recovery.
8. Prepare a dedicated synthetic canary grant with enough quota for the full
   sweep, and confirm read-only access to its reservation and settlement rows.
9. Run `just finite-private-load-contract`. The guarded checked-in load driver
   must support exactly `1,4,8,16,32,64,128,256` and report p50/p95/p99 TTFB,
   completion latency, per-request and aggregate tokens/second, status codes,
   and timeouts.
10. Capture a comparable GLM baseline with the same prompt/output bounds. Keep
   it short and record the workload seed/configuration so the DeepSeek result
   is directly comparable.

    Prep recorded the valid GLM baseline with the checked load driver and its
    fixed 64-token streaming prompt:

    | Concurrency | Aggregate generation tok/s | p50 TTFB | p95 TTFB | p99 TTFB | p50 completion | p95 completion |
    | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
    | 1 | 55.521 | 0.154s | 0.154s | 0.154s | 1.133s | 1.133s |
    | 32 | 320.787 | 2.121s | 5.157s | 5.161s | 3.923s | 6.326s |

    Core recorded all 35 temporary-key preparation requests as `settled`, with
    zero remaining non-settled reservations. The extra requests include the
    functional canary and a discarded timing sample; only the two rows above
    are comparison baselines.
11. Announce one hour as the success-path interruption and at least 90 minutes
    as the incident ceiling. A GLM rollback can itself require about 35 minutes
    of model loading.

Do not start downtime if either measured DeepSeek tag, the measured GLM
rollback tag, an immutable pin, the sweep driver, the canary grant, or Core
accounting visibility is missing.

### Canary grant preparation

Prepare this before the maintenance window. Use the Finite dashboard admin
surface when possible: issue a dedicated synthetic friend key with the
`finite-private-generous-v2` limit profile. The CLI
`finite-private-friend-key-issue` is break-glass only. Do not reuse a customer
key, scope the canary to a customer project/runtime, paste the raw key into a
terminal transcript, or commit it. Core returns the raw key once and persists
only its hash.

Store the one-time value locally as
`secrets/finite-private-canary.env` (or point
`FINITE_PRIVATE_CANARY_ENV_FILE` at an equivalent mode-0600 file) containing
only:

```text
FINITE_PRIVATE_CANARY_API_KEY=<one-time synthetic key>
```

Then verify, without printing the credential:

```bash
python3 -c 'import pathlib, stat; assert stat.S_IMODE(pathlib.Path("secrets/finite-private-canary.env").stat().st_mode) == 0o600'
infra/runbooks/finite-private-ops.sh canary
infra/runbooks/finite-private-ops.sh load-canary 1
```

Confirm the two calls created distinct Core reservations, both settled, and
no synthetic reservation remains `reserved`. Confirm the grant has the exact
`finite-private-generous-v2` profile and enough remaining burst headroom for
the 1–256 sweep plus its post-sweep 1- and 32-way recovery checks. If the key,
grant, quota, file permissions, or accounting visibility is missing, the
window does not start.

## Maintenance-window sequence

### 1. Freeze and record — minute 0

Announce the maintenance window and stop initiating synthetic work. There is
no separate checked-in Finite Private traffic-drain or maintenance-page switch;
the approved Tinfoil relaunch itself creates the accepted interruption. Do not
invent a DNS, Caddy, Core, or limiter mutation during this window. Preserve the
current deployment status, measured tag/hash, health, limiter metrics, GPU
state, and settled Core canary before-state. Run the read-only gate:

```bash
infra/runbooks/finite-private-ops.sh gate
```

Do not continue if the before-state is already unhealthy or ambiguous.

### 2. Launch DeepSeek with DSpark — minutes 0–20

With fresh explicit approval, relaunch only the exact measured DSpark-on tag:

```bash
export FINITE_PRIVATE_RELAUNCH_APPROVED='<approved-deepseek-dspark-tag>'
infra/runbooks/finite-private-ops.sh relaunch '<approved-deepseek-dspark-tag>'
infra/runbooks/finite-private-ops.sh wait-ready
```

Minute 20 is the hard readiness decision point. If the model is not deeply
ready, workers crash/OOM, an image or model pin differs, or the limiter cannot
reach both vLLM and Core, begin GLM rollback. This protects enough of the
announced incident window for the known-good model to load.

Before functional smoke, preserve the model-container startup line that names
the vLLM version and require `0.26.0`. Cross-check it against the CI image
proof and measured image digest. A missing or different runtime version is a
failed preflight, even if `/health` is green.

### 3. Functional smoke — minutes 20–30

Set `FINITE_PRIVATE_MODEL` to the new canonical served name for direct tests,
then test the retained `glm-5-2` alias separately. Preserve all output.

```bash
infra/runbooks/finite-private-ops.sh gate
infra/runbooks/finite-private-ops.sh stream-canary
infra/runbooks/finite-private-ops.sh responses-canary
infra/runbooks/finite-private-ops.sh repeated-id-canary
```

Also run a small fixed prompt set covering normal chat, streamed chat,
reasoning, JSON/tool selection and tool-result continuation, plus one real
Hermes conversation through the normal client path. Confirm invalid Finite
keys return 401 before inference, every successful request has a distinct Core
reservation, streaming settles after `[DONE]`, token accounting is plausible,
and nothing remains `reserved`.

Any protocol, auth, accounting, tool-parser, or Hermes failure is a rollback
condition. DSpark's theoretical losslessness is not a substitute for these
service-level checks.

### 4. Progressive concurrency and speed sweep — minutes 30–45

Use the same short prompt set, deterministic sampling where supported, bounded
output, per-request timeout, and dedicated canary grant at every tier:

```bash
export FINITE_PRIVATE_LOAD_SWEEP_APPROVED='1,4,8,16,32,64,128,256'
infra/runbooks/finite-private-ops.sh load-sweep
```

| Tier | Purpose | Advance rule |
| ---: | --- | --- |
| 1, 4, 8, 16, 32 | Replacement proof for present load | Zero HTTP failures, p99 TTFB below 90 seconds, correct settlement, healthy service |
| 64 | First limit probe | Advance only if service and accounting remain healthy after drain |
| 128 | High limit probe | Run only if 64 recovered cleanly |
| 256 | Maximum planned probe | Run only if 128 recovered cleanly; never repeat during this window |

At every tier record p50/p95/p99 TTFB, p50/p95 completion latency, generated
tokens/second per request and in aggregate, status/error counts, GPU memory and
utilization, scheduler queue depth, and DSpark accepted-token efficiency.

For the 64/128/256 probes, stop the sweep at the first tier that produces any
non-2xx response or request timeout, p99 TTFB at or above 90 seconds, a worker
restart/OOM, a stuck reservation, unhealthy limiter/vLLM/Core status, or a
failure to drain and return healthy within two minutes. Record the previous
tier as the last clean observed tier. A cleanly recovered saturation boundary
does not fail the deployment; corruption, stuck accounting, crash loops, or
failure to recover does.

After the highest attempted tier, always re-run one single-request canary, the
32-way replacement gate, health, and Core settlement checks. Do not judge the
model only by the peak tier.

### 5. Decide and observe — minutes 45–60

Compare DeepSeek with the recorded GLM baseline using the same workload:

| Result | Decision |
| --- | --- |
| Functional/accounting failure, current-load regression, or unhealthy recovery | Roll back |
| Healthy and ≥2× aggregate throughput | Performance target met; continue observation |
| Healthy and ≥3× aggregate throughput | Upside target met; continue observation |
| Healthy, no current-load regression, but <2× | Do not call it a performance win; operator chooses keep or rollback |
| Only an exploratory 64/128/256 tier saturates cleanly | Keep the last clean tier as evidence; no automatic rollback |

End maintenance only after a final real-client request streams successfully,
its Core reservation settles, health remains deep-ready, and the exact running
measured tag/hash is recorded. Continue watching errors, latency, queue depth,
GPU health, and reservation age after traffic returns.

## DSpark-off fallback

Use the prepared measured DSpark-off release only when DeepSeek is otherwise
healthy but DSpark itself produces a reproducible compatibility, stability, or
performance problem. This is a second model relaunch and may exceed the
one-hour target. Re-run the full functional smoke and at least concurrency 1,
8, and 32; do not infer DSpark-off safety from the DSpark-on result.

If the base DeepSeek runtime, model load, protocol surface, limiter, or
accounting is unhealthy, skip this fallback and return directly to GLM.

## GLM rollback

Rollback is one guarded relaunch to the exact prior known-good measured tag
captured at minute 0:

```bash
export FINITE_PRIVATE_RELAUNCH_APPROVED='<prior-known-good-measured-glm-tag>'
infra/runbooks/finite-private-ops.sh relaunch '<prior-known-good-measured-glm-tag>'
infra/runbooks/finite-private-ops.sh wait-ready
infra/runbooks/finite-private-ops.sh gate
```

After rollback, verify a fresh real-client request and Core settlement. Inspect
all DeepSeek-era canary reservations before closing the incident and prefer
user-favorable correction for any stale estimates. Do not bypass the limiter
to restore service unless a separate break-glass decision explicitly accepts
unmetered access.

## Change record

Preserve the following outside public git where it could expose operational or
secret data: operator, start/end times, prior and target measured tags/hashes,
source/model/image pins, startup duration, smoke results, each concurrency-tier
report, GLM comparison, last clean tier, final decision, and any rollback or
stale-reservation follow-up. Secret values and authenticated response bodies
do not belong in this repository.
