# Austin Vision Spark Qwopus Checkpoint

Date: 2026-07-07
Status: first working specialization
Capability: `vision-input`
Hermes surface: `auxiliary.vision`
Backing plane: Spark public beta

## Result

Austin's Finite Hermes agent has a working vision specialization for a
text-only main model profile. Image input is handled by a dedicated
`auxiliary.vision` model, not by MoA. The auxiliary model is Qwopus through the
Spark public-beta front door.

This is the first verified specialization in this repo that is both:

- a concrete Hermes capability config; and
- backed by a specialized model served through the Spark cluster.

## Working Shape

The target Hermes fragment is:

```yaml
auxiliary:
  vision:
    provider: custom
    model: qwopus3-6-35b-a3b-v1-q5-k-m-gguf-fast
    base_url: http://qwopus-vision-bridge:18998/v1
    api_key: external_bridge_secret_only
    api_mode: chat_completions
    timeout: 120
    download_timeout: 30
```

The bridge runs in the target agent namespace and forwards to:

```yaml
SPARK_BASE_URL: https://inference.finite.computer/v1
SPARK_MODEL: qwopus3-6-35b-a3b-v1-q5-k-m-gguf-fast
SPARK_API_KEY: external_secret_only
```

No live API key belongs in this repo.

## Why The Bridge Exists

The verified Spark route is Responses-shaped:

- `images: true`
- `responses: true`
- `chat_completions: false`

Hermes v0.14 can target `codex_responses`, but that path uses streaming
Responses. The Spark public-beta Qwopus image route was verified with
non-streaming `/v1/responses`. The bridge keeps Hermes on its existing
`chat_completions` auxiliary surface and converts the request to a
non-streaming Spark Responses request.

## Deployment Scope

This checkpoint was deployed only for the Austin agent namespace:

- namespace: target agent namespace
- bridge service: `qwopus-vision-bridge`
- bridge port: `18998`
- Hermes surface changed: `auxiliary.vision`
- other agents changed: no
- public front door changed: no

## Shared Worker Cleanup

Later on 2026-07-07, this namespace-local bridge was superseded by the shared
`finite-specialization-worker` in the `fc-specializations` namespace:

```yaml
auxiliary:
  vision:
    provider: custom
    model: qwopus3-6-35b-a3b-v1-q5-k-m-gguf-fast
    base_url: http://finite-specialization-worker.fc-specializations.svc.cluster.local:18998/v1
    api_key: external_worker_secret_only
    api_mode: chat_completions
    timeout: 120
    download_timeout: 30
```

The namespace-local `qwopus-vision-bridge` Deployment, Service, ConfigMap, and
Secret are no longer part of the active lane after the shared worker is verified.
The bridge details above are retained as historical bring-up evidence.

## Verification Evidence

The public front door recovered to the expected healthy posture:

- unauthenticated `GET /v1/models`: `401 Unauthorized`
- authenticated `GET /v1/models`: lists
  `qwopus3-6-35b-a3b-v1-q5-k-m-gguf-fast`
- route health: `healthy`
- route health reason: `qwopus_q5_public_beta_image_text_smoke_passed`

Bridge-level smokes passed:

- generated red PNG returned `Red`
- cached dashboard screenshot returned a correct description including
  `AUSTIN-FINITE`, `Austin`, and `austin@finite.vip`

Hermes tool smoke passed:

```json
{
  "success": true,
  "analysis": "This image shows a dark-themed user interface..."
}
```

The successful analysis identified the important visible text and controls:

- `AUSTIN-FINITE`
- `Austin`
- `Assigned to austin@finite.vip. Chat relay is connected.`
- `Chat`
- `OpenCode`
- `Connections`
- `Restart machine`
- `Sign out`

## Incident During Bring-Up

The first live demo failed because the public inference front door returned
Caddy `502` responses. The failure was not in Austin Hermes and not in Qwopus
itself. The likely broken edge was the public Caddy host's route to the Pi
public-beta ingress over Tailnet.

Healthy public edge behavior is:

- unauthenticated request reaches ingress and returns `401`;
- authenticated request reaches ingress and returns the model list or inference
  result.

If public requests return empty Caddy `502`, inspect the Caddy edge to Pi
target before changing Hermes config.

## Rollback

Rollback is target-agent scoped:

1. Remove or replace `auxiliary.vision` in the target Hermes config.
2. Restart only that target agent workload.
3. Delete the namespace-local `qwopus-vision-bridge` Deployment, Service,
   ConfigMap, and Secret if nothing else is using them.

Do not rotate public-beta keys for a simple vision rollback.

## Next Specialization Lessons

- Start with the Hermes capability surface first.
- Treat Spark as the backing plane only after route health and a
  capability-specific smoke pass.
- Do not use MoA for raw image input.
- Keep compatibility bridges namespace-local until the API shape is stable.
- Record front-door health separately from Hermes config success.
