# OpenRouter to Finite Private: investigation notes

Recorded September 4, 2026 from the read-only fleet investigation that day.
This is a snapshot for discussion, not an approved migration plan or executable
runbook. The draft PR can remain unmerged as a record of these findings. It
does not authorize production changes, a deployment, or a migration tonight.

## Findings

The investigation found 41 running bots configured to use OpenRouter for their
main chat model:

| Host | OpenRouter main-chat bots |
| --- | ---: |
| box1 | 26 |
| TRF | 12 |
| smoke | 3 |
| lat3 / lat4 | 0 |
| Total | 41 |

All 53 active hosted agents on lat3/lat4 had Finite Private saved as their main
route. Another 10 active bots explicitly referenced OpenRouter for auxiliary
tasks or fallback despite having a different main provider. The reported scope
is therefore **at least 51 bots needing attention** for a complete OpenRouter
exit. Saved configuration is not proof of the provider used by every request;
automatic selection and session overrides remain unverified.

Of the 41 main-chat bots, 23 had credentials matching active Finite Private
keys. The remaining 18 lacked a Finite Private key in the checked environment
files, including mounted shared environments: 3 on box1, 12 on TRF, and 3 on
smoke. This does not establish that those accounts lack an entitlement or that
no credential exists elsewhere.

Existing saved provider settings survive reconciliation. Changing a shared
default alone would not switch all existing bots. A main-chat switch would
also change Claude/GPT conversations to GLM-5.3-Flash on Finite Private; it is
a model change as well as an endpoint change.

## Unresolved checks

- **Routing completeness:** inventory main, auxiliary, delegation, fallback,
  automatic provider selection, session overrides, and launch/recreation
  defaults. Several bots explicitly use OpenRouter for vision; a compatible
  private replacement has not been demonstrated.
- **Capacity:** Finite Private was healthy at the time of inspection, but the
  recorded load tests did not establish the planned 120-user capacity gate.
  Bot count alone does not establish simultaneous demand or available headroom.
- **Accounting:** the investigation reported 311 usage reservations older than
  15 minutes awaiting settlement. Their cause and impact remain unexplained;
  this observation is not a diagnosis or authority to alter accounting rows.
- **Real conversations:** gateway state and successful inference calls do not
  prove replies in users' actual chats. Text replies, tool calls, image handling,
  and continuing existing conversations need end-to-end verification.

## What a separately authorized migration would need

1. Refresh the inventory and resolve the unknown routes and credential gaps.
   Use `scripts/finite-status` for platform/fleet status; add any missing probe
   there instead of retaining the temporary investigation script as an operator
   command.
2. Trace each configuration writer and reader through launch, reconciliation,
   session selection, and gateway execution. Prove the affected existing-state
   and mixed-version behavior on synthetic state before production changes.
3. Name and verify the backup and rollback boundary for each batch, preserving
   durable history, identity, and recovery data. Keep stopped legacy copies
   stopped so duplicate channel consumers do not compete for messages.
4. Establish capacity and settlement evidence, then switch small batches with
   gateway restarts where required and explicit stop/rollback criteria. Run
   `scripts/finite-status` before and after each rollout and verify actual chat
   behavior before expanding.

The initial estimate was several hours for a staged main-chat switch, with
same-night completion conditional on these checks passing. A complete exit
from OpenRouter is broader and remains unproven. No schedule is committed.

## Evidence and limitations

The investigation read saved Hermes configuration and environment files on
box1, TRF, smoke, lat3, and lat4, checked active runtime/key records, and reviewed
fleet status and endpoint health. Local temporary evidence was retained as
`/tmp/finite-provider-inventory-{0..4}.jsonl`,
`/tmp/finite-provider-core.jsonl`, and
`/tmp/finite-private-fleet-status.json`. These files are ephemeral and are not
part of this PR. Runtime directories include inactive state; the 53-agent
figure used active runtime records, not a count of directories.

The notes preserve the earlier investigation; production was not queried again
when preparing this PR. Counts can change and are not a current rollout gate.
No credentials, user identifiers, raw environment files, production edits,
credential issuance, or service restarts are included.

Related repository context:

- [Runner Finite Private route](../../infra/runbooks/runner-finite-private-route.md)
- [Finite Private routing migration](../../infra/runbooks/finite-private-routing-migration.md)
- [GLM-5.3-Flash cutover gates](../../infra/runbooks/finite-private-glm-5.3-flash-production-cutover.md)
- [Historical degraded-admission and load-test evidence](../runs/glm-5-3-flash-degraded-admission.md)

The historical degraded-admission record is context, not evidence that its
temporary mode was still active on September 4.

## Follow-up: blast radius and full-load qualification

This follow-up adds read-only Tinfoil inspection on September 4 Central
(September 5 UTC). No inference load, production edits, or provisioning was
performed. The earlier fleet inventory was reanalyzed, not recollected.

### Shared exposure

Joining runtime directories to both active Runtime IDs and their assigned
hosts avoids counting stale copies on another host. The saved inventory has
66 existing Finite Private main-chat consumers: 13 on box1, 31 on lat3, and 22
on lat4. Adding the 41 OpenRouter main-chat bots gives **107 main-chat
consumers**, about 62% more than the existing 66. This is a configured-instance
count, not unique people, simultaneous requests, or a measured traffic increase.
The 10 auxiliary/fallback users overlap this population; they must not simply
be added again. Other API clients and parallel subagents are not bounded by
this inventory.

| Boundary | Potential effect |
| --- | --- |
| Individual bot configuration and restart | Failed authentication, lost in-flight reply, changed model behavior, broken tools or vision; at least 51 bots require settings review |
| Shared GLM scheduler and GPU allocation | Queueing, slow first tokens, timeouts, or model unavailability can affect all existing and newly migrated Finite Private consumers |
| Limiter, Core admission, and Postgres settlement | Every admitted call depends on the accounting path; GPU health alone cannot establish end-to-end capacity |
| Shared launch defaults | Incorrect defaults can affect later launches and recreations, including onboarding |
| Model-container replacement | Service-wide inference downtime; separate from changing one bot's endpoint |

An endpoint/configuration switch does not require moving chat databases or
identities. Preserve those boundaries and keep stopped copies stopped. A
gateway rollback restores that bot's previous route; replacing the shared
model is a different, broader rollback.

### What is verified now

- Tinfoil lists one container in the organization: `finite-private`, ready on
  `control.inf9.tinfoil.sh`, with eight H200s, 32 CPUs, and 524,288 MiB RAM.
  It serves `v2026-08-28-glm-5-3-flash-5`. The released configuration was
  fetched from the satellite tag and is byte-identical to the checked-in
  [candidate configuration](../../infra/tinfoil/confidential-finite-private/tinfoil-config.glm-5.3-flash.candidate.yml).
  TP8/EP8 uses the eight GPUs together for one model-serving deployment; this
  is not eight independently redundant replicas.
- The live `/health` response reports GLM, thinking enabled with high reasoning
  effort by default, `usage-api` admission, and healthy upstream and usage API.
- The 24-hour metrics response contains 30 buckets spanning
  `2026-09-04T03:55Z` to `2026-09-05T03:07Z`. The arithmetic mean of reported
  average GPU-utilization buckets is 17.8%; the highest average bucket is 64%,
  and the highest reported maximum is 96%. GPU memory utilization is 90% in
  every bucket. These coarse metrics do not establish request capacity or
  free KV-cache capacity. Preallocated model/cache memory also prevents
  interpreting 90% memory use as 90% of request capacity consumed.
- The accessible host list contains only `control.inf9.tinfoil.sh`, allowing
  an eight-GPU allocation. This is host eligibility, not proof of a spare
  eight-H200 allocation. No standby container appeared in the organization.

Resource metrics are retrieved with Tinfoil's documented
[read-only metrics command](https://docs.tinfoil.sh/containers/cli#resource-metrics).
Its [lifecycle documentation](https://docs.tinfoil.sh/containers/cli#managing-in-progress-updates)
also states that multi-GPU relaunches have downtime and replace the current
deployment. The deployment changelog records roughly 29-minute GLM reloads;
do not treat shared-model rollback as instantaneous.

### Required proof before calling the full fleet ready

The retained GLM tests are warning signs, not a fresh benchmark of flash-5.
Flash-3 at 32 concurrent requests with high thinking recorded 33.8-second p95
time to first token and 215 aggregate output tok/s. Flash-4's short,
thinking-off 32-way test recorded 15.13-second median time to first byte and
128.5 aggregate tok/s. Different token budgets and concurrency make these
diagnostic results unsuitable for extrapolating 120-way throughput. No passing
120-way GLM run or matching soak was found in the inspected retained evidence.
See the [historical measurements](../runs/glm-5-3-flash-degraded-admission.md)
and local `capacity-flash3-1-32-thinking-on.jsonl` and
`load-canary-flash4-32.log` under
`.local-state/glm53-cutover-2026-08-28-attempt2/`.

**Capacity remains unproven. Do not approve an all-fleet switch on the existing
evidence.** A useful proof needs a defined workload and pass/fail envelope:

1. Measure aggregate current and incoming peak request rate, in-flight
   requests, prompt/context lengths, output and reasoning lengths, tool-loop
   amplification, auxiliary calls, retries, and scheduled bursts. Use metadata
   and synthetic representative prompts without exporting private chats.
   Include other API clients and establish per-agent parallelism bounds.
   The limiter's current [`/metrics` implementation](../../finitecomputer-v2/crates/finite-private-limiter/src/lib.rs)
   exports only liveness, so the metrics inspected here cannot supply this
   demand profile. Missing fleet probes belong in `scripts/finite-status`.
2. Qualify the exact model, GPU type/count, serving configuration, limiter, and
   admission/settlement path on separately authorized isolated capacity. Test
   multiple synthetic accounts/grants and shared-grant contention. Isolation
   must cover admission/accounting load as well as GPUs; a second GPU endpoint
   alone does not isolate a shared production Core database.
3. Pass the existing 120-concurrent-request gate three times and the documented
   35-minute soak: 120/120 terminal streams, zero errors, p50 decode at least
   20 tok/s, p10 at least 10 tok/s, aggregate at least 2,400 tok/s, and p95 time
   to first token at most 10 seconds. The
   [capacity checker](../../scripts/check_finite_private_glm53_capacity.py)
   is a short-prompt synthetic burst, not a complete fleet simulation.
4. Additionally test realistic long histories, cold/warm caches, high thinking,
   long answers, tool loops, auxiliary traffic, synchronized bursts, and
   sustained arrivals. Measure time to useful visible text and complete replies,
   queue growth/drain, errors, memory pressure, and settlement completion.
   Reasoning-token speed does not establish useful-answer latency or quality.
   Verify image handling and existing conversations through representative
   gateway versions. Preserve existing-user latency under the added load.
5. Establish headroom beyond the measured combined peak. A proposed 50% burst
   margin is a planning target, not an existing acceptance contract; 107 single
   calls would round up to 161 simultaneous calls at that margin. Actual
   concurrency may be higher once parallel agents and other clients are counted.
   Record overload/retry behavior and recovery after the burst; do not lower
   the existing acceptance thresholds to fit a failed result.

Spare capacity and its cost/availability need confirmation before an isolated
test can be scheduled. If qualification fails, investigate a separately
measured serving configuration or additional replicas, then repeat the gates.
Fleet migration and any production load testing remain separately authorized
work. No guarantee of full-load readiness is made by this note.

### Existing high-concurrency evidence to reuse

We do have strong historical eight-H200 load evidence for **DeepSeek V4 Flash
0731**, the previous model. The
[August 7 isolated optimization record](2026-08-07-deepseek-v4-eight-h200-optimization.md)
reports 1,024/1,024 successful short-prompt, 128-output-token reasoning requests,
8,373.44 aggregate output tok/s, and 4.045-second p95 time to first token.
A longer 1,024-output-token run also completed 1,024/1,024 with zero errors,
10,756.55 aggregate tok/s, and 3.944-second p95 time to first token. That run
lasted 97.483 seconds; the record explicitly distinguishes it from the
35-minute stability gate and from concurrent maximum-length contexts.

This is valuable evidence for that hardware class and exact DeepSeek/vLLM
recipe, and a baseline to reuse. It does not qualify the current GLM/SGLang
recipe, whose model, cache layout, and parallelism differ. The
[GLM cutover ledger](../runs/glm-5-3-flash-production-cutover-ledger.md)
explicitly records unmet capacity thresholds for flash-4. Its statements about
missing usage admission were superseded by flash-5; do not read the whole
ledger as current operational status.

A follow-up checked these records and GLM PRs #721, #747, and #748 without
finding newer full-load qualification. The available `fbrain` identity could
reach the server but listed no accessible Brains, so organization Brain
content was not searched successfully. This remains a search limitation, not
proof that no other record exists.

### Raw GLM result location and meaning of "passed"

The raw GLM records were located in the separate sibling worktree
`finite-mono-glm-5-3-flash-cutover/.local-state/glm53-cutover-2026-08-28-attempt2/`,
not this checkout's `.local-state`. Direct inspection confirms:

- `capacity-flash3-1-32-thinking-on.jsonl`: 32/32 successful terminal streams,
  zero errors, 59.448 median decode tok/s per request, 215.096 aggregate
  tok/s, and 33.772-second p95 time to first token.
- That file reports `passed: true` against explicitly overridden diagnostic
  thresholds: concurrency 32, one repetition, maximum p95 first-token time
  120 seconds, and minimum aggregate/p10/p50 throughput each 1 tok/s.
  It is a real passing diagnostic result, not a pass of the documented
  120-request production acceptance gate.
- `load-canary-64.log`: 64 requests, 64 completion tokens per request,
  69.606 median decode tok/s per request, 237.393 aggregate tok/s, and
  16.297-second p95 time to first byte.
- `load-canary-flash4-32.log`: 32 requests, 64 completion tokens per request,
  81.928 median decode tok/s per request, 128.526 aggregate tok/s, and
  15.132-second p95 time to first byte.

These are recorded GLM-5.3-Flash load results and should be reused. The open
question is qualification of the combined workload, not whether GLM was ever
load-tested.
