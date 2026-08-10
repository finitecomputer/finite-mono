# Finite Private: DeepSeek production update

Status: preparation only. This file does not authorize a satellite release,
Tinfoil relaunch, NixOS deployment, Runtime rollout, container replacement, or
DNS change.

Production already serves DeepSeek V4 Flash 0731. This update promotes the
scheduler configuration measured on the isolated eight-H200 rack and makes
DeepSeek the canonical fallback label throughout the serving path. It is not a
GLM-to-DeepSeek model cutover.

## Fixed current state

| Role | Identity |
| --- | --- |
| Production host | `control.inf9.tinfoil.sh` |
| Production container | `kimi-k2-6` (historical infrastructure name) |
| Immediate rollback tag | `v2026-08-05-deepseek-v4-flash-0731-retry-2-3` |
| Checkpoint | `deepseek-ai/DeepSeek-V4-Flash-0731@7872f01b1d1fe23eabc4c98b48bffcef5a386062` |
| Runtime image | `ghcr.io/finitecomputer/deepseek-v4-vllm:0.25.1-0731-reasoning.6@sha256:48716fa9c25605ab5fe00fd7eed4e792268aee6c9008616f7641d9bf622ff262` |
| Parallelism and cache | DP8+EP, FP8 KV cache |
| Context ceiling | 393,216 tokens |
| Current scheduler | 64 sequences / 512 batched tokens |
| Candidate scheduler | 128 sequences / 2,048 batched tokens |
| Canonical model | `deepseek-v4-flash-0731` |
| Compatibility alias | `glm-5-2` |

The candidate source is
[`tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml`](../tinfoil/confidential-kimi-k2-6/tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml).
The exact lab measurements, rejected shapes, protocol proof, near-limit context
proof, and soak are in
[`2026-08-07-deepseek-v4-eight-h200-optimization.md`](../../docs/research/2026-08-07-deepseek-v4-eight-h200-optimization.md).

The candidate intentionally changes only:

1. `max-num-seqs` from 64 to 128;
2. `max-num-batched-tokens` from 512 to 2,048; and
3. the limiter fallback label from `glm-5-2` to
   `deepseek-v4-flash-0731`.

The third change affects health/accounting fallback records only when a request
or response omits its model. vLLM continues to serve both names, so an older
Runtime that explicitly sends `glm-5-2` remains compatible.

## PRECONDITIONS

1. Run `scripts/finite-status --json` from the correctly profiled production
   host and retain the output. Any red or unresolved unknown result stops the
   rollout.
2. Confirm Tinfoil reports the production container ready on the fixed host at
   the exact rollback tag above, with eight H200s and the expected three secret
   names. Never print secret values.
3. Run the current production gates through the real limiter/accounting path:

   ```bash
   infra/runbooks/finite-private-ops.sh gate
   infra/runbooks/finite-private-ops.sh stream-canary
   infra/runbooks/finite-private-ops.sh responses-canary
   ```

4. Capture three one-way and three 32-way baselines and retain their medians.
   Confirm all reservations settle and no canary reservation remains reserved.
5. Run both repository contracts:

   ```bash
   just finite-private-deepseek-contract
   just finite-private-deepseek-release-contract
   ```

6. Diff the decoded candidate against the rollback deployment. Any checkpoint,
   MPK, runtime image, limiter image, secret, route, parser, context, numerical
   format, or parallelism change is a stop condition.
7. Record an exact new satellite commit, release tag, deployment artifact,
   Tinfoil hash, and candidate SHA-256 in `compat/matrix.toml` in the same
   reviewed promotion change.
8. On an isolated evaluation target running the exact release candidate, run
   the fixed scored corpus in `scripts/check_deepseek_v4_0731_quality.py`
   against both lanes. The hosted reference must be DeepSeek's hosted
   V4-Flash-0731 service; record its endpoint hostname, advertised model, and
   returned model/fingerprint fields with the reports. Keep both JSON reports
   under `.local-state/deepseek-quality/$TARGET_TAG/` and require every case at
   both `high` and `max` effort to pass:

   ```bash
   export TARGET_TAG='REPLACE_WITH_EXACT_MEASURED_TAG'
   export CANDIDATE_ENDPOINT='REPLACE_WITH_ISOLATED_CANDIDATE_BASE_URL'
   export DEEPSEEK_HOSTED_ENDPOINT='REPLACE_WITH_HOSTED_REFERENCE_BASE_URL'
   export DEEPSEEK_HOSTED_MODEL='REPLACE_WITH_HOSTED_REFERENCE_MODEL'
   QUALITY_DIR=".local-state/deepseek-quality/$TARGET_TAG"
   mkdir -p "$QUALITY_DIR"
   chmod 700 "$QUALITY_DIR"

   python3 scripts/check_deepseek_v4_0731_quality.py \
     --endpoint "$CANDIDATE_ENDPOINT/v1" \
     --model deepseek-v4-flash-0731 \
     --lane self-hosted \
     > "$QUALITY_DIR/candidate.json"

   python3 scripts/check_deepseek_v4_0731_quality.py \
     --endpoint "$DEEPSEEK_HOSTED_ENDPOINT/v1" \
     --model "$DEEPSEEK_HOSTED_MODEL" \
     --api-key-env DEEPSEEK_HOSTED_API_KEY \
     --lane deepseek-hosted \
     > "$QUALITY_DIR/hosted-reference.json"
   ```

   The script sends the same version-controlled cases and sampling parameters
   to both lanes, checks deterministic correctness, instruction following,
   parsed reasoning, and tool selection, emits the
   `finite-deepseek-quality-v1` report schema, and never accepts or records raw
   keys. Any failed case or unresolved reference-identity mismatch stops the
   rollout.
9. Obtain explicit approval for the exact measured tag and the eight-GPU
   maintenance interruption. Passing tests is not rollout authority.

## STEPS — TODO

TODO: This exact production promotion has not yet been exercised. During the
approved window, record the release identity, operator, timestamps, every gate
result, and any Tinfoil behavior that differs from this procedure; update the
runbook before a later reuse.

After the exact release has been independently measured and approved:

```bash
export TARGET_TAG='REPLACE_WITH_EXACT_MEASURED_TAG'
export FINITE_PRIVATE_RELAUNCH_APPROVED="$TARGET_TAG"
infra/runbooks/finite-private-ops.sh relaunch "$TARGET_TAG"
infra/runbooks/finite-private-ops.sh wait-ready
```

Then:

1. Confirm the running tag, host, GPU count, checkpoint, MPK, runtime and
   limiter digests, DP8+EP topology, FP8 KV cache, 393,216 context, and exact
   128/2,048 scheduler arguments.
2. Require `/live`, `/health`, invalid-key rejection, ordinary chat,
   streaming, Responses API, high/max reasoning, tool parsing, and Core
   settlement to pass.
3. Before admitting normal traffic, sweep concurrency progressively through 1,
   4, 8, 16, 32, 64, 128, 256, 512, and 1,024 to warm all measured request
   shapes and DP ranks. Stop on the first failure and require a clean single
   request after each successful tier. Never issue a larger tier or recovery
   load after a failed request tier.
4. Repeat the one-way and 32-way baselines three times. Candidate median
   throughput must be at least 90% of the pre-update median and median p95
   completion latency no more than 125% of the pre-update median.
5. Observe the target for at least 35 minutes with no worker restart, OOM,
   CUDA error, corrupt output, stuck settlement, or readiness regression.
6. Run `scripts/finite-status --json` again and retain the result.

## VERIFY

Keep the update only when identity is exact, protocol and reasoning gates pass,
all reservations settle, bounded load drains cleanly, the current-load numeric
bounds pass, and the full observation remains healthy. The isolated result of
8,373 aggregate output tokens/sec at 1,024 concurrent requests is not a
production acceptance threshold.

The NixOS Runner default and existing Agent Runtime rollout are separate from
the Tinfoil scheduler update. New Runtime configuration should identify
`deepseek-v4-flash-0731`; existing exact image-owned GLM defaults are migrated
by the current Runtime image. User-owned custom provider settings are not
rewritten.

The latest already-published Runtime containing that narrow migration is:

```text
ghcr.io/finitecomputer/agent-runtime:2026-08-07.2@sha256:130ba3036991bbca7b99fae9dbd95f91c86018737004d65107791c1924eaa4ad
```

It was built from `main` revision `0903b426` by workflow run `31222177824`.
Publication is not proof of promotion or fleet rollout. Before using it,
confirm its artifact record and current fleet distribution with
`scripts/finite-status`, prove one disposable canary, and use the explicit
prepare/execute Runtime rollout in [`runtime-image.md`](runtime-image.md).
After the NixOS Runner change, `finite-status` must show effective model
`deepseek-v4-flash-0731` while retaining the historical route; a host-local
`/etc/finite/runner.env` GLM override is red and blocks new launches.

The container-name migration is also separate. Follow
[`finite-private-routing-migration.md`](finite-private-routing-migration.md);
never combine a scheduler change with route/DNS replacement.

## ROLLBACK

Rollback immediately on identity drift, protocol/auth/accounting failure,
worker restart/OOM, deep-health failure, failure to drain within two minutes,
or failure of the current-load bounds:

```bash
export ROLLBACK_TAG='v2026-08-05-deepseek-v4-flash-0731-retry-2-3'
export FINITE_PRIVATE_RELAUNCH_APPROVED="$ROLLBACK_TAG"
infra/runbooks/finite-private-ops.sh relaunch "$ROLLBACK_TAG"
infra/runbooks/finite-private-ops.sh wait-ready
infra/runbooks/finite-private-ops.sh gate
infra/runbooks/finite-private-ops.sh stream-canary
infra/runbooks/finite-private-ops.sh responses-canary
infra/runbooks/finite-private-ops.sh load-canary 1
infra/runbooks/finite-private-ops.sh load-canary 32
scripts/finite-status --json
```

Confirm parsed scheduler values returned to 64/512 and every rollback canary
settled. Do not improvise a third configuration during the window.
