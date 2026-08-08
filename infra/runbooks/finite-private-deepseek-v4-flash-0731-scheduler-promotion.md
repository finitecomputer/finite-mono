# Finite Private: DeepSeek V4 scheduler promotion

Status: prepared handoff only. No release or production mutation is authorized
by this file.

This runbook promotes the measured DeepSeek V4 Flash 0731 scheduler winner on
the existing eight-H200 production service. It is not a GLM-to-DeepSeek
cutover and does not change model identity, weights, numerical formats,
parallel topology, limiter, secrets, public routes, or product defaults.

## Fixed identities

| Role | Immutable identity |
| --- | --- |
| Current production / immediate rollback | `v2026-08-05-deepseek-v4-flash-0731-retry-2-3` |
| Production host | `control.inf9.tinfoil.sh` |
| Production container | `kimi-k2-6` |
| Checkpoint | `deepseek-ai/DeepSeek-V4-Flash-0731@7872f01b1d1fe23eabc4c98b48bffcef5a386062` |
| Model MPK root | `9dd15749a2f9c554cefb41b9bb202c2994d64519b4efbd42af68b51e010d5e21` |
| Runtime image | `ghcr.io/finitecomputer/deepseek-v4-vllm:0.25.1-0731-reasoning.6@sha256:48716fa9c25605ab5fe00fd7eed4e792268aee6c9008616f7641d9bf622ff262` |
| Candidate source branch | `ops/deepseek-v4-h200-20260807` at `839d0f36e1a7c376be2671770af60c413f39520d` |
| Candidate config SHA-256 | `bc5a7606811e164a04c286b6bc6d59f46dda57adfd20bd46ac06fe340bf8a204` |

The candidate differs from the current production config only as follows:

```diff
---max-num-seqs 64
---max-num-batched-tokens 512
+--max-num-seqs 128
+--max-num-batched-tokens 2048
```

The source of truth is
[`tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml`](../tinfoil/confidential-kimi-k2-6/tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml).
Do not reconstruct it manually from this abbreviated diff.

## PRECONDITIONS

1. Run `scripts/finite-status` and retain its output. Any red result stops the
   rollout. An unknown result must be resolved from the correct profiled host;
   a Mac's unprofiled `UNKNOWN` is not production proof.
2. Confirm Tinfoil reports `kimi-k2-6` ready on `control.inf9.tinfoil.sh` at
   the exact rollback tag above.
3. With the root-only canary environment at
   `secrets/finite-private-canary.env`, run:

   ```bash
   infra/runbooks/finite-private-ops.sh gate
   infra/runbooks/finite-private-ops.sh stream-canary
   infra/runbooks/finite-private-ops.sh responses-canary
   infra/runbooks/finite-private-ops.sh repeated-id-canary
   infra/runbooks/finite-private-ops.sh quality-canary
   ```

4. Confirm every successful canary has a distinct Core reservation, every
   reservation settles, and no synthetic reservation remains `reserved`.
5. Run both repository contracts:

   ```bash
   python3 scripts/check_finite_private_deepseek_candidate.py
   python3 scripts/check_finite_private_deepseek_candidate.py --release-ready
   ```

6. Confirm no checkpoint, MPK, image, limiter, secret name, route, parser,
   context, or numerical-format diff accompanies the scheduler change.
7. Confirm an operator has approved the maintenance window, accepted the
   interruption, and named the exact new measured release tag. Preparation or
   a passing test does not supply that authority.

## PREPARE THE MEASURED RELEASE

This phase may run only with separate authorization to publish a satellite
release. Copy the candidate into the satellite repo without changing its
structure, commit it, and publish one new immutable Tinfoil release.

Before production relaunch:

1. Download the new release's `tinfoil-deployment.json` and `tinfoil.hash`.
2. Verify the deployment artifact digest and decode its embedded config.
3. Compare the decoded structure with the candidate; comments may differ, but
   executable content may not.
4. Record the satellite commit, release tag, Tinfoil hash, candidate config
   hash, and runtime image digest.
5. Verify the only executable difference from rollback tag
   `v2026-08-05-deepseek-v4-flash-0731-retry-2-3` is 64/512 to 128/2048.

Any floating tag, rebuilt image, different MPK, unmeasured flag, or broader
diff is a stop condition.

## STEPS

Set `TARGET_TAG` to the newly measured tag only after the evidence above is
recorded. The relaunch helper refuses a tag that is not repeated in the
explicit approval variable.

```bash
export TARGET_TAG='REPLACE_WITH_EXACT_MEASURED_TAG'
export FINITE_PRIVATE_RELAUNCH_APPROVED="$TARGET_TAG"
infra/runbooks/finite-private-ops.sh relaunch "$TARGET_TAG"
infra/runbooks/finite-private-ops.sh wait-ready
```

After readiness:

1. Confirm the running tag equals `TARGET_TAG` and the host remains
   `control.inf9.tinfoil.sh` with eight H200s.
2. Preserve the model startup identity and parsed arguments. Require vLLM
   0.25.1, DP8+EP, FP8 KV, 393,216 context, speculation off, and exactly
   `max_num_seqs=128` / `max_num_batched_tokens=2048`.
3. Run the full precondition canary set again, including quality at high and
   max reasoning. Confirm auth rejection, tool/reasoning structure, streaming
   `[DONE]`, Responses API identity, and Core settlement.
4. Warm the scheduler progressively through the checked load harness:

   ```bash
   export FINITE_PRIVATE_LOAD_SWEEP_APPROVED='1,4,8,16,32,64,128,256'
   infra/runbooks/finite-private-ops.sh load-sweep
   ```

5. After the sweep, require a clean single-request canary, clean 32-way load
   canary, deep health, no stuck reservations, and no worker restart, OOM,
   CUDA error, traceback, or limiter error.
6. Observe the exact target for at least 35 minutes. Repeat deep health,
   authenticated canary, settlement inspection, and error-log scan at the end.
7. Run `scripts/finite-status` again and retain the output together with the
   exact running tag and hash.

## VERIFY

The promotion passes only when all of the following are true:

- exact target tag, image, checkpoint, MPK, and scheduler arguments match;
- process liveness and deep readiness remain healthy;
- normal, streaming, Responses, reasoning, and tool-call protocols pass;
- invalid keys fail before inference;
- every canary reservation settles with plausible token accounting;
- the guarded load sweep recovers cleanly after every attempted tier;
- the 35-minute observation has no crash, OOM, corruption, or error-log match;
- single-user behavior is not materially worse than the measured 54.78 tok/s,
  0.214-second-TTFT lab result under a comparable request; and
- the before/after status evidence contains no red platform result.

The 8,373 aggregate tok/s result is a lab reference, not a production pass
threshold. Production traffic shape, network path, limiter, and accounting are
different; correctness and clean recovery remain authoritative.

## ROLLBACK

Rollback immediately on identity drift, corrupt output, protocol/auth/
accounting failure, worker restart/OOM, deep-health failure, failure to recover
within two minutes after a load tier, or a material current-load regression.

The immediate rollback is the exact currently running DeepSeek baseline:

```bash
export ROLLBACK_TAG='v2026-08-05-deepseek-v4-flash-0731-retry-2-3'
export FINITE_PRIVATE_RELAUNCH_APPROVED="$ROLLBACK_TAG"
infra/runbooks/finite-private-ops.sh relaunch "$ROLLBACK_TAG"
infra/runbooks/finite-private-ops.sh wait-ready
infra/runbooks/finite-private-ops.sh gate
scripts/finite-status
```

After rollback, confirm the parsed scheduler values returned to 64/512 and
repeat settlement inspection. Do not improvise a third configuration during
the window. If the exact baseline cannot recover, stop and use the separately
reviewed historical GLM recovery boundary from the retry-2 runbook; that is an
incident escalation, not the normal rollback for this scheduler promotion.
