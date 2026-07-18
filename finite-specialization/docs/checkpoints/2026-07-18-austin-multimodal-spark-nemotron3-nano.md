# Austin multimodal specialization on Nemotron 3 Nano Omni

The canonical `austin-finite` specialization model is:

```text
nemotron-3-nano-omni-30b-a3b-reasoning-nvfp4-fast
```

The worker continues to own media retrieval and normalization. Image Analysis
uses image input, Audio Interpretation uses canonical `input_audio`, and
Sampled-Video Analysis extracts up to four chronological frames and sends them
as standard image inputs through Responses. The Spark route does not advertise
the optional direct `video_url` extension because the pinned NVIDIA 26.06
runtime does not decode the representative H.264 MP4 fixture.

Promotion used the operator-scoped practical gate: authenticated Tyk API
compatibility, text and structured tool calls, image/audio/sampled-video
semantics, bounded multimodal concurrency probes, and fresh worker and Maya
canaries. The operator explicitly waived the full agent benchmark for this
finite-specialization cutover. The prior Gemma 12 raw runtime is retired: its
aliases were withdrawn before the final spark-ee82 container was stopped with
restart disabled. Maya TTS now has spark-ee82 to itself.

That gate passed on 2026-07-18. The authenticated Tyk path returned `729` for
the fixed image, transcribed the fixed Audex WAV phrase, and returned red and
blue in chronological order for the sampled video. Fast and thinking profiles
both passed their final multimodal checks. With the vLLM runtime, Tyk routes,
and worker media semaphore all set to eight, warm c8 probes completed 8/8 for
image, WAV audio, sampled video, and a mixed workload. Wall times were 0.944,
2.583, 2.322, and 1.626 seconds respectively, while runtime metrics observed
eight running requests and zero waiting. At c9, homogeneous audio and video
each returned one intentional `capacity_exceeded` rejection, so eight is the
supported all-modality ceiling. This is a short practical capacity check, not
a sustained media benchmark.

The final worker canaries completed healthy at Unix timestamps `1784413657`
(image, 423 ms), `1784413459` (audio, 2873 ms), and `1784413557` (video,
1020 ms). The deployed worker is
`ghcr.io/finitecomputer/finite-specialization-worker:2026-07-18.1@sha256:93bb5f03df49fb62d32247e45f379432d5671e1201d6f46c096f1e0bdb6dc5c0`,
built from commit `2d48d43`. Nemotron audio uses a system capability message;
the model's multimodal template rejects a developer message followed by
list-valued audio content.

The executed Spark runtime image is
`sha256:7bcc7cc08c926b8ba67e05efb6d9b7e7a227c932e98d3aef2b85533644f27650`;
the runner resolves the configured mutable tag to this immutable image ID
before container creation.

The initial Spark promotion artifacts are in
`spark-cluster/runs/2026-07-18-nemotron-omni-2f73-cutover/`; the c8 and
Maya-solo evidence is in
`spark-cluster/runs/2026-07-18-maya-solo-nemotron-mm-concurrency/`. The c8
public route backup is
`.public-beta-ingress.env.bak-public-beta-route-sync-20260718T231019Z` on
`finite-gateway`. The pre-worker-fix deployment backup is
`/root/nemotron-cutover-20260718T2117Z/worker-deployment-before-audio-fix.yaml`
on `clawland-ovh`.

Rollback is fail-closed: withdraw the two Nemotron aliases before stopping its
runtime or changing the worker model. If c8 proves unhealthy, reduce the
worker, runtime, and both route manifests together to c4, then require fresh
image, audio, and video canaries. Gemma is not a rollback target.
