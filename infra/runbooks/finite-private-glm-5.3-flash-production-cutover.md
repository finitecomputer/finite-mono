# Finite Private: GLM-5.3-Flash production cutover

Status: **preparation only**. The intended maintenance window begins at
`2026-08-28 02:30 America/Chicago` (`2026-08-28 07:30 UTC`) and has a hard
`05:30 America/Chicago` (`10:30 UTC`) terminal boundary. This file and its
draft PR do not authorize an image publication, satellite release, Tinfoil
container create/delete/replace/relaunch, DNS change, Runtime rollout, or load
against the serving endpoint.

The goal is to replace the single eight-H200 DeepSeek workload with
GLM-5.3-Flash, prove useful protocol behavior and capacity for at least 120
simultaneous users, and give the GPU container the model-independent
`finite-private` name. The model must still identify itself as
`glm-5-3-flash`.

The generated route changes with the container name. Existing issued Runtimes
still read `kimi-k2-6.finite.containers.tinfoil.dev`, so this is a two-container
topology after cutover:

| Role | External Tinfoil name | Workload | Public route |
| --- | --- | --- | --- |
| Inference | `finite-private` | GLM-5.3-Flash + Finite limiter, 8 H200s | `finite-private.finite.containers.tinfoil.dev` |
| Compatibility | `kimi-k2-6` | CPU-only Caddy bridge, no model or secrets | `kimi-k2-6.finite.containers.tinfoil.dev` |

The direct `finite-private` route is the capacity measurement target. The
historical route gets bounded protocol/accounting canaries because its extra
enclave hop is compatibility infrastructure, not the serving-speed reference.

## Fixed acceptance contract

The first candidate uses the official SGLang H200 throughput recipe:

- checkpoint
  `zai-org/GLM-5.3-Flash@04c4e9e95c5da8862dced7e5056455116f83a7e0`;
- TP8 + EP8;
- TileLang DSA for prefill and decode;
- DeepGEMM MoE runner;
- BF16 KV cache;
- `glm45` reasoning and `glm47` tool-call parsers;
- 393,216-token initial service ceiling; and
- no speculative decoding or unmeasured scheduler override.

The 120-user gate is hard. Every one of three 120-way repetitions must have:

- 120/120 successful requests and terminal SSE streams;
- zero transport, HTTP, JSON, usage, or terminal-stream errors;
- p50 decode of at least 20 output tokens/second per request;
- p10 decode of at least 10 output tokens/second per request;
- at least 2,400 aggregate output tokens/second; and
- p95 TTFT no greater than 10 seconds.

Any failed correctness, authentication, accounting, capacity, stability,
fleet-delta, or compatibility gate means **rollback immediately**. Do not tune
live through a failed tier. Speculative EAGLE decoding, a lower static memory
fraction, larger context, a different KV format, or different request limits
require a separately reviewed candidate.

If full qualification, including the 35-minute soak, is not complete by
`04:45 America/Chicago`, begin rollback. The retained DeepSeek deployment took
about 35 minutes to become ready in prior windows; the 45-minute rollback
reserve is part of the acceptance contract, not optional test time. At 05:30
the historical route must be healthy on either fully qualified GLM or the exact
restored DeepSeek release.

## Stop markers and immutable inputs

Do not enter the window until all values below are real immutable identities,
the preparation and release contracts pass, and a second operator has reviewed
the decoded satellite files. Never put secret values in this runbook, a shell
history, logs, or git.

```bash
export FINITE_MONO_SHA='REPLACE_WITH_MERGED_MONO_COMMIT'
export FINITE_PRIVATE_GLM_TAG='REPLACE_WITH_MAIN_SATELLITE_RELEASE_TAG'
export FINITE_PRIVATE_BRIDGE_TAG='REPLACE_WITH_BRIDGE_SATELLITE_RELEASE_TAG'
export FINITE_PRIVATE_GLM_IMAGE='REPLACE_WITH_GHCR_TAG_AND_AMD64_DIGEST'
export FINITE_PRIVATE_LIMITER_IMAGE='REPLACE_WITH_GHCR_TAG_AND_AMD64_DIGEST'
export FINITE_PRIVATE_MODEL_MPK='REPLACE_WITH_TINFOIL_MODELWRAP_MPK'
export FINITE_PRIVATE_MODEL_ROOT='REPLACE_WITH_64_HEX_ROOT_HASH'
export FINITE_PRIVATE_MAIN_CONFIG_SHA256='REPLACE_WITH_SHA256'
export FINITE_PRIVATE_BRIDGE_CONFIG_SHA256='REPLACE_WITH_SHA256'
export FINITE_PRIVATE_MAIN_DEPLOYMENT_SHA256='REPLACE_WITH_SHA256'
export FINITE_PRIVATE_MAIN_HASH_SHA256='REPLACE_WITH_SHA256'
export FINITE_PRIVATE_BRIDGE_DEPLOYMENT_SHA256='REPLACE_WITH_SHA256'
export FINITE_PRIVATE_BRIDGE_HASH_SHA256='REPLACE_WITH_SHA256'
```

The checked-in candidates intentionally contain `REPLACE_WITH_*` values. The
preparation contract accepts only the named stop markers; the promotion
contract rejects all of them:

```bash
just finite-private-glm53-contract
just finite-private-glm53-release-contract
```

## Non-disruptive preparation before the window

These steps create reviewed artifacts but do not contact the serving endpoint
or change the production container.

### 1. Merge and publish source-labelled images

The image workflows build on CI, preserve an OCI source revision label, publish
only linux/amd64, require provenance, verify the exact promoted digest, prove
anonymous GHCR pull, and keep production tags behind
`FINITE_GHCR_PRODUCTION_PUBLISH_ENABLED`.

After this PR is merged, dispatch from the merged commit. Use one version string
for both images and retain the two workflow URLs and resulting digests:

```bash
export FINITE_MONO_SHA="$(git rev-parse origin/main)"
export GLM_IMAGE_VERSION='2026-08-28.1'

gh variable set FINITE_GHCR_PRODUCTION_PUBLISH_ENABLED \
  --body true \
  --repo finitecomputer/finite-mono

gh workflow run glm-5.3-flash-sglang-image.yml \
  --repo finitecomputer/finite-mono \
  --ref main \
  -f version="$GLM_IMAGE_VERSION" \
  -f publish_production=true

gh workflow run service-images.yml \
  --repo finitecomputer/finite-mono \
  --ref main \
  -f image=private-limiter \
  -f version="$GLM_IMAGE_VERSION" \
  -f publish_production=true
```

GitHub workflow dispatch accepts a branch or tag here, not a raw commit SHA.
Immediately require both runs' `headSha` to equal `$FINITE_MONO_SHA`; a mismatch
invalidates the runs. Require both jobs to pass, inspect the anonymous
linux/amd64 digests and OCI revision labels, and delete or restore the
`FINITE_GHCR_PRODUCTION_PUBLISH_ENABLED` variable to its pre-window value as
soon as both promotions finish. Do not copy a mutable tag without its digest.

### 2. Generate and bind the modelwrap artifact

Generate a Tinfoil Models artifact from the exact checkpoint revision above.
Record the complete MPK descriptor and its 64-hex root. Replace the MPK and
mounted root placeholders together; the repository contract rejects a mismatch.
This is an external artifact publication and needs operator authorization even
though it does not affect live inference.

Replace both immutable image placeholders, then run:

```bash
just finite-private-glm53-contract
just finite-private-glm53-release-contract
git diff --check
sha256sum \
  infra/tinfoil/confidential-finite-private/tinfoil-config.glm-5.3-flash.candidate.yml \
  infra/tinfoil/confidential-kimi-k2-6/tinfoil-config.compatibility-bridge.candidate.yml
```

Record the two checksums as the expected config checksums. A different checksum
at release time is a stop condition.

### 3. Publish the main and bridge satellites

`finitecomputer/confidential-finite-private` did not exist during preparation.
Create it only after the source PR is merged. Scaffold its release workflow from
the reviewed Tinfoil satellite workflow, not from an unreviewed model template.
The repo contains only the release workflow, README, and the reviewed candidate
copied as `tinfoil-config.yml`.

The bridge release belongs to the existing
`finitecomputer/confidential-kimi-k2-6` satellite. Create its branch from the
commit/tag actually serving at the pre-window inventory, never from stale
satellite `main`, and replace only `tinfoil-config.yml` with the bridge
candidate.

For each satellite:

1. review `git diff --word-diff=plain` and the decoded Tinfoil deployment;
2. verify the source config SHA-256 matches the expected value;
3. dispatch `tinfoil-release.yml` with `--ref` set to the reviewed commit;
4. require the release tag to resolve to that exact commit; and
5. download `tinfoil-deployment.json` and `tinfoil.hash`, record their SHA-256
   values, and retain the files in the private evidence directory.

Publication does not authorize a container operation.

## Pre-window read-only inventory

Run immediately before the approved window. Store all output under a mode-0700
directory outside git. Production identity must come from this inventory, not
from an old runbook.

```bash
set -euo pipefail
export FINITE_PRIVATE_EVIDENCE_DIR=".local-state/glm53-cutover/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$FINITE_PRIVATE_EVIDENCE_DIR"
chmod 700 "$FINITE_PRIVATE_EVIDENCE_DIR"

scripts/finite-status --json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/finite-status-before.json"
tinfoil container get kimi-k2-6 --output json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/tinfoil-before.json"
tinfoil container hosts --output json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/tinfoil-hosts-before.json"
tinfoil container metrics kimi-k2-6 --time 1h --output json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/tinfoil-metrics-before.json"
```

From the retained JSON, explicitly set and print only these non-secret values:

```bash
export FINITE_PRIVATE_ROLLBACK_CONTAINER_ID='REPLACE_FROM_TINFOIL_BEFORE'
export FINITE_PRIVATE_ROLLBACK_REPO='finitecomputer/confidential-kimi-k2-6'
export FINITE_PRIVATE_ROLLBACK_TAG='REPLACE_FROM_TINFOIL_BEFORE'
export FINITE_PRIVATE_ROLLBACK_HOST='REPLACE_FROM_TINFOIL_BEFORE'
```

Verify that the live record has eight H200s, debug mode is false, its repository
and tag exist, and its mounted secret names are exactly:

- `VLLM_API_KEY`;
- `VLLM_INTERNAL_API_KEY`; and
- `FINITE_USAGE_API_SERVICE_KEY`.

Never display secret values. Confirm all four rollback values are non-empty and
that the rollback tag resolves to the retained deployment assets. Validate the
bridge release through Tinfoil before the window and require a two-CPU,
2,048-MiB, zero-GPU, secret-free result plus organization capacity for one
additional instance. The bridge uses Tinfoil's shared CPU scheduling and must
not be pinned to the dedicated H200 host. Confirm there is still one active
eight-H200 allocation; this plan does not assume a second lab rack.

Select one existing internal canary Agent with known durable chat history and
record its non-secret Account, Project, Runtime, Agent Principal, and Room
identifiers. Confirm the same Room is usable before the window. Also reserve a
single-use internal Launch Code and a fresh canary email for the post-cutover
new-user proof. Creating or consuming either is production state; do it only in
the authorized window and never use a customer account as the canary.

Record the accounting boundary, then prove the current service at bounded load:

```bash
export FINITE_PRIVATE_LEDGER_SINCE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
infra/runbooks/finite-private-ops.sh gate
infra/runbooks/finite-private-ops.sh stream-canary
infra/runbooks/finite-private-ops.sh responses-canary
infra/runbooks/finite-private-ops.sh mixed-version-canary
infra/runbooks/finite-private-ops.sh load-canary 1
infra/runbooks/finite-private-ops.sh load-canary 32
infra/runbooks/finite-private-ops.sh settlement-status \
  "$FINITE_PRIVATE_LEDGER_SINCE"

python3 scripts/check_deepseek_v4_0731_quality.py \
  --endpoint https://kimi-k2-6.finite.containers.tinfoil.dev/v1 \
  --model deepseek-v4-flash-0731 \
  --lane self-hosted \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/deepseek-quality-before.json"

python3 scripts/prepare_glm53_blind_comparison.py capture \
  --endpoint https://kimi-k2-6.finite.containers.tinfoil.dev/v1 \
  --model deepseek-v4-flash-0731 \
  --lane reference \
  --output "$FINITE_PRIVATE_EVIDENCE_DIR/blind-reference.json"
```

Do not run the new 120-user gate against current production during preparation.
Any new/worsened fleet red or unknown, serving error, unsettled rollout-era
reservation, unavailable rollback artifact, or identity mismatch stops the
window.

## Named pre-existing fleet exceptions

Two `fleet_convergence` findings are pre-existing on this fleet. They were
present before this runbook was written, they are identical on the DeepSeek
baseline, and neither is caused by or causal to this model swap. They are
named here so the window starts from an honest baseline instead of a gate
that cannot go green:

1. **Every active Runtime reports readiness `unknown`.** The runner-ferried
   standing-readiness feature (deployed 2026-08-27) registers a health-report
   target only when a Runtime completes a fresh launch or relocation
   (`finite-saas-runner/src/lib.rs` calls `record_target` only on launch
   completion; the upgrade path's `refresh_target_endpoint` is a no-op without
   an existing registry entry, `finite-saas-runner/src/health_reports.rs`).
   The entire fleet was upgraded in place, so no current Runtime can report
   until it next launches. Carry this exception only while all of the
   following hold in every `scripts/finite-status --json` run:
   - the set of `health_unknown` runtime IDs is exactly the set in the
     retained before report;
   - `health_not_ready` is empty on every host;
   - `health_ready_count` is zero or unchanged.
   Any Runtime leaving `unknown` toward `not_ready`, any newly `unknown`
   runtime, or any readiness change observed mid-window stops the window
   immediately.
2. **Distribution aggregate inconsistent with the detail snapshot.** The
   aggregate query inner-joins `runtime_artifacts` while the detail snapshot
   left-joins it, so rows without a runtime artifact appear only in the
   snapshot (today, the single documented artifact-less `smoke` row,
   `infra/deployment-changelog.md`). Carry this exception only while the
   disagreement is explained entirely by such rows and their count matches
   the before report.

Anything else in `fleet_convergence` is judged against the acceptance
contract exactly as written. The reporting gap in exception 1 must be fixed
and the fleet observed reporting before any future runbook relies on
readiness signals.

## Window procedure

Every mutating command below requires the operator's explicit authorization for
the exact values recorded above. Keep a second terminal open with this rollback
section and the retained pre-window JSON.

### Resume rules

Treat the procedure as a small state machine. Before a first attempt or resume,
read both names with `tinfoil container get ... --output json` and compare IDs,
repos, tags, hosts, GPU counts, debug/staging flags, and secret names with the
retained evidence.

| Observed state | Allowed next action |
| --- | --- |
| Exact old GPU ID exists as `kimi-k2-6`; `finite-private` is absent | Run the GPU `--replace` once. |
| Exact target `finite-private` exists; `kimi-k2-6` is absent | Do not reuse the consumed rollback ID. Create the exact bridge. |
| Exact target main and bridge both exist | Skip both create commands and resume at readiness. |
| Anything else, including a different ID/tag under either name | Stop. Do not replace, delete, or adopt it. |

The retained create JSON is the authority for the new IDs. A shell failure after
a successful control-plane request is not permission to rerun it. Re-read state
and follow the table. Apply the same rule during rollback: if the exact restored
old release already exists, skip its create and continue with verification.

### 1. Replace the GPU workload under the generic name

Re-run both repository contracts and compare all six artifact checksums with the
approved values. Then create `finite-private`, atomically replacing only the
exact inventoried old container ID:

There is no zero-downtime path with one eight-H200 allocation and a generated
hostname that cannot move. The approved maintenance interruption begins when
`--replace` consumes the old container and ends only when GLM and the historical
bridge are both ready. Create the bridge immediately after the GPU replacement;
do not extend this interruption with protocol or quality testing first.

```bash
test -n "$FINITE_PRIVATE_ROLLBACK_CONTAINER_ID"
test -n "$FINITE_PRIVATE_ROLLBACK_HOST"
test -n "$FINITE_PRIVATE_GLM_TAG"

tinfoil container create finite-private \
  --replace "$FINITE_PRIVATE_ROLLBACK_CONTAINER_ID" \
  --host "$FINITE_PRIVATE_ROLLBACK_HOST" \
  --repo finitecomputer/confidential-finite-private \
  --tag "$FINITE_PRIVATE_GLM_TAG" \
  --secret VLLM_API_KEY \
  --secret VLLM_INTERNAL_API_KEY \
  --secret FINITE_USAGE_API_SERVICE_KEY \
  --output json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/finite-private-create.json"
```

Do not pass `--debug`, `--disable-cc-mode`, `--staging`, variables, SSH keys, or
a custom domain. Read the new exact ID from the retained create JSON, then
immediately recreate the old external name from the reviewed CPU-only bridge
release. It receives no secrets, variables, SSH keys, model, or GPU flags:

```bash
export FINITE_PRIVATE_NEW_CONTAINER_ID='REPLACE_FROM_CREATE_JSON'

tinfoil container create kimi-k2-6 \
  --repo finitecomputer/confidential-kimi-k2-6 \
  --tag "$FINITE_PRIVATE_BRIDGE_TAG" \
  --output json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/compatibility-bridge-create.json"

export FINITE_PRIVATE_BRIDGE_CONTAINER_ID='REPLACE_FROM_CREATE_JSON'
export FINITE_PRIVATE_CONTAINER=finite-private
export FINITE_PRIVATE_ENDPOINT='https://finite-private.finite.containers.tinfoil.dev'
export FINITE_PRIVATE_MODEL=glm-5-3-flash
export FINITE_PRIVATE_EXPECTED_RESPONSE_MODEL=glm-5-3-flash
infra/runbooks/finite-private-ops.sh wait-ready
tinfoil container get "$FINITE_PRIVATE_NEW_CONTAINER_ID" --output json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/finite-private-ready.json"

FINITE_PRIVATE_CONTAINER=kimi-k2-6 \
  FINITE_PRIVATE_ENDPOINT=https://kimi-k2-6.finite.containers.tinfoil.dev \
  infra/runbooks/finite-private-ops.sh wait-ready
```

Readiness may take about 45 minutes while the 305.8-GiB checkpoint becomes
available. Require the exact release tag, eight GPUs, false debug/staging, three
secret names, attested direct route, and healthy `/live`, `/health`, and
`/ready`. The bridge healthcheck intentionally follows the main `/live`, so the
historical route becomes ready with the model rather than before it.

### 2. Prove canonical and mixed-version APIs directly

Run the complete protocol/accounting gates once for the canonical model and
once for each previously issued label:

```bash
for model in glm-5-3-flash deepseek-v4-flash-0731 glm-5-2; do
  FINITE_PRIVATE_MODEL="$model" infra/runbooks/finite-private-ops.sh canary
  FINITE_PRIVATE_MODEL="$model" infra/runbooks/finite-private-ops.sh stream-canary
  FINITE_PRIVATE_MODEL="$model" infra/runbooks/finite-private-ops.sh responses-canary
done

infra/runbooks/finite-private-ops.sh negative-canary
infra/runbooks/finite-private-ops.sh settlement-status \
  "$FINITE_PRIVATE_LEDGER_SINCE"
```

All successful responses and public health metadata must identify
`glm-5-3-flash`, even when the accepted request label is
`deepseek-v4-flash-0731` or `glm-5-2`. Send a validly authenticated request with
an unrecognized model and require HTTP 400 `unsupported_model`; compare the
settlement report before and after to prove it did not create a reservation.

Separately require:

- thinking disabled returns ordinary content without leaked parser markers;
- thinking enabled with `reasoning_effort=high` returns parsed reasoning and a
  final answer;
- one forced tool choice returns a structured `tool_calls` object with valid
  JSON arguments;
- chat streaming terminates with `[DONE]` and positive usage;
- `/v1/responses` returns a response ID and output; and
- invalid Finite credentials never reach inference.

The protocol gate must also cover the newly documented GLM behavior before
capacity load begins:

- explicit `clear_thinking=true` over assistant history, with reasoning kept
  separate from final content;
- non-streaming and streaming tool calls with valid JSON arguments;
- a tool result fed back into a second model turn;
- two requested tools returned as parallel tool calls;
- `response_format={"type":"json_object"}` content that parses as the requested
  JSON object;
- a client-cancelled stream followed by healthy inference and settled usage;
- malformed JSON and unsupported fields failing without a stuck reservation;
- one approximately 128,000-token prompt and one approximately 360,000-token
  near-limit prompt at the fixed 393,216-token service ceiling; and
- a short request immediately after each long prefill, proving the service did
  not wedge or restart.

Store the exact request generators and sanitized outputs in the evidence
directory. The report may contain prompts, response identity, parser shapes,
timings, usage, and errors, but never the canary key or full generated answers.
Check settlement after cancellation, malformed input, and each long-context
call. Any OOM, restart, parser ambiguity, nonterminal stream, invalid JSON, or
rollout-era reserved row means rollback immediately.

Run the checked-in generator against the direct route. It emits only sanitized
case metadata and token counts:

```bash
python3 scripts/check_finite_private_glm53_protocol.py \
  --endpoint https://finite-private.finite.containers.tinfoil.dev/v1 \
  --api-key-env FINITE_PRIVATE_CANARY_API_KEY \
  --timeout-seconds 1200 \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/glm53-protocol.json"
infra/runbooks/finite-private-ops.sh settlement-status \
  "$FINITE_PRIVATE_LEDGER_SINCE"
```

Any parser, model-identity, or settlement failure means rollback immediately.

Run the fixed GLM reasoning/tool suite at low, high, and max effort through the
direct route. This is a minimum eligibility floor, not a broad benchmark; all
18 cases and the canonical response identity must pass:

```bash
python3 scripts/check_finite_private_glm53_quality.py \
  --endpoint https://finite-private.finite.containers.tinfoil.dev/v1 \
  --model glm-5-3-flash \
  --efforts low,high,max \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/glm53-quality.json"
```

Compare its deterministic arithmetic, logic, code reasoning, exact instruction,
and tool-selection outcomes with the retained pre-window DeepSeek report. A GLM
failure where the pre-window DeepSeek lane passed is a rollback condition; do
not excuse it with throughput.

Capture the candidate side of the fixed long-horizon and adversarial packet,
then generate randomized A/B responses and a separate key:

```bash
python3 scripts/prepare_glm53_blind_comparison.py capture \
  --endpoint https://finite-private.finite.containers.tinfoil.dev/v1 \
  --model glm-5-3-flash \
  --lane candidate \
  --output "$FINITE_PRIVATE_EVIDENCE_DIR/blind-candidate.json"

python3 scripts/prepare_glm53_blind_comparison.py packet \
  --reference "$FINITE_PRIVATE_EVIDENCE_DIR/blind-reference.json" \
  --candidate "$FINITE_PRIVATE_EVIDENCE_DIR/blind-candidate.json" \
  --seed "$FINITE_PRIVATE_GLM_TAG" \
  --output "$FINITE_PRIVATE_EVIDENCE_DIR/blind-packet.json" \
  --key-output "$FINITE_PRIVATE_EVIDENCE_DIR/blind-key.json"
```

Two reviewers independently score copies of `blind-packet.json` without opening
the key. For every case they separately record correctness pass/fail,
tool-safety pass/fail, and a concrete note for response A and response B, then
record A/B/tie preference. Resolve scoring-rule
disagreements before unblinding. After opening the key, GLM must have no
tool-safety failure, at least five of six correctness passes from each reviewer,
and no more than one additional preference loss versus the reference. A tie is
neutral. Retain both completed packets and the key; any missed floor means
rollback immediately.

### 3. Prove the restored historical route

The bridge was created immediately after GPU replacement. Switch the operator
environment to it and verify the exact bridge ID, release, CPU-only shape,
secret-free manifest, and public health:

```bash
export FINITE_PRIVATE_CONTAINER=kimi-k2-6
export FINITE_PRIVATE_ENDPOINT='https://kimi-k2-6.finite.containers.tinfoil.dev'
export FINITE_PRIVATE_MODEL=glm-5-3-flash
export FINITE_PRIVATE_EXPECTED_RESPONSE_MODEL=glm-5-3-flash
tinfoil container get "$FINITE_PRIVATE_BRIDGE_CONTAINER_ID" --output json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/compatibility-bridge-ready.json"
infra/runbooks/finite-private-ops.sh gate
```

Through the historical route, run canonical chat/stream/Responses canaries and
both `deepseek-v4-flash-0731` and `glm-5-2` alias canaries. Require model
identity `glm-5-3-flash`, terminal streams, accounting settlement, and the same
invalid-key behavior. Do not begin the capacity sweep until this compatibility
proof passes.

Prove the chat through-line through the normal product, not an operator API:

1. Sign in as the recorded existing internal canary, open the same dashboard
   Room, and confirm its pre-window history and Agent identity are unchanged.
2. Send two ordinary messages. Both must reach the existing Runtime through the
   historical URL and historical request label, and both must receive complete
   Hermes replies without re-enrollment, rebinding, duplicate messages, or a
   new Room.
3. In a separate browser session, redeem the reserved single-use Launch Code as
   the fresh internal canary. Complete Account enrollment, Agent admission,
   Runner launch, `/contact` identity readiness, Hosted Web Device binding, and
   two real dashboard chat turns.
4. Correlate both canaries with canonical `glm-5-3-flash` reserve/settle records
   created after `$FINITE_PRIVATE_LEDGER_SINCE`. Do not inspect or rewrite their
   durable Hermes configuration.

If a normal user must use a worksheet, shell, raw JSON, or manual state repair
to complete either chat, the canary failed. Keep the fresh canary's recovery
state; model rollback is not purge authority.

### 4. Prove 120-user capacity on the direct route

Return the operator environment to the direct route. The acceptance CLI reads
the key from the named environment variable and records no credential or
response text:

```bash
export FINITE_PRIVATE_CONTAINER=finite-private
export FINITE_PRIVATE_ENDPOINT='https://finite-private.finite.containers.tinfoil.dev'
export FINITE_PRIVATE_MODEL=glm-5-3-flash
export FINITE_PRIVATE_LOAD_SWEEP_APPROVED='1,4,8,16,32,64,128,256'

python3 scripts/check_finite_private_glm53_capacity.py \
  --url "$FINITE_PRIVATE_ENDPOINT" \
  --api-key-env FINITE_PRIVATE_CANARY_API_KEY \
  --concurrency 1,32,64,120 \
  --repetitions 3 \
  --output-tokens 256 \
  --thinking on \
  --reasoning-effort high \
  --tag "$FINITE_PRIVATE_GLM_TAG" \
  | tee "$FINITE_PRIVATE_EVIDENCE_DIR/capacity.jsonl"
```

The CLI exits nonzero if any required 120-way repetition misses the fixed hard
gate. Inspect the lower tiers for monotonic degradation and errors; they are
diagnostic, while every 120-way repetition is binding. Immediately run one
clean stream canary and settlement status. Capture one-hour Tinfoil metrics and
require no GPU/host OOM, restart, stuck request, malformed stream, or limiter
settlement error.

### 5. Hold a 35-minute soak

After the three-run acceptance sweep, sustain the exact hard tier for a
35-minute soak. Each iteration is one 120-way repetition followed by direct and
historical-route health/stream checks and settlement status. Stop on the first
failure; do not issue a recovery load.

```bash
soak_started="$(date +%s)"
soak_deadline="$((soak_started + 2100))"
soak_run=0
while [ "$(date +%s)" -lt "$soak_deadline" ]; do
  soak_run="$((soak_run + 1))"
  python3 scripts/check_finite_private_glm53_capacity.py \
    --url https://finite-private.finite.containers.tinfoil.dev \
    --api-key-env FINITE_PRIVATE_CANARY_API_KEY \
    --concurrency 120 \
    --repetitions 1 \
    --output-tokens 256 \
    --thinking on \
    --reasoning-effort high \
    --tag "$FINITE_PRIVATE_GLM_TAG-soak-$soak_run" \
    | tee -a "$FINITE_PRIVATE_EVIDENCE_DIR/soak.jsonl"

  FINITE_PRIVATE_ENDPOINT=https://finite-private.finite.containers.tinfoil.dev \
    FINITE_PRIVATE_CONTAINER=finite-private \
    FINITE_PRIVATE_MODEL=glm-5-3-flash \
    infra/runbooks/finite-private-ops.sh stream-canary
  FINITE_PRIVATE_ENDPOINT=https://kimi-k2-6.finite.containers.tinfoil.dev \
    FINITE_PRIVATE_CONTAINER=kimi-k2-6 \
    FINITE_PRIVATE_MODEL=deepseek-v4-flash-0731 \
    infra/runbooks/finite-private-ops.sh stream-canary
  infra/runbooks/finite-private-ops.sh settlement-status \
    "$FINITE_PRIVATE_LEDGER_SINCE"
done
```

Every iteration must pass the same 120/120, p10, p50, aggregate, and p95 TTFT
thresholds. Record iteration count and elapsed wall time; a gap or early exit is
not a 35-minute soak.

### 6. Close the observation boundary

```bash
tinfoil container get finite-private --output json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/tinfoil-main-after.json"
tinfoil container get kimi-k2-6 --output json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/tinfoil-bridge-after.json"
tinfoil container metrics finite-private --time 1h --output json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/tinfoil-main-metrics-after.json"
tinfoil container metrics kimi-k2-6 --time 1h --output json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/tinfoil-bridge-metrics-after.json"
infra/runbooks/finite-private-ops.sh settlement-status \
  "$FINITE_PRIVATE_LEDGER_SINCE"
scripts/finite-status --json \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/finite-status-after.json"
```

Any new or worsened red/unknown result, unsettled rollout-era reservation,
container restart, resource exhaustion, or route failure means rollback
immediately. Only the exceptions in `Named pre-existing fleet exceptions` may
carry through, and only while every one of their carry conditions still holds
against the retained before report; a violated carry condition is a new
finding and stops the window.

## Rollback

Rollback recreates the exact pre-window external identity and release. It does
not attempt to repair or migrate durable user state.

1. Stop new operator load.
2. If the compatibility bridge exists, verify its exact ID from retained JSON
   and delete only that ID:

   ```bash
   test -n "${FINITE_PRIVATE_BRIDGE_CONTAINER_ID:-}"
   tinfoil container get "$FINITE_PRIVATE_BRIDGE_CONTAINER_ID" --output json
   tinfoil container delete "$FINITE_PRIVATE_BRIDGE_CONTAINER_ID"
   ```

3. Verify `$FINITE_PRIVATE_NEW_CONTAINER_ID` is the failed `finite-private`
   container and atomically replace it with the exact inventoried release under
   the old name:

   ```bash
   test -n "$FINITE_PRIVATE_NEW_CONTAINER_ID"
   test -n "$FINITE_PRIVATE_ROLLBACK_TAG"
   test -n "$FINITE_PRIVATE_ROLLBACK_HOST"
   tinfoil container get "$FINITE_PRIVATE_NEW_CONTAINER_ID" --output json

   tinfoil container create kimi-k2-6 \
     --replace "$FINITE_PRIVATE_NEW_CONTAINER_ID" \
     --host "$FINITE_PRIVATE_ROLLBACK_HOST" \
     --repo "$FINITE_PRIVATE_ROLLBACK_REPO" \
     --tag "$FINITE_PRIVATE_ROLLBACK_TAG" \
     --secret VLLM_API_KEY \
     --secret VLLM_INTERNAL_API_KEY \
     --secret FINITE_USAGE_API_SERVICE_KEY \
     --output json \
     > "$FINITE_PRIVATE_EVIDENCE_DIR/rollback-create.json"
   ```

4. Restore the default operator environment and wait for the old route:

   ```bash
   export FINITE_PRIVATE_CONTAINER=kimi-k2-6
   export FINITE_PRIVATE_ENDPOINT=https://kimi-k2-6.finite.containers.tinfoil.dev
   export FINITE_PRIVATE_MODEL=deepseek-v4-flash-0731
   unset FINITE_PRIVATE_EXPECTED_RESPONSE_MODEL
   infra/runbooks/finite-private-ops.sh wait-ready
   infra/runbooks/finite-private-ops.sh gate
   infra/runbooks/finite-private-ops.sh stream-canary
   infra/runbooks/finite-private-ops.sh responses-canary
   infra/runbooks/finite-private-ops.sh mixed-version-canary
   infra/runbooks/finite-private-ops.sh settlement-status \
     "$FINITE_PRIVATE_LEDGER_SINCE"
   scripts/finite-status --json \
     > "$FINITE_PRIVATE_EVIDENCE_DIR/finite-status-rollback.json"
   ```

Do not delete failed-release evidence until the restored route, accounting, and
fleet status are proven. Record the rollback reason, exact IDs/tags, time to
readiness, and final state in `infra/deployment-changelog.md` through a reviewed
follow-up PR.

## Runtime/default-model migration is a separate phase

The limiter canonicalizes the canonical GLM label and both issued historical
labels to `glm-5-3-flash`, so the model returned by SGLang and used for new Core
accounting is correct immediately. The CPU bridge preserves the old URL.

Do not migrate durable Runtime configurations, roll a Runtime image, or change
Runner/Hermes defaults in this first model cutover. Existing durable readers
must continue to work through the old URL and old labels during the mixed-version
period. A later reviewed rollout can change new-user defaults and narrowly
migrate only exact Finite-owned provider shapes after the model has passed this
window. User-owned/custom provider configuration must remain untouched.

The compatibility bridge can be retired only after a stable custom route is
available, `scripts/finite-status --json` inventories every reader on it, and a
full observation window records zero reads of the generated historical route.
