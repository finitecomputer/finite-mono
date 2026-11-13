# Qualifying Inkling 2 Small on eight H200s

This procedure prepares an optional Finite Private maintenance trial. The
[candidate](../tinfoil/confidential-finite-private/tinfoil-config.inkling-2-small.candidate.yml)
is deliberately incomplete. Merging preparation, publishing a canary image,
and publishing a measured release do not authorize replacing GLM. Record
runtime evidence and the go/no-go decision in the PR and owning Linear issue.

## Evidence and interpretation

The candidate fixes checkpoint
`thinkingmachines/Inkling-2-Small@8361e7a31ae7e9966a49a91d3994fde618936c2b`.
The [publisher](https://huggingface.co/thinkingmachines/Inkling-2-Small)
describes native NVFP4 training; the checkpoint actually mixes NVFP4 and
MXFP8 experts with other tensors. Its approximately 393.51 GB storage size
is not loaded GPU memory. Neither parameter counts nor Blackwell benchmarks
predict H200 throughput.

The [pinned SGLang H200 recipe](https://github.com/sgl-project/sglang/blob/29327444b130ee007fd06ff957b3fdc7066ceaa8/docs/src/snippets/configs/thinkingmachines/inkling-2-small.jsx)
marks TP8/EP8 Balanced and DSpark verified; its H200 performance entries are
empty. Use Balanced first, with FA4, BF16, `inkling` reasoning and `inkling2`
tools. This candidate explicitly limits context to the existing 393,216-token
service ceiling and uses BF16 KV. These additional limits require Finite
measurement; they are not a verbatim upstream qualification. Do not carry over
GLM's DSA or DeepGEMM flags. No DSpark result is implied by a Balanced result.

Hopper's [generic NVFP4 Marlin path](https://github.com/sgl-project/sglang/blob/29327444b130ee007fd06ff957b3fdc7066ceaa8/python/sglang/srt/layers/quantization/modelopt_quant.py#L2396)
uses packed weights with 16-bit activations. Confirm the actual image's kernel
selection and MXFP8-tail handling from startup logs; this source is not proof
of the entire development image. Record source revision, image digest, GPU
topology, driver/CUDA versions, load/peak memory per GPU, remaining KV capacity,
startup duration, and any conversions or unsupported-kernel warnings.

The [checkpoint chat template](https://huggingface.co/thinkingmachines/Inkling-2-Small/blob/8361e7a31ae7e9966a49a91d3994fde618936c2b/chat_template.jinja)
maps `none/low/high/max` to thinking efforts `0/0.2/0.9/0.99`. It does not
reference `enable_thinking`. Existing clients and the limiter send that flag,
including false combined with `reasoning_effort=high`. Treat this as an
unresolved compatibility gate: prove the exact server normalizes the old
payload or implement and qualify a bounded adapter before exposing the
replacement to those clients. A few outputs with empty reasoning do not prove
that the flag is honored. The template also requires parsed objects for
historical tool-call arguments; the server must canonicalize OpenAI JSON-string
arguments on tool-result replay. The probes preserve client format to expose
this failure rather than hide it.

The publisher [acknowledges chat and tool-selection weaknesses](https://thinkingmachines.ai/news/inkling-2-small/).
Synthetic protocol passes do not settle these. Use the same representative
chat, coding, many-skill, and long-history tasks for GLM and Inkling and retain
a human assessment of correctness, concision, tool choice and refusal behavior.

## Prepare without stopping GLM

Run from the monorepo root with Nix-managed tools:

```sh
just finite-private-inkling-contract
just finite-private-inkling-artifacts-ready
```

The first command runs offline checks and synthetic fixtures. The second
**must fail while image/MPK stop markers remain**. It checks immutable artifact
syntax and mount consistency, not runtime results, measurement or authorization.

The [Docker Hub tag metadata](https://hub.docker.com/v2/repositories/lmsysorg/sglang/tags/dev-inkling-2-small)
reported the Linux/amd64 manifest
`sha256:7cdd32fdd1acdd44207ff2b1483ea776d31769d600241d77fefe878dd0e4fa44`
on 2026-11-13; the Dockerfile fixes that digest. Direct registry access timed
out during preparation. Metadata resolution is not a successful image pull or
runtime test. Run the `Inkling 2 Small qualification preparation` workflow
manually on the reviewed source revision.
The workflow builds remotely, publishes a canary only, checks provenance,
source/base labels, fail-closed auth and anonymous pull, and prints the Finite
image digest. It never publishes a satellite release or updates an instance.
If GitHub has not registered this new workflow, register it through the normal
review/merge path before dispatch. Review the exact upstream build's code and
dependencies as well as its recipe; the recipe revision is not an image pin.

Use Tinfoil's current modelwrap tooling to package the **fixed checkpoint
revision**, including tokenizer, template, processor and required auxiliary
files. Verify the packaged revision and file completeness against Hugging Face
metadata; do not download `main` and label it with the fixed SHA. Record the
resulting MPK identity and root. Replace the candidate's two MPK/mount markers
and Finite image marker, then rerun the artifact check. No weights should be
built or packaged on a production host. The retained limiter digest is an
input to qualification, not proof of the currently deployed digest.

Run direct-server and limiter qualification on isolated hardware if available.
Otherwise the eight-H200 tests remain unexecuted until the explicitly approved
window. Do not borrow or stop the live GPUs as an implicit preparation step.

## Execute serial protocol probes

Choose an explicit endpoint and supply the proper direct-server or Finite
canary key through `FINITE_PRIVATE_CANARY_API_KEY`. Never put its value in a
command, report, manifest or PR. The probe refuses redirects and has no default
endpoint. It sends synthetic inference requests, which consume capacity and
can create usage reservations when routed through the limiter.

For a local candidate or approved tunnel:

```sh
scripts/with-dev-env python3 scripts/check_finite_private_inkling_protocol.py \
  --endpoint http://127.0.0.1:8001/v1 --tag inkling-balanced-direct
```

For a separately authorized remote candidate, pass its explicit HTTPS `/v1`
URL with `--allow-remote`. Run again through the limiter and separately with
each retained `--model`: `glm-5-3-flash`, `glm-5.3-flash`, `glm-5-2`, and
`deepseek-v4-flash-0731`. The expected response model remains
`inkling-2-small`. A `--tag` is an evidence label, not identity/attestation proof.
The probe checks separated reasoning, native effort inputs, existing fast
controls and reasoning history, automatic tools, JSON argument fragments,
parallel calls, tool-result replay, complete SSE/usage and cancellation recovery.
It never executes a requested tool. All failed cases block compatibility signoff.

After the short suite, repeat with `--context-tokens 128000`, then
`--context-tokens 360000 --timeout-seconds 1200`. Long-prefill probes are opt-in,
serial and calibrate their synthetic input using reported prompt usage. Their
pass requires an observed prompt count within the target/service bounds and
successful post-prefill recovery. This establishes one request, not concurrent
long-context capacity or long-context retrieval quality. Reports redact prompts,
generated content, upstream error bodies and keys.

For throughput, reuse `scripts/check_finite_private_glm53_capacity.py` with
`--model inkling-2-small --thinking default --reasoning-effort high` and an
explicit `--url`, `--tag`, and canary-key environment. Start with concurrency
1, then 4/16/32 before any higher load. For exploratory tiers, explicitly set
`--concurrency 1 --required-concurrency 1` (and similarly for each later tier);
keep the full-gate thresholds unchanged and label these as exploration. The
script requires the acceptance tier to be in the requested concurrency list.
Its schema/name is historical: it is a
synthetic forced-length token-throughput measure and includes reasoning tokens,
not a useful-answer latency metric or mixed long-context production benchmark.
Its existing full gate requires 120 successes, p10 >= 10 tok/s, p50 >= 20 tok/s,
aggregate >= 2400 tok/s and p95 first output <= 10 s. A smaller sweep must not
be reported as passing that gate. Agree on any changed acceptance criteria in
Linear before the window; do not tune thresholds after seeing Inkling results.
Compare both models with identical requests and explicit thinking controls.

Also check direct negative authentication, malformed requests, unknown-model
rejection before reservation, `/v1/responses`, native images if exposed, a
complete existing hosted-Agent chat/tool turn, a resumed historical session,
and a disposable new-Agent onboarding-to-chat path. The protocol script does
not claim those gates. Reuse [existing canaries](finite-private-ops.sh) with
explicit endpoint/model/expected-model overrides for the candidate and verify
reservation settlement. Many-skill tasks and chat quality require human review.

## Maintenance window and rollback boundary

Before proposing replacement, capture `scripts/finite-status` and the existing
instance's status, exact measured GLM tag, measurement, configuration, resource
allocation, volume bindings and secret **names/custody** in access-controlled
operator evidence. Confirm a compatible GLM release is still deployable and
that keys were not rotated incompatibly. Preserve GLM checkpoint and images
for another cold start. Do not assume this repository's GLM candidate is live.

Record the exact trial tag, exact GLM rollback tag, maximum trial duration,
latest rollback-start time, operator and approval scope before proceeding.
Budget a full cold start for the candidate and another for GLM, including
pulls, weight loads and compilation. No measured timings exist yet; a short
maintenance window cannot be promised. If the available window cannot cover
recovery, do not start. Obtain explicit authorization for replacement and the
specified rollback contingency; preparation alone grants neither.

Follow [current Finite Private operations](../tinfoil/confidential-finite-private/README.md)
for measured release publication and same-instance Update/Deploy. Both release
phases must succeed. Preserve the stable `finite-private` identity, secret
selection, usage-api admission and retained aliases. The candidate inherits
CVM 0.14.17's patched-kernel requirement and explicit unencrypted NVLink setting;
confirm the selected host's requirements and security acceptance before release.
Do not combine model trial with a host, CVM, limiter, Runtime, route or billing
change. Do not delete/recreate the instance to work around an update failure.

Eight GPUs require replacement with downtime. Once the old enclave stops, the
update cannot be canceled and automatic rollback is unavailable. A readiness
timeout is a decision point, not a recovery-time guarantee. Prove exact live tag
and attestation against the published measurement before interpreting HTTP
health; the ops helper's `wait-ready` can pass while the old release serves.

If readiness, kernel support, protocol/old-client compatibility, settlement,
memory, quality or agreed performance fails, stop increasing load and follow
the approved rollback contingency. A failed/stopped instance needs Deploy of
the preserved compatible GLM release on the same instance; a running candidate
uses Update. Re-prove exact GLM tag/attestation, streaming through `[DONE]`, all
aliases, existing-Agent chat, new-Agent readiness, auth and settlement, and run
`scripts/finite-status` after recovery. Record actual outage and cold-start
times in the PR/issue.

The inference service does not own chat history. This trial requires no durable
user-state migration or purge. Preserve provider volumes, identity and the
independently held Recovery Set; a CVM or volume is not a backup. Rollback
restores serving artifacts, not lost user data. If the trial uncovers any
state repair requirement, stop that work and qualify it separately on synthetic
state before seeking authorization.
