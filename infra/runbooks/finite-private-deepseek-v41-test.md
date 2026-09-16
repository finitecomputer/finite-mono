# DeepSeek V4.1 Flash: September 16 H200 maintenance test

Preparation for **2026-09-16 03:00 America/Chicago (CDT, UTC−05:00)**,
which is **08:00 UTC**. This is a temporary A/B test with GLM restoration,
not a permanent model promotion. No production mutation or inference load was
performed during preparation. A future start is not evidence that a scheduler
has been armed; see the execution record below.

## Window and recovery

Proposed two-hour window, pending the operator's duration preference:

| Central | Action |
| --- | --- |
| 03:00–03:10 | Fresh status, GLM protocol canary and bounded baseline |
| By 03:10 | Start the measured DeepSeek candidate only if all entry checks pass |
| By 03:50 | Candidate must answer authenticated requests; otherwise restore GLM |
| 03:50–04:15 | Protocol, model aliases, bounded performance tiers |
| **04:15** | **Stop tests and start GLM restoration even if DeepSeek passes** |
| **05:00** | GLM healthy; final status and accounting evidence retained |

The 45-minute restoration reserve is based on prior roughly 35-minute model
loads, not a guaranteed recovery time. If GLM is not healthy by 05:00, treat it
as an incident and continue recovery. Do not extend testing into that reserve.
A late start shortens testing; it never moves the end of the window. No new
candidate start after 03:10. No second engine/config attempt within this window.

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
does not prove startup or performance on our CVM. A CUDA assert, OOM, repeated
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

1. Confirm the duration/rollback cutoff and an operator who will watch the
   entire window. Do not arm unattended production replacement with only this
   document as a timer.
2. Publish the candidate from a dedicated satellite branch/tag, after its source
   is committed in mono. Use the existing measured release workflow with
   `mark-as-latest: false`; leave satellite `main`, the latest stable release,
   and the live container unchanged. Record the candidate tag and workflow.
3. Download its `tinfoil-deployment.json` and `tinfoil.hash`. Verify the hash,
   verify the GitHub attestation, and require the decoded `config` bytes to
   equal the reviewed candidate exactly. A tag or image pin alone is not this
   proof. Confirm the config has the existing limiter, secrets by NAME,
   internal model network, and wildcard shim routing.
4. Re-fetch the GLM rollback assets and check the recorded digests. Confirm
   their image/model artifacts remain available. Retain both release bundles
   locally before stopping anything.
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
   before starting. Any new/worsened finding, or uncertain scope, stops entry.
8. Source the canary credential privately. Run limiter readiness, negative
   authentication, one stream, and accounting settlement checks. A bad baseline
   means no model swap. Configure `FINITE_PRIVATE_CORE_HOST=root@64.34.80.19`.

## Measurement commands

Use the same checkout and exact release tags for both models. Run project
Python commands through `scripts/with-dev-env`. The benchmark driver defaults
to dry-run; `--execute` sends inference requests but never deploys a model.
It refuses execution before 03:00, verifies the live release before each tier,
requires stream model identity, bounds each tier by the supplied deadline,
and refuses to overwrite evidence.

```bash
export FINITE_PRIVATE_CANARY_ENV_FILE=/Users/plebdev/Desktop/Projects/finite-mono/secrets/finite-private-canary.env
set -a
source "$FINITE_PRIVATE_CANARY_ENV_FILE"
set +a
export FINITE_PRIVATE_CORE_HOST=root@64.34.80.19

# GLM baseline: deadline 03:10 Central. Omit --execute to inspect the plan.
scripts/with-dev-env python3 scripts/finite_private_v41_benchmark.py \
  --model glm --tag v2026-08-28-glm-5-3-flash-5 \
  --deadline-utc 2026-09-16T08:10:00Z \
  --evidence-dir "$FINITE_PRIVATE_EVIDENCE_DIR/baseline" --execute

# Candidate: deadline 04:15 Central under the proposed two-hour window.
scripts/with-dev-env python3 scripts/finite_private_v41_benchmark.py \
  --model deepseek --tag "$FINITE_PRIVATE_V41_TAG" \
  --deadline-utc 2026-09-16T09:15:00Z \
  --evidence-dir "$FINITE_PRIVATE_EVIDENCE_DIR/deepseek" --execute
```

The tiers are 1, 8, 16, 32, 64; three repetitions each, 1,024 output tokens,
thinking on/high, one bounded warmup, and distinct synthetic short prompts.
Report decode p10/p50, aggregate output, TTFT p50/p95, and errors separately.
Stop escalation on any error, p10 below 10 tok/s, p50 below 20 tok/s, or p95
TTFT above 10 seconds. Aggregate throughput is measured without a claimed
120-user acceptance floor. A failed speed tier marks the capacity limit;
protocol/auth/accounting failure triggers immediate rollback. This test does
not establish long-context concurrency, image performance, or equal reasoning
quality between models' `high` settings.

Before the DeepSeek sweep, use `check_finite_private_glm53_protocol.py --model
deepseek-v4-1-flash --endpoint "$FINITE_PRIVATE_ENDPOINT" --max-context-tokens
128000 --timeout-seconds 90` for reasoning, tools, terminal streams, malformed
requests, and a long-prefill recovery probe. Run it with a total process
deadline before 04:15; an HTTP timeout alone is not a whole-suite deadline.
Its historical script/schema name is retained, but the report's model field
and response checks must be DeepSeek. Also run one stream for each preserved
model alias, expecting `deepseek-v4-1-flash` in the response, and settlement
status. Do not skip compatibility because the canonical model passed.

## Swap and unconditional restoration

Only after entry checks pass, use the existing guarded relaunch command with
the measured candidate tag. It keeps the existing container identity and route:

```bash
export FINITE_PRIVATE_CONTAINER=finite-private
export FINITE_PRIVATE_ENDPOINT=https://finite-private.finite.containers.tinfoil.dev
export FINITE_PRIVATE_RELAUNCH_APPROVED="$FINITE_PRIVATE_V41_TAG"
infra/runbooks/finite-private-ops.sh relaunch "$FINITE_PRIVATE_V41_TAG"
```

Do not use `create --replace`, rename the container, or delete it for a routine
test. If a control-plane command times out, re-read the exact UUID and current
tag/update state before retrying. Do not assume the mutation failed. The
window operator must watch readiness independently of the 120-minute
healthcheck start period in the upstream config.

At the first rollback condition or at 04:15, whichever comes first:

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

## Execution record

- Private preparation evidence: `.local-state/deepseek-v41-20260916/` in the
  dedicated worktree. It contains fleet state and must not be committed.
- Candidate release: pending measurement/publication.
- Window duration: preference requested; two-hour schedule above is provisional.
- Scheduling: **not armed**. No scheduled production operation exists yet.
- Runtime H200/TDX proof: pending the maintenance test.
