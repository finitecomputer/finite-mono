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

This promotion does not rename the Tinfoil container. The current production
identity remains `kimi-k2-6` for mixed-version continuity; the eventual stable
production service name is `finite-private`, handled only by the separate
routing migration.

There is one active eight-H200 cluster. The winning 128/2,048 recipe was
already measured on the temporary `gpu-lab` container at
`control.inf12.tinfoil.sh`; that lab target was deleted after the retained
measurements were captured. Do not require, create, or budget for a second
eight-H200 target during this release. The existing performance evidence is
applicable only while the candidate checksum and the byte-level identity gates
below prove that the checkpoint, images, model settings, topology, context, and
all other serving arguments remain unchanged.

## SATELLITE RELEASE PREPARATION

The satellite's `main` branch is stale and must not be the source of this
release. Current production and the immediate rollback are commit
`e337db3606d67c53387113700362adec7b4dfdf7`, tagged
`v2026-08-05-deepseek-v4-flash-0731-retry-2-3`. Create the release branch from
that exact commit, never from satellite `main`:

```bash
export SATELLITE_REPO='finitecomputer/confidential-kimi-k2-6'
export ROLLBACK_TAG='v2026-08-05-deepseek-v4-flash-0731-retry-2-3'
export ROLLBACK_COMMIT='e337db3606d67c53387113700362adec7b4dfdf7'
export SATELLITE_BRANCH='ops/deepseek-v4-128-2048-REPLACE_DATE'
export TARGET_TAG='REPLACE_WITH_UNIQUE_DEEPSEEK_128_2048_TAG'
export EXPECTED_CANDIDATE_SHA256='22a3b8030aeb2a47dab8547690cf125880f630d3163bcb713534fb43bffa8907'
export FINITE_MONO_CHECKOUT="$(pwd)"
export SATELLITE_CHECKOUT='REPLACE_WITH_CLEAN_SATELLITE_CHECKOUT'

gh repo clone "$SATELLITE_REPO" "$SATELLITE_CHECKOUT"
cd "$SATELLITE_CHECKOUT"
git fetch origin --tags
test "$(git rev-list -n 1 "$ROLLBACK_TAG")" = "$ROLLBACK_COMMIT"
git switch --create "$SATELLITE_BRANCH" "$ROLLBACK_COMMIT"
cp "$FINITE_MONO_CHECKOUT/infra/tinfoil/confidential-kimi-k2-6/tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml" tinfoil-config.yml
```

Review the decoded diff against the rollback tag. Its only semantic changes
must be the 64 to 128 sequence limit, the 512 to 2,048 batched-token limit, and
the missing-model fallback label becoming canonical DeepSeek. Checkpoint, MPK,
both image digests, secrets, route, parser, context, numerical format, and
parallelism must be byte-for-byte unchanged. Record the candidate checksum:

```bash
git diff "$ROLLBACK_TAG" -- tinfoil-config.yml
test "$(sha256sum tinfoil-config.yml | cut -d ' ' -f 1)" = \
  "$EXPECTED_CANDIDATE_SHA256"
```

After review, commit and push that exact satellite file. Dispatch the satellite
workflow on the reviewed branch explicitly; never omit `--ref`:

```bash
export SATELLITE_COMMIT="$(git rev-parse HEAD)"
gh workflow run tinfoil-release.yml \
  --repo "$SATELLITE_REPO" \
  --ref "$SATELLITE_BRANCH" \
  -f version="$TARGET_TAG"
```

Require the release tag to resolve to `$SATELLITE_COMMIT`, both publish jobs to
pass, and the release to contain `tinfoil-deployment.json` and `tinfoil.hash`.
Retain both assets and their SHA-256 values. Publication is preparation, not
authority to relaunch production.

Publishing the satellite release does not create a container. The release
reuses the measured configuration; production remains on its current tag until
the separately approved relaunch window.

## PRECONDITIONS

1. Run the exact merged `scripts/finite-status --json` and companion
   `scripts/finite_status.py` from the correctly profiled production host and
   retain the output with the mono commit SHA. Until the command is installed
   by the normal NixOS deployment, use an exact read-only checkout or install
   those two files together as described in `infra/runbooks/README.md`; do not
   deploy NixOS merely to prepare this scheduler update. This fleet-wide report
   is a before/after non-regression boundary, not authority to couple unrelated
   repairs into the scheduler window. A pre-existing non-causal exception may
   carry through only when it is named below, retained in the before report,
   and independently proved not to intersect the Tinfoil container, limiter,
   Core accounting path, or Runner route. Any new or worsened red or unknown
   result stops the rollout; an exception that disappears is an improvement.

   The only reviewed pre-existing non-causal exceptions for this promotion
   are:

   - `fleet_convergence` red solely because `Sites Canary 0715`
     (`runtime_6633703b0a4d4de545b2`) and `Smoke Studio`
     (`runtime_be7a5a7418409a0d6a29`) are not on the current Runtime artifact;
     they are not part of the Tinfoil scheduler deployment, and the historical
     request label an older Runtime may send is proved by
     `mixed-version-canary`;
   - `recovery_boundary` red solely because the deployed
     `finite-litestream-health.service` measures write recency on a quiet chat
     database. Carry this exception only while the local `litestream_txid` and
     newest replicated LTX transaction ID are equal and the snapshot and Borg
     subchecks remain green.

   Prove that second exception immediately before relaunch from a root shell on
   lat1. This prints transaction positions only, never credential values:

   ```bash
   set -a
   . /etc/finite/litestream-latitude.env
   set +a
   FP_CHAT_DB='/var/lib/private/finite-chat/data/server.sqlite3'
   FP_METRICS="$(curl -fsS --max-time 10 http://127.0.0.1:9351/metrics)"
   FP_DB_TXID="$(printf '%s\n' "$FP_METRICS" | awk -v needle="db=\"$FP_CHAT_DB\"" \
     'index($0, "litestream_txid{") == 1 && index($0, needle) > 0 {printf "%.0f", $2}')"
   FP_REPLICA_HEX="$(litestream ltx -config /etc/litestream.yml "$FP_CHAT_DB" \
     | tail -n +2 | awk '{print $3}' | sort | tail -1)"
   FP_REPLICA_TXID=$(( 16#$FP_REPLICA_HEX ))
   test -n "$FP_DB_TXID"
   test "$FP_DB_TXID" -eq "$FP_REPLICA_TXID"
   printf 'litestream caught up: db_txid=%s replica_txid=%s\n' \
     "$FP_DB_TXID" "$FP_REPLICA_TXID"
   ```

   The continuing Litestream retention-delete `403 AccessDenied` is tracked as
   a separate backup-maintenance problem and is not repaired in this window.
   Do not trigger a Runtime, NixOS, Litestream, storage-policy, or host-storage
   change to make this scheduler-only preflight green. Any different Runtime
   straggler, replication lag, snapshot/Borg regression, host-health problem,
   Runner route/model problem, or other additional finding is a stop
   condition. Before the canonical Runner role is deployed,
   `mixed-version-compatibility` is an expected green state. After that role is
   deployed, an operator file forcing the historical alias is
   `stale-operator-override` and red.

   If the command is not yet installed, the minimal staging path during the
   separately authorized maintenance window is:

   ```bash
   export MONO_SHA="$(git rev-parse HEAD)"
   export STATUS_DIR="/root/finite-status-$MONO_SHA"
   export NIXPKGS_REV='b6018f87da91d19d0ab4cf979885689b469cdd41'
   ssh root@64.34.82.77 "install -d -m 0700 '$STATUS_DIR'"
   scp scripts/finite-status scripts/finite_status.py \
     "root@64.34.82.77:$STATUS_DIR/"
   ssh root@64.34.82.77 "chmod 0500 '$STATUS_DIR/finite-status' && cd '$STATUS_DIR' && nix --extra-experimental-features 'nix-command flakes' shell 'github:NixOS/nixpkgs/$NIXPKGS_REV#python3' --command python3 ./finite-status --json"
   ```

   This installs only the canonical read-only collector files and does not
   restart or deploy a service. The production host does not otherwise include
   Python, so the command uses the exact Nixpkgs revision pinned by this mono
   checkout rather than an unpinned interpreter. Retain the exact directory and
   output for the matching post-rollout observation.
2. Confirm Tinfoil reports the production container ready on the fixed host at
   the exact rollback tag above, with eight H200s and the expected three secret
   names. Never print secret values.
3. Run the current production gates through the real limiter/accounting path:

   ```bash
   infra/runbooks/finite-private-ops.sh gate
   infra/runbooks/finite-private-ops.sh stream-canary
   infra/runbooks/finite-private-ops.sh responses-canary
   infra/runbooks/finite-private-ops.sh mixed-version-canary
   ```

4. Capture three one-way and three 32-way baselines and retain their medians.
   Confirm all canonical and mixed-version canary reservations settle and no
   canary reservation remains reserved.
5. Run both repository contracts:

   ```bash
   just finite-private-deepseek-contract
   just finite-private-deepseek-release-contract
   ```

6. Diff the decoded candidate against the rollback deployment. Any checkpoint,
   MPK, runtime image, limiter image, secret, route, parser, context, numerical
   format, or parallelism change is a stop condition.
7. Open a small reviewed finite-mono promotion change that records the exact
   new satellite commit, release tag, `tinfoil-deployment.json` SHA-256,
   `tinfoil.hash` SHA-256, and candidate-config SHA-256 in
   `compat/matrix.toml`. That evidence cannot be filled in before publication;
   the follow-up must merge before production approval.
8. Run the fixed scored corpus in
   `scripts/check_deepseek_v4_0731_quality.py` against the current live
   DeepSeek service before the window. Use the same approved canary credential
   as the protocol and accounting gates. This is the correct pre-update quality
   baseline because the candidate changes scheduler admission and the
   missing-model accounting label only; the checkpoint, runtime image, parser,
   sampling configuration, and model topology are identical. The exact
   128/2,048 recipe already has retained isolated performance and protocol
   evidence, so this scheduler-only promotion does not require a second hosted
   DeepSeek credential or a new model-quality comparison. Keep the JSON report
   under `.local-state/deepseek-quality/$TARGET_TAG/` and require every case at
   both `high` and `max` effort to pass:

   ```bash
   export TARGET_TAG='REPLACE_WITH_EXACT_MEASURED_TAG'
   export CURRENT_ENDPOINT="${FINITE_PRIVATE_ENDPOINT:-https://kimi-k2-6.finite.containers.tinfoil.dev}"
   QUALITY_DIR=".local-state/deepseek-quality/$TARGET_TAG"
   mkdir -p "$QUALITY_DIR"
   chmod 700 "$QUALITY_DIR"

   python3 scripts/check_deepseek_v4_0731_quality.py \
     --endpoint "$CURRENT_ENDPOINT/v1" \
     --model deepseek-v4-flash-0731 \
     --lane self-hosted \
     > "$QUALITY_DIR/production-before.json"
   ```

   The script checks deterministic correctness, instruction following, parsed
   reasoning, and tool selection, emits the `finite-deepseek-quality-v1` report
   schema, and never records the canary key. Any failed case or returned-model
   mismatch stops the rollout. Repeat the same self-hosted command after
   relaunch as required below.
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
   settlement to pass. Run `mixed-version-canary` as an existing-Runtime edge;
   DeepSeek remains the canonical label even while that request alias works.
   Repeat the self-hosted quality command from precondition 8 against the
   relaunched production target and retain it as `production-after.json`; every
   case at both `high` and `max` effort must pass before the rollout is declared
   successful.
3. As soon as readiness returns, monitor live traffic and sweep concurrency
   progressively through 1, 4, 8, 16, 32, 64, 128, 256, 512, and 1,024 to warm
   all measured request shapes and DP ranks. Stop on the first failure and
   require a clean single request after each successful tier. Never issue a
   larger tier or recovery load after a failed request tier.
4. Repeat the one-way and 32-way baselines three times. Candidate median
   throughput must be at least 90% of the pre-update median and median p95
   completion latency no more than 125% of the pre-update median.
5. Observe the target for at least 35 minutes with no worker restart, OOM,
   CUDA error, corrupt output, stuck settlement, or readiness regression.
6. Run `scripts/finite-status --json` again and retain the result. Compare it
   with the exact before report: the two reviewed pre-existing non-causal
   exceptions may be identical or improved, but any new or worsened red or
   unknown result is a rollback condition.

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

The current fleet-roll target, which also contains that narrow migration, is:

```text
ghcr.io/finitecomputer/agent-runtime:2026-08-11.1@sha256:c48da1985c9bbd0a820240d6224c20864f2e9950ac668238466ed38d733d866d
```

It was built from `main` revision `8404f98d` by successful workflow run
`31525093333`.
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
infra/runbooks/finite-private-ops.sh mixed-version-canary
infra/runbooks/finite-private-ops.sh load-canary 1
infra/runbooks/finite-private-ops.sh load-canary 32
scripts/finite-status --json
```

Confirm parsed scheduler values returned to 64/512 and every rollback canary
settled. Do not improvise a third configuration during the window.
