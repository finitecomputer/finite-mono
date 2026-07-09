# Box1 Vision Specialization Rollout Plan

Status: draft plan
Target host: box1 / `clawland-ovh`
Initial capability: `vision-input`
Initial Spark-backed model: `qwopus3-6-35b-a3b-v1-q5-k-m-gguf-fast`

## Goal

Deploy the first durable Finite Specialization Config on box1: vision input for
text-only Finite Private agents using GLM 5.2.

The specialization should become active only when an Agent Runtime is using:

- Inference Profile: `finite-private`
- Model: `glm-5-2`

It must not overwrite user-owned custom vision settings.

## Resolved Decisions

- Use the existing Hermes `auxiliary.vision` surface. This is not MoA.
- Use Spark as the backing plane through `https://inference.finite.computer/v1`.
- Store the Spark gateway API key only as an external secret. Do not commit it.
- Deploy one shared `finite-specialization-worker` on box1 instead of one
  Qwopus bridge per agent namespace.
- Run the shared worker in a dedicated Kubernetes namespace:
  `fc-specializations`.
- Put two responsibilities in that deployment, separated internally:
  - request worker: translate Hermes vision requests and call Spark;
  - activation reconciler: decide which agents should point Hermes at the
    worker.
- Activate only for agents matching Finite Private + `glm-5-2`.
- Fill vision only when `auxiliary.vision` is blank, `auto`, or already marked
  as Finite-managed.
- Record Finite-managed ownership in a sidecar metadata file under the runtime
  Hermes directory, not as unknown fields inside Hermes `config.yaml`.
- Remove or update only Finite-managed specialization config. Leave user-owned
  custom vision alone.
- Make successful inference-profile changes report back to Core so the worker
  is triggered by product state changes rather than continuous polling.
- Have Core write durable targeted specialization reconcile jobs. The worker
  consumes those jobs instead of relying on a direct fire-and-forget HTTP call.
- Store effective Inference Profile state and specialization reconcile jobs in
  Core SQLite, not a loose host filesystem queue.
- Have the worker claim and complete jobs through a small internal Core/finited
  API. Do not mount Core SQLite directly into the worker.
- Implement the worker, Core schema/API, Nix/k3s deployment, and dashboard /
  finitec integration in `finitecomputer`.
- Keep `finite-specialization` as the vocabulary, config, plan, and checkpoint
  repository for Specialization Configs.

## Worker Request Path

Hermes calls the worker using an OpenAI-compatible chat-completions shape:

```text
POST /v1/chat/completions
```

The worker:

1. verifies the calling Agent Runtime with an inbound bearer token;
2. converts Hermes chat-completions image input to Spark Responses input;
3. calls Spark through the Tyk gateway;
4. returns normal chat-completion text back to Hermes.

The worker should not store conversations, make profile decisions during a
request, run the main agent, or replace Hermes.

## Activation Reconciler

The reconciler is the activation part of the worker deployment.

For a target Agent Runtime on box1, it should:

1. read Core's recorded effective Inference Profile state;
2. classify whether the runtime is using Finite Private + `glm-5-2`;
3. inspect the current Hermes `auxiliary.vision` setting;
4. inspect the sidecar ownership file;
5. apply the managed vision fragment only if the runtime is eligible;
6. record managed ownership in the sidecar file;
7. remove the managed vision fragment when the runtime is no longer eligible;
8. skip and report any runtime with user-owned custom vision.

For the first durable version, `finitec runtime inference apply` should report
successful profile changes back to Core. That report is non-secret product
state, not raw Hermes YAML. Core should then trigger a targeted specialization
reconcile for that Agent Runtime.

Manual or recovery commands may still run a one-shot "reconcile all" pass, but
the normal product path should not depend on continuous polling.

## Trigger Queue

Core should write a durable targeted job in SQLite when an effective Inference
Profile changes:

```json
{
  "kind": "specialization.reconcile",
  "machineId": "austin-finite",
  "reason": "inference_profile_changed",
  "requestedAt": "2026-07-07T00:00:00Z"
}
```

The worker consumes the job and reconciles only that Agent Runtime. If the
worker is restarting, the job remains available instead of being lost.

This queue is not a fleet poll. A separate one-shot "reconcile all" command may
exist for initial rollout and recovery.

The Core-owned queue should support:

- pending / running / succeeded / failed job status;
- claim-with-lease so only one worker handles a job;
- retry attempts and last error;
- one active job per machine + reason where practical;
- enough history to audit what changed.

The host filesystem reconcile queue remains for host manifest operations. It is
not the right owner for product-level Specialization Config activation.

The worker should access this queue through an internal Core/finited API:

- claim next pending specialization job;
- mark job succeeded;
- mark job failed with a bounded error string;
- read the target Agent Runtime's effective Inference Profile state.

The worker should not mount or write Core SQLite files directly.

## Inference Profile State Report

After a successful inference-profile apply or rollback, `finitec` should report
the effective state to Core:

```json
{
  "machineId": "austin-finite",
  "activeProfile": "finite-private",
  "model": "glm-5-2",
  "provider": "custom",
  "baseUrl": "https://kimi-k2-6.finite.containers.tinfoil.dev/v1",
  "apiMode": "chat_completions"
}
```

Do not report API keys or raw Hermes config.

## Hermes Fragment Shape

The managed Hermes target should point at the shared worker, not directly at
Spark:

```yaml
auxiliary:
  vision:
    provider: custom
    model: qwopus3-6-35b-a3b-v1-q5-k-m-gguf-fast
    base_url: http://finite-specialization-worker.fc-specializations.svc.cluster.local:18998/v1
    api_key: external_agent_worker_token_only
    api_mode: chat_completions
    timeout: 120
    download_timeout: 30
```

## Secrets

External secrets required:

- Spark/Tyk gateway bearer token for worker-to-Spark requests.
- Per-agent inbound bearer tokens for Hermes-to-worker requests.

No live API key, bearer token, runtime `.env`, or full Hermes config belongs in
this repository.

## Ownership Metadata

The first version should use a runtime-local sidecar file such as:

```text
~/.hermes/.finite-specializations.json
```

The sidecar records which Hermes config surfaces the Finite Specialization
Config owns. It should include at least:

- specialization id;
- owned Hermes surface, such as `auxiliary.vision`;
- applied model and worker base URL;
- generated inbound worker token id or fingerprint, not the raw token;
- last applied timestamp.

Do not add unknown Finite-only keys inside Hermes `config.yaml` for ownership.

## Safety Rules

- Dry-run must list exactly which agents would be changed and why.
- Agents with `auxiliary.vision.provider` set to a non-empty value other than
  `auto` are skipped unless the setting is already marked Finite-managed.
- Rollback removes only the managed vision config and worker resources.
- A worker outage should degrade vision only; it should not break the main
  Hermes loop for text requests.
- The worker must validate inbound auth before any box-wide rollout.

## Verification

Before rollout:

- confirm Spark gateway returns expected auth behavior;
- smoke the worker with a generated local image;
- smoke Austin's existing cached dashboard screenshot through the worker;
- run dry-run reconciliation over all box1 agents;
- verify skipped custom-vision agents are reported and untouched.

After rollout:

- check Austin still succeeds through `vision_analyze`;
- check at least one other eligible blank/auto Finite Private + `glm-5-2`
  runtime gets vision;
- check a non-Finite Private runtime does not get the specialization;
- check a custom-vision runtime remains unchanged.

## Later Product Direction

Move desired Inference Profile and Specialization Config state into Core so the
activation reconciler reads product state rather than inferring from live Hermes
YAML. The box1 plan is intentionally transitional.
