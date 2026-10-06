# DeepSeek V4.1 Flash: H200 maintenance test and retry

The **September 17 retry completed**, with DeepSeek passing protocol checks and
concurrency 1–64. Concurrency 128 completed without request errors but failed
the first-token latency threshold. The original GLM release was restored and
verified with final stability checks by **04:22 America/Chicago
(CDT, UTC−05:00)**. See the execution result
below. No further swap is armed.

The retained procedure describes a temporary A/B test with GLM restoration,
not a permanent model promotion. Host model-pack preparation is separate from
the serving container. A future start is not evidence that a scheduler has
been armed.

## Window and recovery

Executed retry window: **September 17, 03:00–06:00 Central
(08:00–11:00 UTC)**. The user instructed the live session to wait until
02:30 for preflight and continue the prepared attempt. No scheduler was used.

| Central | Action |
| --- | --- |
| 03:00–03:10 | Fresh status, GLM protocol canary and bounded baseline |
| By 03:10 | Start the measured DeepSeek candidate only if all entry checks pass |
| By 04:15 | Candidate must answer authenticated requests; otherwise restore GLM |
| 04:15–05:15 | Protocol, model aliases, bounded performance tiers |
| **05:15** | **Stop tests and start GLM restoration even if DeepSeek passes** |
| **06:00** | GLM healthy; final status and accounting evidence retained |

The 45-minute restoration reserve is based on prior roughly 35-minute model
loads, not a guaranteed recovery time. If GLM is not healthy by 06:00, treat it
as an incident and continue recovery. Do not extend testing into that reserve.
A late start shortens testing; it never moves the end of the window. No new
candidate start after 03:10. No second engine/config attempt within this window.
The retry allows initial DeepSeek startup until 04:15, while retaining
one hour for protocol/throughput checks and the full restoration reserve.

## Exact inputs

- Live identity observed during preparation: `finite-private`, UUID
  `acc651a6-9de6-4da5-9fdc-bb9888245962`, status `ready`, host
  `control.inf9.tinfoil.sh`, eight H200s, TDX, auto-update disabled.
- Satellite: `finitecomputer/confidential-finite-private`.
- **Restore release:** `v2026-08-28-glm-5-3-flash-5`, commit
  `f4c7cf20e0d02fb8d9078c0f6ea8995fe570b0dc`.
- Restore deployment SHA256:
  `164712aca02fd5094768f95a3a6d9b6f9589daea0f471a21bff44963fd212f31`.
- Restore decoded config SHA256:
  `c4da30ecd93111cd06b6470abfaf872ea92e059fa7d71f37e5c0157881dfa03b`.
- Candidate source:
  [`tinfoil-config.deepseek-v4.1-flash.candidate.yml`](../tinfoil/confidential-finite-private/tinfoil-config.deepseek-v4.1-flash.candidate.yml).
- Tinfoil upstream: `tinfoilsh/confidential-deepseek-v4-1-flash` release
  `v0.0.3`, commit `0cc8920b9db0efa007c5bd6b57b9784f53fc71ff`.
- Upstream deployment SHA256:
  `8448fed68f4ed10c829a6433bdf570fa12a7b0e6d589bf1a89a8e9a5ca37a3ac`.
- Upstream image index digest:
  `sha256:fd77a686a50cc854424a55d6222530e6d5197d9b2f999ede32019b5dfd0ea9c0`;
  linux/amd64 manifest
  `sha256:a28f300c5465f34d8e3f76d2979ae1cccb9a978a1af619ad9cf11e852b79ac28`.
- Model: `deepseek-ai/DeepSeek-V4.1-Flash` revision
  `dba1be0a40aa45a94ad051997016db3960a90277`; schema-2 MPK/root are pinned in
  the candidate. Its size is 510,313,381,888 bytes.
- Existing limiter image is unchanged from the decoded live GLM release.
- Secret names remain `VLLM_API_KEY`, `VLLM_INTERNAL_API_KEY`, and
  `FINITE_USAGE_API_SERVICE_KEY`. Use the existing sealed values; never print
  or commit them. Canary credential location on this operator machine:
  `/Users/plebdev/Desktop/Projects/finite-mono/secrets/finite-private-canary.env`.

## Candidate decisions and remaining runtime proof

Use Tinfoil's published patched image: its GPU device-mirror patch avoids
host-mapped GPU reads under TDX. Keep Engram tables on GPU; do not copy the
InferenceX CPU-offload setting into this enclave. The public InferenceX speed
run also uses synthetic acceptance, which this test explicitly does not use.

The candidate uses TP8, real DSpark block rejection with five draft tokens,
64 maximum sequences, 8,192 batched tokens, a 512-token CUDA graph capture cap,
and 0.85 GPU memory utilization. It keeps the product's 393,216-token context
ceiling and serves text only for the first performance comparison.
The image's vision capability is not qualified by this test.

[vLLM issue 56443](https://github.com/vllm-project/vllm/issues/56443) reports a
DSpark profiling overrun when the request count times draft width exceeds the
token buffer. Our explicit budget has margin (64 × 6 = 384 < 8,192), but this
does not prove startup or performance on our CVM. On September 16 the
reporter confirmed the profiling-cap fix runs DSpark on H200; the fix PR
remained unmerged at preparation time. The existing candidate stays below
the triggering buffer limit, so its measured image is unchanged. A CUDA assert, OOM, repeated
restart, unsupported kernel, or readiness deadline means restore GLM, not
patch/tune the running production machine. The CVM changes from 0.10.8 to
0.14.6 with the upstream recipe; reverting the exact GLM release also reverts
that CVM configuration.

## Production through-line and boundaries

Existing Runtime provider configurations send the stable `finite-private`
URL and GLM or older model labels. The Tinfoil shim forwards every public path
to the unchanged limiter. The limiter accepts `glm-5-3-flash`,
`glm-5.3-flash`, `glm-5-2`, and `deepseek-v4-flash-0731`, canonicalizes them to
`deepseek-v4-1-flash`, reserves usage with Core, and forwards to vLLM with
internal authentication. vLLM emits the actual DeepSeek model identity and
usage; the limiter settles the reservation in Core. New accounting records
must name the actual model. Rollback restores the prior alias/model binding.

No durable Runtime provider configuration, Device identity, Agent launch state,
chat transcript, database schema, DNS, or Runner configuration is rewritten.
During model restart, inference fails temporarily; existing chat history and
Core accounting remain on their existing authorities. In-flight requests may
fail and retry. Check their reservation settlement before and after the swap.
Do not purge or repair durable state to make a canary pass.

Recovery boundary: retained measured GLM release, decoded config, immutable
images/model pack, and the existing named sealed secrets. This inference
release is not a backup of user data. Chat/Brain/Runtime recovery authorities
remain unchanged, and the canonical recovery status must stay healthy.

## Entry checks

1. Establish the retry date/rollback cutoff and identify an operator who will watch the
   entire window. Do not arm unattended production replacement with only this
   document as a timer.
2. Publish the candidate from a dedicated satellite branch/tag, after its source
   is committed in mono. Use the existing measured release workflow with
   `mark-as-latest: false`; leave satellite `main`, the latest stable release,
   and the live container unchanged. Record the candidate tag and workflow.
3. Download its `tinfoil-deployment.json` and `tinfoil.hash`. Verify the hash,
   verify the GitHub attestation using predicate type
   `https://tinfoil.sh/predicate/snp-tdx-multiplatform/v1`, and require the decoded `config` bytes to
   equal the reviewed candidate exactly. A tag or image pin alone is not this
   proof. Confirm the config has the existing limiter, secrets by NAME,
   internal model network, and wildcard shim routing.
4. Re-fetch the GLM rollback assets and check the recorded digests. Confirm
   their image/model artifacts remain available. Obtain explicit provider-side
   confirmation that both pinned model packs are staged on the exact host;
   release publication alone does not prove this. Retain both release bundles
   locally before stopping anything. Use the host model-pack gate below, not
   just a release hash or a historical job listing. Both jobs must be complete
   and their exact host, revision, schema, root, offset and verity UUID must match.
5. Re-read Tinfoil state by the exact inventoried UUID. Require the expected
   name/repo/host/tag, eight H200s, `ready`, no staged update, no debug mode,
   the exact three secret names, and auto-update disabled. A mismatch stops
   the window for investigation; do not choose another container by position.
6. Run the canonical `scripts/finite-status` on the **lat2** app plane
   (`64.34.80.19`). The stale lat1 address in historical runbooks is not the
   active Core. When the wrapper is not installed, execute the identical
   `scripts/finite_status.py` implementation over stdin with `main()` appended,
   using the remote NixOS PATH. Retain its JSON and exit status.
7. Compare fresh status with the private preparation evidence. Preparation
   found `chat_plane`, `recovery_boundary`, and `rollout_state` green,
   `fleet_convergence` red, and `host_health` unknown. Do not silently waive
   these: identify the exact pre-existing findings and their carry conditions
   before starting. Recorded detail/distribution snapshots disagree and active
   Agent artifacts span multiple versions; the app-only host lacks `nerdctl`,
   causing container collection to be unknown. These observations do not prove
   all findings harmless. Any new/worsened finding, or uncertain scope, stops entry.
8. Source the canary credential privately. Run limiter readiness, negative
   authentication, one stream, and accounting settlement checks. A bad baseline
   means no model swap. Configure `FINITE_PRIVATE_CORE_HOST=root@64.34.80.19`.

## Host model-pack gate

The supported [Tinfoil model preparation API](https://docs.tinfoil.sh/admin/admin-api#wrap-model)
stages weights without relaunching the serving container. On September 16,
preparation was requested with this exact body:

```json
{"host":"control.inf9.tinfoil.sh","repo":"deepseek-ai/DeepSeek-V4.1-Flash","commit":"dba1be0a40aa45a94ad051997016db3960a90277","schema":2}
```

DeepSeek job: `rigfuoralxtfifii`. GLM rollback job: `zlgwkqrylpqzwjzp`.
The published CLI v0.18.5 source does not forward a schema selector, so the documented
Admin API was used to request schema 2 explicitly. The key remains in the
existing Tinfoil CLI credential file; no credential value is retained in git.

Run this read-only gate during preparation and again immediately before the
swap. It fetches each job live from the exact host. Pending/running/failed,
missing metadata, identity mismatch, and API failures all fail closed. Logs
are excluded from the report. Keep private reports under `.local-state`.

```bash
scripts/with-dev-env python3 scripts/check_finite_private_v41_modelpacks.py \
  --deepseek-job rigfuoralxtfifii \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/modelpacks.json"
```

A completed job is provider evidence of staging, not proof that the engine can
load the model. H200/TDX startup, DSpark, protocol correctness, and sustained
performance still require the maintenance test. Do not delete either pack or
regenerate the already-pinned rollback pack. If the new pack differs from the
measured candidate, investigate the derivation before any release or swap.

## Measurement commands

Use the same checkout and exact release tags for both models. Run project
Python commands through `scripts/with-dev-env`. The benchmark driver defaults
to dry-run; `--execute` sends inference requests but never deploys a model.
It refuses execution before 03:00 or a deadline after 05:15 on the explicit
maintenance date, verifies the live release before each tier,
requires stream model identity, bounds each tier by the supplied deadline,
and refuses to overwrite evidence.

```bash
set -euo pipefail
export FINITE_PRIVATE_EVIDENCE_DIR="$PWD/.local-state/deepseek-v41-20260917/window"
mkdir -p "$FINITE_PRIVATE_EVIDENCE_DIR"
chmod 700 "$FINITE_PRIVATE_EVIDENCE_DIR"
export FINITE_PRIVATE_V41_TAG=v2026-09-16-deepseek-v4-1-flash-test-1
export FINITE_PRIVATE_ENDPOINT=https://finite-private.finite.containers.tinfoil.dev
export FINITE_PRIVATE_CANARY_ENV_FILE=/Users/plebdev/Desktop/Projects/finite-mono/secrets/finite-private-canary.env
set -a
source "$FINITE_PRIVATE_CANARY_ENV_FILE"
set +a
export FINITE_PRIVATE_CORE_HOST=root@64.34.80.19

# Fresh GLM sanity baseline at 1 and 8 requests; deadline 03:08 Central.
# Omit --execute to inspect the plan.
scripts/with-dev-env python3 scripts/finite_private_v41_benchmark.py \
  --model glm --tag v2026-08-28-glm-5-3-flash-5 --window-date 2026-09-17 \
  --deadline-utc 2026-09-17T08:08:00Z --tiers 1,8 \
  --evidence-dir "$FINITE_PRIVATE_EVIDENCE_DIR/baseline" --execute
```

Only after the separate swap procedure and protocol gates succeed:

```bash
# Candidate: deadline 05:15 Central under the planned three-hour retry window.
scripts/with-dev-env python3 scripts/finite_private_v41_benchmark.py \
  --model deepseek --tag "$FINITE_PRIVATE_V41_TAG" --window-date 2026-09-17 \
  --deadline-utc 2026-09-17T10:15:00Z \
  --evidence-dir "$FINITE_PRIVATE_EVIDENCE_DIR/deepseek" --execute
```

The retry refreshes GLM at 1 and 8 requests before the swap; its full September
16 sweep below is the prior-day reference, not a simultaneous matched run.
This avoids consuming the swap deadline or mistaking the known 128-request
latency limit for a protocol failure. DeepSeek retains the full sweep.

The full tiers are 1, 8, 16, 32, 64, 128; three repetitions each, 1,024 output tokens,
thinking on/high, one bounded warmup, and distinct synthetic short prompts.
The 128 tier means 128 concurrent client requests. The measured candidate
retains its 64-sequence engine limit, so this tier also measures queueing and
admission under saturation; it does not claim 128 simultaneously decoding
sequences. Reach it only if the earlier tiers pass and time remains.
Report decode p10/p50, aggregate output, TTFT p50/p95, and errors separately.
Stop escalation on any error, p10 below 10 tok/s, p50 below 20 tok/s, or p95
TTFT above 10 seconds. Aggregate throughput is measured without a claimed
120-user acceptance floor. A failed speed tier marks the capacity limit;
protocol/auth/accounting failure triggers immediate rollback. This test does
not establish long-context concurrency, image performance, or equal reasoning
quality between models' `high` settings.

Before the DeepSeek sweep, use `check_finite_private_glm53_protocol.py --model
deepseek-v4-1-flash --endpoint "$FINITE_PRIVATE_ENDPOINT/v1" --max-context-tokens
128000 --timeout-seconds 90` for reasoning, tools, terminal streams, malformed
requests, and a long-prefill recovery probe. Run it with a total process
deadline before 05:15; an HTTP timeout alone is not a whole-suite deadline.
Its historical script/schema name is retained, but the report's model field
and response checks must be DeepSeek. Also run one stream for each preserved
model alias, expecting `deepseek-v4-1-flash` in the response, and settlement
status. Do not skip compatibility because the canonical model passed.

## Swap and unconditional restoration

Only after all entry checks pass, use the dedicated launch guard. It performs
fresh host-pack checks, requires the exact original ready GLM container and
secret names, and restricts the candidate swap to 03:00–03:10 Central on the
explicit date. It rechecks the clock after remote calls. It does not replace
the fleet, release-attestation, protocol, or accounting checks above.

Omit `--execute` during preparation: this runs only read-only preflight checks.
The wrapper then invokes the existing relaunch operation by UUID; it never
creates, deletes, or renames a container.

```bash
scripts/with-dev-env python3 scripts/finite_private_v41_start.py \
  --window-date 2026-09-17 --deepseek-job rigfuoralxtfifii --execute \
  > "$FINITE_PRIVATE_EVIDENCE_DIR/candidate-start.log" 2>&1
```

Do not use `create --replace`, rename the container, or delete it for a routine
test. If a control-plane command times out, re-read the exact UUID and current
tag/update state before retrying. Do not assume the mutation failed. The
window operator must watch readiness independently of the 120-minute
healthcheck start period in the upstream config.

At the first rollback condition or at 05:15, whichever comes first:

```bash
export FINITE_PRIVATE_RELAUNCH_APPROVED=v2026-08-28-glm-5-3-flash-5
infra/runbooks/finite-private-ops.sh relaunch v2026-08-28-glm-5-3-flash-5
export FINITE_PRIVATE_MODEL=glm-5-3-flash
export FINITE_PRIVATE_EXPECTED_RESPONSE_MODEL=glm-5-3-flash
export FINITE_PRIVATE_READY_TIMEOUT_SECS=2400
infra/runbooks/finite-private-ops.sh wait-ready
infra/runbooks/finite-private-ops.sh gate
infra/runbooks/finite-private-ops.sh stream-canary
infra/runbooks/finite-private-ops.sh responses-canary
infra/runbooks/finite-private-ops.sh mixed-version-canary
infra/runbooks/finite-private-ops.sh settlement-status "$FINITE_PRIVATE_LEDGER_SINCE"
```

Re-run canonical fleet status, check all pre-existing exceptions against their
before state, and prove the exact original tag/UUID/host is serving GLM. Retain
the canary's existing history and verify it remains readable after recovery.
Summarize actual downtime, startup failure/success, every measured tier,
first-token/decode tradeoffs, accounting, and restored state. Do not interpret
passing speed measurements as permission to leave DeepSeek serving.

## September 17 retry preparation

- The exact schema-2 DeepSeek pack completed on `control.inf9.tinfoil.sh`,
  job `rigfuoralxtfifii`, at **September 16 09:37:55 Central**. Preparation
  took about 15 minutes. Root hash, offset and verity UUID exactly match the
  existing measured candidate; no candidate/config release change is required.
- Provider modelwrap image:
  `ghcr.io/tinfoilsh/modelwrap@sha256:714595d190dcb141099b20d178f9cf6e5d16763be53eaba9f9e521167709978f`.
- Private receipt and reports are in `.local-state/deepseek-v41-20260917/`.
  The final live pack gate and read-only launch preflight both passed.
- The exact schema-1 GLM rollback pack passes the new live host gate.
- The candidate launch guard refuses incomplete/mismatched packs, changed
  serving identity, and early/late starts before it can stop GLM.
- The measured candidate remains byte-for-byte unchanged; its introductory
  September 16 comments describe the original preparation, not a new release.
- Benchmark execution now requires `--window-date`; UTC bounds derive from
  America/Chicago, including daylight saving time. Dry-run remains the default.
- Validation: 32 focused Python tests pass; dry-run confirms September 17
  UTC bounds, the 05:15 cutoff, and concurrency tiers through 128. The live
  launch dry-run first refused the incomplete job and then passed after the
  exact pack completed, without relaunching anything.
- Tinfoil's candidate-config validation returned `valid: true` for the exact
  release and container UUID. This is separate from host-pack verification.
- GLM remained `ready` on its original tag/host with unchanged sealed secret
  names and update settings. Health checks stayed HTTP 200 during preparation.
  Canonical status after preparation had green chat/recovery/rollout sections;
  existing fleet convergence red and app-host collection unknown remain
  explicit entry-review items.
- At preparation time, no timed execution was armed. The user subsequently
  requested a live sleep loop to 02:30; that execution is recorded below.

## September 17 execution result

### Entry and startup

The operator session waited in 30-minute intervals, with a shorter final wait,
and began read-only preflight at 02:30 Central. Fresh candidate and rollback
deployment/config hashes, GitHub attestations, image manifests, both live host
model-pack receipts, and exact serving identity passed. GLM authentication and
terminal streaming passed. Canonical fleet host detail and rollout state
matched preparation after removing heartbeat timestamps. Two launch batches
had reached their recorded expiry times; the canonical unexpired-only batch
listing omitted them, while their existing running placements were unchanged.
The existing version-convergence discrepancy and missing `nerdctl` collection
on app-only lat2 were carried explicitly, with no new unhealthy active Agents.

At 03:00, the fresh GLM baseline passed at concurrency 1 and 8. All 35 entry
and baseline requests settled with actual usage and no new reserved rows.
The guarded DeepSeek relaunch was accepted at **03:03:11**. CPU attestation,
all eight GPU attestations, model mounting, and the limiter passed boot checks.
The image pull took about five minutes. Candidate readiness was observed at
**03:34:03**, approximately 31 minutes after the relaunch request.

The unchanged measured candidate ran on the eight-H200 TDX host with its
DSpark-enabled configuration. This is runtime evidence for that configuration,
not a DSpark-on/off ablation or a measurement of speculative acceptance rates.

### Protocol and performance

Authenticated chat, terminal streaming, Responses API, and all four preserved
aliases passed, returning `deepseek-v4-1-flash`. All 12 protocol cases passed:
thinking off/high, forced tool, tool-result continuation, streaming tool,
parallel tools, JSON object, thinking-history handling, malformed JSON,
cancellation recovery, a 128,010-token prefill, and post-prefill recovery.
The protocol endpoint requires `/v1`; the command above now includes it.
The report's legacy reasoning-token counter reads a top-level usage field,
while this vLLM response nests that count under `completion_tokens_details`;
its reported zero is not evidence of zero reasoning. Separated reasoning was
checked directly. This limitation does not affect output-token throughput.

Each performance tier used three repetitions of the same short synthetic,
1,024-output-token, thinking-high workload. Decode and aggregate columns below
are medians across repetition results; TTFT is the worst repetition p95.

| Model/date | Concurrent clients | Decode tok/s per request | Aggregate tok/s | Worst p95 TTFT (s) | Result |
| --- | --- | --- | --- | --- | --- |
| GLM, September 17 | 1 | 62.427 | 60.984 | 0.393 | pass |
| GLM, September 17 | 8 | 62.131 | 478.506 | 0.768 | pass |
| DeepSeek, September 17 | 1 | 134.930 | 132.766 | 0.133 | pass |
| DeepSeek, September 17 | 8 | 106.303 | 766.381 | 1.606 | pass |
| DeepSeek, September 17 | 16 | 87.193 | 1253.068 | 0.631 | pass |
| DeepSeek, September 17 | 32 | 63.741 | 1820.100 | 0.827 | pass |
| DeepSeek, September 17 | 64 | 53.178 | 3058.300 | 0.938 | pass |
| DeepSeek, September 17 | 128 | 52.281 | 3128.020 | 22.502 | fail: TTFT |

All measured DeepSeek requests completed without errors and with terminal
streams. Concurrency 128 failed the 10-second p95 first-token threshold in all
three repetitions. The engine's 64-sequence limit means the second wave waits;
aggregate throughput rose only about 2.3% from 64 to 128 clients. The highest
fully passing tested tier is **64**, the same tested ceiling as GLM.

Against the fresh same-night baseline, DeepSeek single-stream decode was
**2.16×** GLM, and at eight clients decode was **1.71×**, with **1.60×** aggregate
throughput. Against the separately dated September 16 GLM 64-client result,
DeepSeek decode was about 17% higher and aggregate throughput about 8% higher.
That higher-concurrency comparison is cross-day evidence, not a matched
same-night measurement. GLM's own single-stream result varied from about 82
tok/s on September 16 to 62 tok/s on September 17. These live-endpoint tests
do not isolate production traffic, equalize tokenizers/reasoning quality,
establish sustained real-user capacity, or prove long-context concurrency.

### Restoration and closeout

The full sweep finished before 03:43, so GLM restoration began early rather
than waiting for 05:15. Rollback was accepted at **03:43:30**. GLM health first
returned HTTP 200 around 04:12, and the original release was observed `ready`
at **04:12:39**. By **04:13**, authentication, terminal chat streaming,
Responses API, older `glm-5-2` compatibility, and settlement checks passed.
The two initial readiness interruptions were approximately 31 and 29 minutes.
A closeout recheck at 04:15 then caught repeated five-second GLM upstream
health timeouts, while the usage API stayed healthy and an additional terminal
stream canary passed. No further restart was performed. Health recovered at
04:17:27, followed by four consecutive HTTP 200 samples from 04:18:44 through
04:21:03 and another successful authenticated gate. Final settlement and
canonical fleet checks completed by **04:22**. The intermittent health failures
are retained in the evidence; first readiness at 04:12 is not claimed as
uninterrupted health thereafter.

The exact original UUID, host, GLM tag, sealed-secret names, and update settings
were verified; no staged update or error remained. Canonical chat, recovery,
and rollout sections stayed green. Fleet convergence matched entry after
removing heartbeat timestamps; app-host collection remained the same known
limitation. No durable user state was rewritten. As on the first attempt,
persisted chat transcript replay was not independently exercised.

Final canary accounting recorded **828 settled requests**: 783 DeepSeek actual,
2 DeepSeek estimated, and 43 GLM actual. No new reservations remained; the 50
pre-existing reserved rows were unchanged. The aggregate settlement probe does
not attribute the two estimates to individual protocol cases, so this result
does not claim actual-usage settlement for every request.

Private evidence is under `.local-state/deepseek-v41-20260917/window/`, including
per-repetition JSONL, protocol report, release attestations, entry/recovery
status, poll timelines, and settlement reports. The live wait and maintenance
attempt are complete. **No later automatic model swap is armed.**

## September 16 preparation and execution record

- Private preparation evidence: `.local-state/deepseek-v41-20260916/` in the
  dedicated worktree. It contains fleet state and must not be committed.
- Candidate release: `v2026-09-16-deepseek-v4-1-flash-test-1`;
  measurement workflow: <https://github.com/finitecomputer/confidential-finite-private/actions/runs/35049160787>.
  Published successfully; deployment hash and GitHub attestation verified, and
  decoded configuration equals the reviewed candidate byte for byte.
  Deployment SHA256: `7cc6efab72885dcee0f2454c289805f5af6427fae8b5955b82c85906c370fc99`.
  Candidate config SHA256: `4caf0d4ed3ef3d43a8976202989c7ce173d77b8721d049e12ded46e83f979e89`.
- Window duration: confirmed by the user, 03:00–06:00 Central; restore by 05:15.
- Execution: the user requested this session remain awake with 15-minute
  sleeps. Preflight began at 03:00 Central on September 16; no scheduler was used.
- Runtime H200/TDX proof: **blocked before engine startup** by the missing
  host model pack; see the result below.

- Final preparation check: original GLM tag still `ready`, auto-update false,
  no update staged. Sixteen focused Python tests pass.

## September 16 execution result

Preflight began at 03:00 Central. Exact container identity, the original GLM
tag, sealed secret names, authentication, readiness, terminal streaming,
release attestations, candidate config bytes, and rollback image availability
passed. Fleet differences were heartbeat timestamps only: after excluding
`reported_at` and `age_seconds`, host detail matched the preparation snapshot.
The unchanged version-convergence discrepancy is outside this inference-only
swap, and the unknown container collection is from app-only lat2 lacking
`nerdctl`. Chat, recovery, and rollout status remained green.

GLM completed three repetitions at each of 1, 8, 16, 32, and 64 requests.
The pre-swap baseline deadline guard skipped 128; that tier was completed
after GLM recovery as described below. All 380 pre-swap baseline/canary requests
settled; zero new reservations remained. Fifty old reservations predated the
window and were not modified.

| Concurrent requests | Median of per-repetition median output tok/s | Median aggregate tok/s | Worst repetition p95 TTFT (s) |
| --- | --- | --- | --- |
| 1 | 81.72 | 79.33 | 0.378 |
| 8 | 72.20 | 553.32 | 0.626 |
| 16 | 65.78 | 1011.37 | 0.650 |
| 32 | 52.50 | 1623.06 | 0.750 |
| 64 | 45.56 | 2820.83 | 0.977 |

All measured tiers passed without request errors. These short synthetic,
1,024-output-token, thinking-high results do not establish long-context or
real production workload capacity.

The candidate relaunch was accepted at 03:07:14 Central. By 03:08:24 the
container was `failed`: the pinned model pack did not exist on the selected
host. The control plane explicitly required downloading the model before
deploying. The engine never started, so this provides no DeepSeek throughput,
DSpark acceptance, or H200 kernel compatibility result.

Rollback to `v2026-08-28-glm-5-3-flash-5` was requested immediately and accepted.
The first readiness success was observed around 03:36 Central, followed by a
transient upstream-health timeout. By 03:39, the full gate, terminal streaming,
Responses API, older `glm-5-2` alias compatibility, and settlement checks passed.
The conservative disruption interval was approximately 03:07–03:39 (32 minutes).
Canonical fleet status after restoration had green chat, recovery, and rollout
sections. Fleet convergence matched entry after excluding heartbeat timestamps;
the same app-host collection limitation remained. No durable user state was
rewritten. This window did not independently replay a persisted chat transcript;
canonical chat/recovery probes and API compatibility were the recovery evidence.

The requested 128-request tier then ran on restored GLM, with the same workload
and three repetitions, under a 300-second process timeout. It returned no
request errors but failed the 10-second p95 first-token gate:

| Repetition at 128 | Median output tok/s per request | Aggregate tok/s | p95 TTFT (s) |
| --- | --- | --- | --- |
| 1 | 42.098 | 5125.607 | 1.227 |
| 2 | 42.181 | 3300.354 | 15.455 |
| 3 | 26.103 | 3230.807 | 1.584 |

Thus 64 was the highest fully passing tier tested. The 128 results were
collected after a restart and should be labeled separately from the pre-swap
baseline. They do not establish a DeepSeek comparison.

At 03:42 Central the final health and settlement checks passed. All 773 window
requests were settled with actual usage, zero new reservations remained, and
the 50 pre-existing reservations were unchanged. The exact original container
UUID, host, and GLM release were `ready`, with no pending update or error.
The maintenance attempt is complete; no later automatic swap is armed.

### Required before another attempt

Stage `deepseek-ai/DeepSeek-V4.1-Flash` revision
`dba1be0a40aa45a94ad051997016db3960a90277` on the exact Tinfoil host and verify
its MPK against the candidate descriptor. The observed missing host path was
`/mnt/large/tinfoil/models/deepseek-ai/DeepSeek-V4.1-Flash/dba1be0a40aa45a94ad051997016db3960a90277.mpk`.
A measured release proves the configuration, not host-local model availability.
Add explicit provider-side confirmation of both model packs to future entry
checks before stopping the serving model. No second candidate attempt is
planned during this window.
