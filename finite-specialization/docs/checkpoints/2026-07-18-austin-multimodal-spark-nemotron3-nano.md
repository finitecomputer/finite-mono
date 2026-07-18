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

Promotion requires current semantic passes for image, audio, and sampled video
through the authenticated Tyk and Public Beta Ingress path, followed by fresh
worker canaries for all three capabilities. The prior Gemma 12 raw runtime
remains running but absent from the Tyk-visible model catalog during the
rollback soak.

That gate passed on 2026-07-18. The authenticated Tyk path returned `729` for
the fixed image, transcribed the fixed Audex WAV phrase, and returned the
chronological `red, red, blue, blue` markers for four sampled video frames.
The final worker canaries completed healthy at Unix timestamps `1784409457`
(image), `1784409559` (audio), and `1784409657` (video). The deployed worker is
`ghcr.io/finitecomputer/finite-specialization-worker:2026-07-18.1@sha256:93bb5f03df49fb62d32247e45f379432d5671e1201d6f46c096f1e0bdb6dc5c0`,
built from commit `2d48d43`. Nemotron audio uses a system capability message;
the model's multimodal template rejects a developer message followed by
list-valued audio content.

The Spark promotion artifacts are in
`spark-cluster/runs/2026-07-18-nemotron-omni-2f73-cutover/`. The final public
route backup is
`.public-beta-ingress.env.bak-public-beta-route-sync-20260718T212159Z` on
`finite-gateway`. The pre-worker-fix deployment backup is
`/root/nemotron-cutover-20260718T2117Z/worker-deployment-before-audio-fix.yaml`
on `clawland-ovh`.

Rollback is capability-safe: restore the Gemma 12 aliases in the Public Beta
Ingress manifest, set `FINITE_SPECIALIZATION_VISION_MODEL` back to
`aeon-gemma-4-12b-k4-nvfp4-unified-fast`, roll out only the specialization
worker, and require the three semantic canaries to return healthy before
removing Nemotron.

The exact fail-closed boundary is the Public Beta route manifest: restore its
named backup first so both models overlap, reconcile the worker to the approved
Gemma rollback alias, verify all three worker canaries, and only then withdraw
Nemotron. The Gemma raw runtime stays hot and operator-visible throughout the
rollback soak.
