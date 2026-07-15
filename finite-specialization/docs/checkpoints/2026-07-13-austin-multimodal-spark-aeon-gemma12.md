# Austin Multimodal Spark AEON Gemma 12B Checkpoint

Date: 2026-07-13
Status: active verified specialization
Hermes product capability: `vision_analyze` through `vision-input`
Raw worker capabilities: audio interpretation and sampled-video analysis
Hermes surface: `auxiliary.vision`
Backing plane: Spark public beta through Tyk

## Result

AEON Gemma 4 12B K4 NVFP4 Unified replaces Qwopus as the shared worker's
multimodal model. Maya remains the separate resident text-to-speech model.

The active alias is:

```text
aeon-gemma-4-12b-k4-nvfp4-unified-fast
```

The full unified checkpoint passed semantic raw-runtime probes for text,
streaming, forced tool calls, image text, audio transcription, sampled-video
frame interpretation, and native `/v1/responses` image input. Authenticated
image requests through both Tyk and the shared worker passed on the final ee82
runtime, each reading the expected `729` marker.

## Hermes Shape

```yaml
auxiliary:
  vision:
    provider: custom
    model: aeon-gemma-4-12b-k4-nvfp4-unified-fast
    base_url: http://finite-specialization-worker.fc-specializations.svc.cluster.local:18998/v1
    api_key: external_worker_secret_only
    api_mode: chat_completions
    timeout: 120
    download_timeout: 30
```

The shared worker selects the Gemma alias explicitly with
`FINITE_SPECIALIZATION_VISION_MODEL`. Secrets remain outside this repository.

## Capability Boundary

Hermes `auxiliary.vision` sends image requests through the compatibility
worker. Audio and sampled-video support are properties of the same Spark model
route, but Hermes does not expose agent tools for either one today. Maya owns
speech generation; voice-message transcription remains Hermes's own
transcript-first flow.

## Rollback

Restore the previous verified vision alias in the shared worker configuration,
reconcile that deployment, and keep the public route fail-closed if no raw
backend is listening. Do not change Maya during a vision rollback.
