# Finite Private: DeepSeek-V4-Flash-0731 retry 2

Status: preparation only. Production remains on measured GLM 5.2 release
`v2026-07-02-glm-5-2-limiter-routing-1`. Nothing in this runbook authorizes an
image publication, satellite release, Tinfoil relaunch, or product-default
change.

## Why this is a new attempt

The first attempt proved that Finite wrapped the exact official checkpoint and
that target-only generation could serve the existing protocol. It also exposed
three independent problems:

1. vLLM 0.26.0 mapped the new 0731 reasoning levels incorrectly. Hermes asked
   for `high`, but the rendered prompt was effectively 0731 `low`; v0.26 `max`
   rendered only the 0731 `high` prompt.
2. The official vLLM recipe pinned 0731 back to the 0.25 line after a reported
   v0.26/nightly crash at roughly 30 minutes. Finite's short functional and load
   checks did not provide that soak duration.
3. The one-engine TP8+EP shape matched GLM instead of producing the expected
   throughput improvement. Tinfoil's published DeepSeek V4 Pro deployment and
   the public H200 benchmark recipe use data-parallel attention plus expert
   parallelism for the throughput lane.

DSpark was a separate correctness failure: approximately 1% acceptance and
corrupt output on the attempted v0.26/H200 shape. It is out of scope for retry
2. Target-only output remains authoritative.

Supporting evidence is recorded in
[`docs/research/2026-08-04-deepseek-v4-flash-0731-identity-and-h200-recipes.md`](../../docs/research/2026-08-04-deepseek-v4-flash-0731-identity-and-h200-recipes.md).

## Fixed retry-2 candidate

| Area | Retry-2 decision |
| --- | --- |
| Checkpoint | Keep `deepseek-ai/DeepSeek-V4-Flash-0731@7872f01b1d1fe23eabc4c98b48bffcef5a386062` |
| Tinfoil MPK | Keep root `9dd15749a2f9c554cefb41b9bb202c2994d64519b4efbd42af68b51e010d5e21`; do not re-wrap |
| Weights | Keep the official native FP4-expert/FP8-other artifact unchanged |
| KV cache | FP8 |
| Runtime base | Official linux/amd64 vLLM 0.25.1 image at `sha256:f0b9a0dc75a9fca3b6811e3279367b2d6a448055a000bfd13859587d74cef268` |
| Runtime patch | Only upstream vLLM commit `77434861904a9f01ea4818fe9f0c7b2a5c05686e`, backported with exact pre/post source hashes |
| Parallelism | DP8+EP, following Tinfoil's published eight-GPU DeepSeek V4 topology; no tensor parallelism |
| Speculation | Off; no DSpark or MTP |
| Scheduler ceiling | `max-num-seqs=64`, `max-num-batched-tokens=512` |
| Context | 393,216 service tokens |
| Sampling for quality proof | `temperature=1.0`, `top_p=0.95`, explicit thinking high/max |
| Rollback | Exact measured GLM tag above remains the only rollback authority |

The staged config is
[`tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml`](../tinfoil/confidential-kimi-k2-6/tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml).
Its image field deliberately remains a placeholder until the manual image
workflow reports a digest.

## Preparation gates

### 1. Local repository proof

Run:

```bash
python3 -m unittest \
  scripts.tests.test_patch_vllm_deepseek_v4_0731 \
  scripts.tests.test_check_deepseek_v4_0731_quality \
  scripts.tests.test_finite_private_deepseek_candidate \
  scripts.tests.test_finite_private_ops
python3 scripts/check_finite_private_deepseek_candidate.py
```

The prep contract must pass and the release-ready contract must fail only on
the intentional unpublished-image placeholder.

### 2. Publish the measured runtime image — separately authorized

After review, dispatch `.github/workflows/deepseek-v4-vllm-image.yml` with a
version such as `0.25.1-0731-reasoning.1`. The workflow must:

- build only from the pinned official vLLM child digest;
- reject a changed v0.25.1 Python source before patching;
- apply the upstream 0731 prompt mapping and verify the resulting source hashes;
- report vLLM package version `0.25.1`;
- label the exact mono revision and upstream fix;
- publish and report one immutable GHCR digest.

Do not use a floating vLLM nightly, `latest`, an unmeasured local image, or the
old v0.26.0 image.

### 3. Pin and measure the satellite config — separately authorized

Replace only `REPLACE_WITH_MEASURED_DEEPSEEK_V4_VLLM_IMAGE` with the workflow's
exact `ghcr.io/finitecomputer/deepseek-v4-vllm:...@sha256:...` output. Then run:

```bash
python3 scripts/check_finite_private_deepseek_candidate.py --release-ready
```

Create a satellite release only after that gate passes. Decode its
`tinfoil-deployment.json`, compare it structurally with the candidate, and
record the release tag, satellite commit, image digest, and Tinfoil config
hash. This measurement still does not authorize a relaunch.

### 4. Baselines before downtime

Preserve all of the following:

- read-only Tinfoil status naming the exact ready GLM tag;
- GLM deep health, one real-client turn, and one settled canary;
- the existing GLM concurrency-1 and concurrency-32 numbers;
- a short scored hosted-DeepSeek reference report if a temporary
  `DEEPSEEK_API_KEY` is available.

Prep snapshot captured at `2026-08-05T04:05:04Z` without mutation:

- Tinfoil reported `ready` on eight H200s at exact tag
  `v2026-07-02-glm-5-2-limiter-routing-1`;
- `/live` and deep `/health` both returned HTTP 200, including healthy GLM and
  Core usage-API components;
- the temporary quota-backed key completed the exact authenticated canary as
  model `glm-5-2`; and
- its response fingerprint was `vllm-0.23.0-tp8-8d3efe69`.

The DeepSeek compatibility-image workflow was not dispatched and no satellite
or Tinfoil state was changed during this snapshot.

The hosted reference uses the same checked scorer without exposing the key:

```bash
python3 scripts/check_deepseek_v4_0731_quality.py \
  --endpoint https://api.deepseek.com/v1 \
  --model deepseek-v4-flash \
  --lane deepseek-hosted \
  --api-key-env DEEPSEEK_API_KEY
```

## Maintenance-window gates

These steps begin only after the user explicitly approves the measured retry-2
release and accepts downtime.

### 1. Relaunch and prove identity

Relaunch only the recorded retry-2 tag. Require deep readiness, the exact image
digest, vLLM `0.25.1`, the upstream-fix label, DP8+EP startup lines, eight H200s,
and the expected model/MPK identifiers. Any mismatch starts GLM rollback.

### 2. Protocol and reasoning correctness

Run the ordinary gate plus:

```bash
infra/runbooks/finite-private-ops.sh stream-canary
infra/runbooks/finite-private-ops.sh responses-canary
infra/runbooks/finite-private-ops.sh quality-canary
```

The scored quality canary runs five deterministic cases at both high and max.
All ten must pass, every response must return non-empty parsed
`reasoning_content`, raw DSML must not leak into final content, and the tool case
must produce the expected parsed call. Compare against the hosted report using
the same prompts and sampling; do not compare retry output to the old
temperature-zero throughput prompt.

Also complete one real Hermes tool-use turn through the normal Finite route.
Confirm Hermes `high` now renders the actual 0731 high prefix and that an
explicit `max` request renders the new beyond-maximum prefix.

### 3. Minimum stability soak

Keep the service continuously exercised for at least 35 minutes before calling
the runtime stable. Alternate short high/max chat, tool selection, and bounded
longer-context requests while watching:

- worker exits/restarts and CUDA assertions;
- top-k/sparse-prefill failures;
- queue depth and GPU memory;
- malformed DSML or missing reasoning;
- Core reservations older than the request timeout.

The 35-minute minimum is specifically intended to cross the reported v0.26
failure horizon. A clean two-minute smoke is not sufficient evidence.

### 4. Concurrency and recovery

After correctness and soak pass, run `1,8,16,32,64` using the existing bounded
driver. Require zero HTTP failures, complete settlement, healthy drain, and a
successful single request after every tier. Record aggregate generation rate,
TTFB percentiles, completion percentiles, scheduler queue depth, and GPU state.

The exploratory 128 and 256 tiers remain optional limit probes. Run them only
after 64 drains cleanly, stop at the first failure, and never advertise them as
capacity from a short success.

### 5. Production decision

Keep retry 2 only if all of the following are true:

1. checkpoint/image/config identity is exact;
2. high/max reasoning and tool parsing pass 10/10 plus the real Hermes turn;
3. the 35-minute soak has no crash, corruption, or stuck settlement;
4. concurrency 32 is at least as healthy as GLM and 64 recovers cleanly;
5. output quality is comparable to the hosted 0731 reference on the fixed set;
6. the operator explicitly accepts the measured throughput result.

Only after that decision should the separate Finite Runtime/default-model
changes be merged or deployed so `/new` identifies DeepSeek. Until then GLM is
the product and rollback authority.

## Rollback

Rollback is a guarded relaunch to exactly
`v2026-07-02-glm-5-2-limiter-routing-1`, followed by status, live/deep health,
invalid-key rejection, chat, streaming, Responses, a real Hermes turn, the
32-way bounded check, and Core settlement verification. Do not change DNS,
secrets, limiter topology, or persisted chat state during rollback.
