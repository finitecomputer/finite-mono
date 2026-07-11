# Runner Contract v1

Status: accepted boundary. Kata is the first production Runner; Phala is the
fast follow. Both implementations are incomplete.

## Decision

A Runner is a generic adapter between Core's desired compute lifecycle and a
hosting substrate. Core assigns a Runner class from product policy when
creating an agent and stores that placement with the Project. The user does not
choose Kata, Phala, or another provider during onboarding. A future
customer-facing hosting tier may select a class by promised product behavior,
but provider names and handles remain internal. Placement is not a process-wide
environment switch and does not change the dashboard or Runtime product
contract.

Runner interprets compute concerns only. It never implements chat, Google,
Telegram, Sites, Brain, skills, or Hermes feature behavior.

## Provider-Neutral Inputs

Core gives every Runner the same `RuntimeSpec`:

- Project and Runtime ids plus an idempotent operation id;
- Core-assigned Runner class and resource class;
- one immutable Finite Product Release and Runtime image digest;
- one durable state identity mounted at the Runtime's `/data` contract;
- the fixed Runtime entrypoint, network endpoints, and health contract; and
- opaque environment and secret references that the adapter transports but
  does not interpret.

Provider-specific ids, Kubernetes objects, Phala concepts, host paths, shell
commands, and feature settings do not belong in `RuntimeSpec`.

After provider creation, Core durably records an opaque Provider Runtime Handle
before waiting for application readiness. The handle contains only what the
same adapter needs to inspect or re-adopt that Runtime after a worker restart;
it is not exposed as the product model.

## First-Slice Lifecycle

Every adapter implements the same idempotent operations:

- `validate(spec)`
- `ensure(spec, operation_id)`
- `inspect(handle)` and `adopt(handle)`
- `restart(handle, operation_id)`
- `stop(handle, operation_id)`

`restart` uses the same durable state and receives the same provider-neutral,
bounded non-secret desired environment as `ensure`. If an adapter replaces
compute while reconciling a release, it merges only those explicitly desired
opaque keys into the inspected environment. Existing contract variables,
provider settings, and credentials are preserved; restart never provisions or
rotates inference credentials. An adapter that cannot update environment on
existing compute replaces that compute when a desired opaque key is missing or
different, even if the image digest already matches. `stop` halts active
compute without deleting durable state. Compute retirement is a separate
generic operation; Purge User Data is a separate, explicitly authorized
product workflow and is not a Runner v1 operation.

Leases and fencing prevent two workers from mutating one Runtime concurrently.
A worker that crashes after a provider mutation resumes from the persisted
operation id and Provider Runtime Handle instead of blindly creating again.

## Adapter Order

Kata ships first on finite-lat-1. Its adapter may use containerd and
`io.containerd.kata.v2` internally, but that vocabulary stops at the adapter.

Phala follows against this exact contract. It may add confidential-compute
evidence as provider facts, but it may not add a second Project model,
scheduler, dashboard flow, Runtime image, or feature-specific launch path.

## Data And Recovery Boundary

Same-volume restart preservation is a first-slice requirement: agent identity,
chat state, Hermes memory, workspace data, and locally held connection state
must remain available after restart.

A Provider Durable Volume is not a backup. Provider-independent Recovery
Snapshots, key backup, export, retention, and empty-target restore remain an
explicit TODO/open design question, not a gate for launching and iterating on
the first SaaS slice. Until that work is proven, the product must not claim
disaster recovery or cryptographic operator-blindness, and normal lifecycle
operations must not delete user data.

## Conformance Gate

The same black-box suite runs against fake, Kata, and Phala adapters:

- duplicate `ensure` returns the same Runtime rather than creating another;
- a worker crash at each creation boundary is recoverable through `adopt`;
- restart preserves `/data`, release identity, and agent identity;
- stop preserves `/data` and a later `ensure` resumes it;
- stale workers are fenced and secrets do not appear in argv, logs, or handles;
- application health and release telemetry use the same contract; and
- product features behave identically without adapter-specific branches.

## Current Gaps

Each worker advertises one adapter class and Core matches it to the class stored
on the Project. Project requests do not yet carry a complete provider-neutral
`RuntimeSpec`, and Core does not persist a structured handle early enough for
full adopt/reconcile. Compute destroy now preserves durable state, but the
separate Recovery Snapshot/export/purge lifecycle remains open.

The shared launch path now accepts a bounded `FC_RUNNER_RUNTIME_ENV_JSON` map as
transitional local-development scaffolding. Every adapter carries the same
non-secret map opaquely; contract-owned keys, secret-looking names, malformed
names, and oversized values fail closed, and diagnostics expose keys only. It
exists so Devfinity can hand an Agent Runtime reachable local product-service
URLs without an Apple-specific Sites or Brain branch. Production still needs
Core-owned per-Project RuntimeSpec persistence and secret references; the
process-wide JSON setting is not that final contract and must not be used for
connection credentials.

Production may additionally point `FC_RUNNER_RUNTIME_SECRET_ENV_FILE` at one
root-owned, mode-0600 `KEY=VALUE` file. The initial launch path validates the
bounded map, rejects Runtime-contract keys, exposes key names only in
diagnostics, and transports values through each adapter's secret-safe channel.
This restores the legacy shared tool-provider set without teaching Runner what
FAL, xAI, or any product feature means. It is transitional host-wide bootstrap,
not a replacement for Core-owned per-Project secret references. Runtime
restart preserves the credential set already held by the Runtime; it does not
silently rotate shared or inference credentials.
