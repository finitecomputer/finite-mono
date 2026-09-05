# Box1 OpenRouter to Finite Private: migration plan and evidence

Recorded September 4, 2026 from the read-only fleet investigation that day.
This is a snapshot for discussion, not an approved migration plan or executable
runbook. The draft PR can remain unmerged as a record of these findings. It
does not authorize production changes, a deployment, or a migration tonight.

## Current planning decision: box1 main-chat migration

The user has confirmed concurrency was proved in another thread and asked to
cement the box1 migration plan. Accept that as the planning premise; do not
repeat the capacity investigation as a prerequisite to drafting this plan.
The executed GLM sanity check at the end of this document is linked evidence
available here. Attach any additional result from the other thread to the
execution handoff without inventing its workload or broadening what the
recorded 32-way result proves. Earlier capacity warnings below describe the
investigation at that time; they are not the current box1 planning decision.

**Proposed outcome:** each eligible, actively served box1 OpenRouter Agent
continues its existing conversations using GLM-5.3-Flash through Finite Private,
with working tools, correct account charging, and the same history, Device
identity, channels, host, PVC, and runtime image. This is a main-conversation
provider/model switch. Host migration, runtime upgrades, shared inference
deployment changes, and a complete OpenRouter exit are separate work.

This section is a proposed plan, not an executable or approved production
runbook. All execution steps remain TODO. Preparing and reviewing the bounded
runbook and its evidence comes before requesting production authorization.

### Scope from the saved September 4 inventory

| Box1 population / attribute | Count | Planning consequence |
| --- | ---: | --- |
| Running OpenRouter main-chat Agents | 26 | Initial candidate set; refresh before selecting a batch |
| Matching active Finite Private key and grant | 23 | Potential first cohort; key ownership and effective environment precedence still need verification |
| No matching credential in checked files | 3 | Separate credential-preparation cohort; do not borrow a shared or another account's key |
| Claude Sonnet 4.6 / Claude Opus 4.6 defaults | 25 / 1 | Explicitly record model behavior change; include the Opus case in compatibility review |
| Candidates with a recorded Telegram credential | 19 | Cross-host channel-consumer ownership check required |
| Candidates with explicit OpenRouter delegation | 2 | Record as retained exceptions or hold for separate delegation work |
| Candidates with auxiliary configuration | 26 | Resolve `auto` and explicit routes against the deployed Hermes version |

All 26 saved candidates referenced `fc-agent-runtime:main-015rdv4dc6hz`.
A shared tag is not digest or behavior proof. The inventory parsed saved
configuration, not every session's effective route; it found no explicit
fallback block in these 26. Another nine running box1 Agents outside this
cohort had explicit OpenRouter references and belong to a later exit audit.
Counts are Agent instances, not unique accounts. TRF, smoke, and lat3/lat4
Agents are outside the mutation scope, but active replacements and shared
service health must be checked across hosts.

### TODO 1: freeze the eligible roster and exact change surface

Prepare a protected local execution manifest from fresh read-only evidence.
For each candidate, record:

- Account/Project attribution where applicable, namespace, StatefulSet UID and
  resource version, desired/ready replicas, pod UID, image digest, Hermes
  version, PVC UID/path, and authoritative durable home.
- Config hash, model block, session override locations, environment sources
  and precedence, key record/grant/profile identifiers and status, allowance,
  and whether the effective key belongs to the intended account/Project.
  Record secret locations and presence only; never publish key values.
- Active channel owners on every relevant host, stopped/replacement copies,
  and any unresolved externally managed consumer. Use protected token
  fingerprints for equality checks without exposing tokens. A newer creation
  timestamp, selected row, or matching name does not choose the authority.
- Current main, auxiliary/title/vision, delegation, fallback, scheduled-job,
  and session routes; record which will change implicitly when `auto` follows
  the main model. Exclude a candidate if a needed capability would regress.
- Named config rollback files, a verified Recovery Set and empty-target restore
  evidence for the durable state, existing health findings, and batch assignment.

An earlier Iherbs attempt demonstrated the ownership failure: starting its
legacy box1 copy caused Telegram polling conflicts with its active lat3
replacement. The operator restored the original box1 config and zero-replica
state; the replacement already used Finite Private. Keep that retired source
excluded. This lesson comes from the local Iherbs investigation retained in
the main worktree (`infra/runbooks/iherbs-finite-private.md`, untracked at plan
preparation) and must be rechecked against live state for any candidate.

Use `scripts/finite-status --json` as the platform/fleet status authority.
The inspected command lacks the complete box1 provider/credential/channel
inventory required here. Implement missing read-only probes there, with
unknown/ambiguous results that block selection; do not promote the temporary
inventory script into another operator status command. Saved rows alone do
not constitute the execution roster.

### TODO 2: prove the configuration path and prepare rollback

The production through-line to verify against each deployed digest is:

| Writer / authority | Reader / effect | Required proof |
| --- | --- | --- |
| Per-Agent mounted environment sources and legacy entrypoint | Generated durable `.hermes/.env`, then gateway child | Correct key wins after startup and ordinary recreation; editing only a generated file is insufficient |
| Operator's guarded edit of durable `.hermes/config.yaml`; Hermes/user settings writers | Entrypoint reconciliation and deployed Hermes model loader | Edit only the approved model fields; reject stale hashes, duplicate keys, unexpected shapes, and concurrent edits |
| Saved session/model overrides and auxiliary/delegation configuration | Hermes request selection for each request class | Existing conversations reach the intended route; preserved exceptions remain explicit |
| Existing gateway and channel adapters | Finite Chat and external channel services | Exactly one authorized consumer; prior conversation and Device identity continue |
| Existing Finite Private limiter and Core key/grant records | Admission, GLM inference, and Core settlement | Correct account attribution, successful terminal response, actual usage settled |
| Hermes and chat services | Durable session/message stores and transcripts | Original IDs/content preserved; new replies append without resetting history |

Current mono reconciliation deliberately preserves user model selections
outside narrow Finite-owned migrations. A shared default is therefore not an
existing-Agent migration. Nor does testing current mono prove legacy box1
behavior. Inspect the entrypoint/environment composition and Hermes readers
from the actual deployed image, including user model commands, session state,
cron requests, and restart/recreation behavior.

Prepare this intended model mapping, subject to exact-image validation:

```yaml
model:
  default: glm-5-3-flash
  provider: custom
  base_url: https://finite-private.finite.containers.tinfoil.dev/v1
  api_mode: chat_completions
  context_length: 393216
  api_key: ${FINITE_PRIVATE_API_KEY}
```

Prove on synthetic state using the deployed legacy image and current service
contract: existing Claude histories with tool messages; resumed session
selection; harmless tool round trip; long-history compaction within the
configured limit; auxiliary/title/vision behavior; and exact configuration
rollback after startup. Run the gateway rehearsal with external adapters and
schedulers fenced so it cannot become a second live consumer. A successful
HTTP request alone is insufficient. Tests requiring production inference or
external messages belong to the later explicitly authorized execution.

For the three credential gaps, first resolve intended account/grant and the
established credential-management path. Prepare the exact issuance or delivery
action for review, including every durable input needed to survive startup.
Do not issue keys, change allowance, or edit shared secrets in planning. A
matched active key alone does not establish ownership or sufficient allowance.

Retain per-Agent root-only backups outside the writable home of every file
that will change, including ownership/mode and hashes. Verify the existing
Recovery Set can restore onto an empty target; a config backup is not a chat
backup. Inspect snapshot SQLite only via `scripts/snapshot-sqlite` or a scratch
copy. Preserve Recovery Authorities and keep recovery material after rollout.

### TODO 3: execute staged batches after explicit authorization

Proposed batching, conditional on the refreshed roster: **1 canary → 3 → 7 →
remaining ready Agents** (12 if all 23 credential-matched candidates qualify).
Choose the canary by verified ownership and representative legacy behavior,
not by row order; do not reuse retired Iherbs. Handle the three credential-gap
Agents afterward as **1 → remaining 2**, once their prerequisites pass.
Any excluded Agent stays pending with a named reason, rather than being
silently counted as migrated. The execution handoff names the exact canary
and batches, maximum pause, and stop/rollback authority.

For each Agent, sequentially within each batch:

1. Capture canonical status and the expected source identity/config versions.
   Recheck absence of a conflicting rollout or channel owner. Block on new or
   unexplained chat/recovery/admission failures; retain and explain unrelated
   baseline findings rather than calling a red status green.
2. Wait for in-flight conversation/tool work to complete. Use the proven
   Agent-local lifecycle operation to quiesce its gateway; do not pause box1
   or shared services. If work cannot drain within the agreed window, defer
   that Agent. Record replica/image/PVC state and preserve inbound work.
3. Check the source hash again and atomically apply only the reviewed config
   and, where required, per-Agent credential-source change, preserving file
   owner/mode. Refuse changes by another writer. Resume only the exact fenced
   Agent using the same image, PVC, and identity.
4. Verify effective route/model/key, useful visible reply, one harmless tool
   round trip, continuation of synthetic prior history, channel connectivity,
   and completed actual usage settlement. Verify existing history IDs/content
   on a consistent scratch snapshot and confirm no duplicate consumers or
   unrelated pod restarts. An authorized channel canary is required before
   calling that channel verified; connection status alone is insufficient.
5. Capture canonical status again. Observe at least 30 minutes after the
   first canary and 15 minutes after each later batch, with no new auth/429/5xx,
   stuck test reservations, polling conflicts, or chat regressions. These are
   proposed minimum observation windows, not statistical capacity proof;
   sparse traffic requires explicit verification. Keep a 24-hour follow-up
   after the final batch for scheduled behavior and user-reported regressions.

Proposed diagnostic stop threshold: a synthetic chat canary fails to produce
useful visible text within 30 seconds or a complete bounded reply within 120
seconds; the harmless tool canary fails to finish within 120 seconds; or a
canary reservation remains unsettled after 15 minutes. Confirm these thresholds
in exact-image rehearsal and record them before authorization. First reasoning
output is not useful visible text. Also stop on any new history/identity
mismatch, account misattribution, restart loop, duplicate consumer, or shared
service degradation. Do not retry repeatedly or loosen thresholds mid-rollout.

### TODO 4: rollback and completion

On a per-Agent failure, stop expansion and quiesce only that Agent. Restore
the exact backed-up config and any changed per-Agent environment input using
guarded atomic replacement, then restore its pre-change lifecycle state.
Verify the prior provider, continued conversation, and canonical status.
Keep the original OpenRouter credential valid during the observation window.
Do not restore the whole home/PVC to undo routing: that would discard new
messages. If a concurrent user edit makes rollback ambiguous, stop for
reconciliation instead of overwriting it. A newly issued key remains recorded
for later cleanup until its exclusive ownership/use is established.

If degradation is shared, halt all batches and assess rollback of the changed
cohort; per-Agent rollback may reduce added demand but does not guarantee
recovery of a failing shared service. Do not restart or replace Tinfoil as an
automatic migration rollback. That is a separately authorized incident action.

Completion evidence must account for every refreshed candidate as migrated,
rolled back, or held; show correct account settlement and effective main-chat
routing; preserve history/identity; and pass the observation windows. Record
retained auxiliary/delegation/vision OpenRouter use explicitly. Main-chat
completion does not authorize deleting OpenRouter keys or claiming all
inference is private. New-account defaults and box1-to-lat3 moves remain
separate; verify ordinary recreation of the migrated Agent preserves its
selection as part of rehearsal.

**Next deliverable:** a reviewed box1 execution runbook with the fresh private
roster, canonical inventory probes, exact-image rehearsal results, credential
actions, recovery references, and named first batch. Request authorization
only once that concrete packet is ready. No production action was taken while
preparing this plan.

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

## Prepared first concurrency sanity check

The next proposed step is a small diagnostic against the current GLM-5.3-Flash
service, before considering a provider switch. At preparation time, production
inference traffic was pending authorization. The user subsequently authorized
the live test through 32 concurrent requests with normal users still able to
use the service. Results are recorded below. This section does not change the
full-fleet acceptance requirements above.

### Bounded workload

- Use the existing capacity checker through the public Finite Private limiter
  and normal `usage-api` admission, with an existing dedicated canary key.
  Do not use a customer's key, issue a key, bypass accounting, or raise limits.
- Run **1, 4, 8, 16, then 32** concurrent requests, one tier at a time. Review
  each result and recovery before manually starting the next tier. No automatic
  sweep, retries, or warmups. Hold 64/120-way tests for a later decision.
- Use synthetic short prompts, `glm-5-3-flash`, thinking on, reasoning effort
  high, and a 256-token completion budget per request. The checker requests
  `ignore_eos` to exercise the budget; it is not a useful-answer quality test.
- After each successful tier above 1, allow at least 60 seconds without test
  traffic, confirm readiness, then run one single-request recovery probe using
  the same checker/settings. Compare it with the initial single-request result.
  Wait for settlement before advancing.
- The complete first stage is **65 requests maximum**: 61 tier requests plus
  four recovery probes, requesting at most **16,640 completion tokens** if the
  server honors the per-request cap. Prompt tokens and normal service charges
  are additional. These calls create usage/accounting state but no bot chats.

### Preflight and stop conditions

Immediately before execution, save `scripts/finite-status --json` from the
production host context, endpoint health, and the current Tinfoil release and
GPU allocation in a private local evidence directory. Confirm GLM-5.3-Flash,
eight H200s, healthy upstream/usage admission, and no concurrent rollout or
known chat incident. Record the actual release, rather than assuming flash-5
is still deployed. Confirm the dedicated canary key's entitlement, available
budget, and concurrency allowance for 32 calls; an admission rejection is not
a GPU capacity result. A missing or ambiguous prerequisite means do not start.

Use these conservative **diagnostic progression** criteria for every tier and
recovery probe: all requests succeed, all streams contain output and terminal
`[DONE]`, positive completion usage is returned, p95 first-output latency is at
most 10 seconds, p10 decode is at least 10 tok/s and p50 at least 20 tok/s.
Record aggregate throughput, but do not apply the 2,400 tok/s full-capacity
threshold to these small tiers. A diagnostic `passed: true` is only a pass of
the explicit small-tier criteria, not the 120-way gate.

Stop issuing test traffic on any failed tier, timeout, 429/5xx, readiness
failure, restart/OOM evidence, new unsettled test reservations, or observed
existing-user degradation. Also stop if a recovery probe's first-output time
exceeds twice the initial baseline plus one second. Do not retry a failed
burst or send a recovery inference probe into an unhealthy service. Observe
health and settlement without inference, save evidence, and investigate.

The existing checker evaluates only after a tier finishes; it cannot cancel
the other in-flight requests on the first failure. Its `--timeout` is a socket
timeout, not a total wall-clock deadline. If a tier is still running at 120
seconds, the operator interrupts the foreground command and starts no further
requests. Client cancellation does not prove GPU work or reservations have
already drained. Recovery consists of stopping test traffic and observing the
service; this check has no relaunch, configuration rollback, or ledger repair.

### Prepared command

Run from the repository root in Bash after authorization and preflight. Load
only the existing canary credential into `FINITE_PRIVATE_CANARY_API_KEY` using
the established secret workflow (`secrets/finite-private-canary.env`); never
print it or paste it into arguments. Set `FP_SANITY_EVIDENCE` to a new private
directory under `.local-state` and `FP_SANITY_RUN` to a unique UTC run label.
Save the start timestamp, actual deployment identity, script commit/hash, and
settings alongside results. This function defines one bounded invocation;
defining it sends no traffic.

```bash
fp_sanity_tier() (
  set -euo pipefail
  umask 077
  : "${FINITE_PRIVATE_CANARY_API_KEY:?Load the dedicated canary key first}"
  : "${FP_SANITY_EVIDENCE:?Set a new private local evidence directory}"
  : "${FP_SANITY_RUN:?Set a unique UTC run label}"
  local tier="${1:?Specify one tier}" label="${2:?Specify a unique label}"
  case "$tier" in 1|4|8|16|32) ;; *) return 64 ;; esac
  case "$label" in ''|*[!a-zA-Z0-9_-]*) return 64 ;; esac
  mkdir -p "$FP_SANITY_EVIDENCE"
  set -o noclobber
  export FINITE_PRIVATE_CANARY_API_KEY
  scripts/with-dev-env python3 scripts/check_finite_private_glm53_capacity.py \
    --url https://finite-private.finite.containers.tinfoil.dev \
    --model glm-5-3-flash \
    --api-key-env FINITE_PRIVATE_CANARY_API_KEY \
    --concurrency "$tier" --required-concurrency "$tier" \
    --repetitions 1 --warmup 0 --output-tokens 256 --timeout 60 \
    --thinking on --reasoning-effort high \
    --minimum-p10-output-tok-s 10 --minimum-p50-output-tok-s 20 \
    --minimum-aggregate-output-tok-s 0 --maximum-p95-ttft-s 10 \
    --tag "$FP_SANITY_RUN-$label" \
    > "$FP_SANITY_EVIDENCE/$label.jsonl" \
    2> "$FP_SANITY_EVIDENCE/$label.stderr"
)
```

After approval, the first invocation is `fp_sanity_tier 1 baseline`. Inspect
its exit status, JSONL and stderr before continuing. Subsequent tier labels
are `tier4`, `tier8`, `tier16`, and `tier32`; recovery probes use concurrency
1 and distinct labels such as `recovery4`. No loop is supplied intentionally.
The URL is the origin **without `/v1`**: despite the CLI help wording, the
checker itself appends `/v1/chat/completions`.

Retain raw reports locally: HTTP error bodies may appear in `first_error`, so
review/redact before sharing. Publish only release identity, workload settings,
counts/errors, latency/throughput, recovery and settlement conclusions. Use
the existing `finite-private-ops.sh settlement-status SINCE_UTC` for this
canary key, with the exact pre-test UTC timestamp and correct Core host;
record success of the settlements as well as the absence of reserved rows.
Capture final canonical fleet status, health, and available GPU metrics.

A passing result establishes short-burst service behavior up to 32 additional
client requests under the ambient load at test time. Client concurrency is
not proof that all requests decoded simultaneously on the GPUs. It does not
establish 107-bot capacity, long-history behavior, sustained load, model quality,
or permission to switch the fleet.


## Executed live sanity check: September 4 Central / September 5 UTC

**Passed through 32 concurrent test requests.** The user explicitly authorized
this test on the live service with normal user access left enabled. Test traffic
ran from 2026-09-05T04:07:44Z through approximately 04:13:29Z. No Agents were
switched, no Production Deploy occurred, and no user traffic was paused.

The endpoint served `v2026-08-28-glm-5-3-flash-5` on the existing eight H200s.
The test used the prepared settings: short synthetic prompts, high thinking,
256 completion tokens, one repetition per tier, zero warmups/retries, and the
public limiter with normal `usage-api` admission. Each tier used the explicit
small-tier thresholds above; the 120-way acceptance gate was not run.

| Concurrent requests | Successful / sent | p95 first output (s) | Median decode tok/s/request | Aggregate output tok/s | Burst wall time (s) |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1/1 | 0.409 | 93.546 | 81.387 | 3.145 |
| 4 | 4/4 | 0.621 | 83.393 | 277.424 | 3.691 |
| 8 | 8/8 | 0.666 | 76.022 | 508.621 | 4.027 |
| 16 | 16/16 | 0.709 | 56.979 | 783.282 | 5.229 |
| 32 | 32/32 | 0.825 | 59.097 | 1581.419 | 5.180 |

All 65 requests, including four recovery probes, returned output, positive
usage, and terminal `[DONE]`, with zero transport/HTTP/stream errors. Total
reported completion tokens were 16,640. The 32-way tier's p10 decode speed was
58.806 tok/s/request, also above the diagnostic minimum of 10.

After at least 60 seconds without test traffic following each multi-request
tier, single-request first-output times were 0.372, 0.372, 0.377, and 0.368
seconds. All passed both the ordinary diagnostic thresholds and the recovery
comparison against the 0.409-second initial baseline.

### Accounting and service observations

- The existing dedicated canary key and grant were active. Its profile was
  `finite-private-generous-v2`, with a 100,000,000-unit burst allowance and no
  weekly cap. Its previous window had expired. The inspected profile and
  admission implementation have no separate concurrent-request count limit;
  admission serializes reservations against the grant's usage allowance.
- Final key-scoped settlement evidence: **65 settled with actual usage, zero
  test-era reserved rows**. The 50 preexisting reserved rows remained 50.
  No key, grant, allowance, or accounting repair was performed.
- Endpoint readiness and canonical host/chat/recovery checks were green before
  and after the test. Tinfoil still reported the same container and tag, ready
  on eight GPUs, with no update or error reported.
- Canonical fleet status was red both before and after due to existing artifact
  convergence evidence, including a smoke-host straggler. It was not an
  inference failure. No active Runtime Operations appeared in the preflight
  Core snapshot. The status script was executed unchanged via SSH stdin using
  an already-installed Nix-store Python because the command was not installed
  in the host PATH; no production files were installed or edited. Local
  rollout-file absence is not global proof that no operator work exists.
- Tinfoil resource metrics were retained. Two-minute buckets and reporting lag
  cannot resolve these roughly 3–5-second bursts. A latest available bucket
  reached 85% maximum GPU utilization; this does not establish saturation,
  spare capacity, restart absence, or an OOM-free process history.

### Meaning and retained evidence

This is fresh evidence that the current GLM service handled 32 additional
short requests at once while normal production access remained enabled, with
fast first output and successful accounting. We did not measure the exact
number or timing of other users' simultaneous requests. Service health and
recovery probes are not per-user latency measurements. TTFT includes reasoning
output, so it is not time to a useful visible answer. This run does not prove
long-history, sustained, 107-Agent, or 120-concurrent capacity.

Raw JSONL, timestamps, command settings, exit codes, health/status snapshots,
settlement summaries, and resource metrics remain locally under
`.local-state/glm53-sanity-20260905T040552Z/` in the main `finite-mono` worktree.
The Markdown table preserves the measured results in git; private raw evidence
and the temporary single-tier launcher are not committed. The unchanged
capacity checker came from commit `d14015387951dbf535db681732194292ed5f793d`,
SHA-256 `9f6fffafe42fbffdf2f09ed3e1547c4bf51ea5cbf85f6f84ea2c13c6b4915951`.
