# finite-specialization

Loose scaffolding for giving Finite agents specialized capabilities through
Hermes configuration.

## Basic Idea

Finite agents should ask for a capability, not memorize every model, endpoint,
or provider-specific config field. This repo sketches the small layer that maps
capability needs to the right Hermes config surface.

The corrected working model:

- Hermes is the acting runtime and owns the main agent loop, tools, memory, and
  final user output.
- Dedicated capability configs come first for I/O, media, services, tools, and
  runtime behavior.
- MoA is only the "extra text reasoning" layer: reference models advise the
  acting model, but they do not become STT, TTS, browser, search, or vision I/O.
- Spark is the preferred backing plane for model-backed capabilities whenever a
  healthy OpenAI-compatible route exists, but the first question is still which
  Hermes config surface should own the capability.

This is intentionally not a full service yet. Right now it is a place to write
down the capability vocabulary and example Hermes config fragments before
building a router or generator.

## What Lives Here

- [config/specializations.example.yaml](config/specializations.example.yaml)
  sketches capability records Finite agents can target.
- [config/hermes-capabilities.example.yaml](config/hermes-capabilities.example.yaml)
  sketches the first-priority Hermes config surfaces for a text-only GLM-style
  main agent enhanced with STT, TTS, vision, web, tools, and auxiliary models.
- [config/hermes-moa.example.yaml](config/hermes-moa.example.yaml) sketches the
  narrow MoA layer for cognitive specialists only.
- [docs/routing-model.md](docs/routing-model.md) explains how to pick a
  dedicated config surface before reaching for MoA.

## Working Checkpoints

- [Austin Multimodal Spark Nemotron 3 Nano Omni](docs/checkpoints/2026-07-18-austin-multimodal-spark-nemotron3-nano.md)
  is the active specialization for image input and the verified Spark model for
  audio interpretation and sampled-video analysis. The reusable Hermes vision
  fragment is
  [config/working/vision-input.spark-nemotron3-nano.hermes-fragment.yaml](config/working/vision-input.spark-nemotron3-nano.hermes-fragment.yaml).
- [Austin Multimodal Spark AEON Gemma 12B](docs/checkpoints/2026-07-13-austin-multimodal-spark-aeon-gemma12.md)
  is the historical rollback checkpoint retained for the prior public slot.
- [Austin Vision Spark Qwopus](docs/checkpoints/2026-07-07-austin-vision-spark-qwopus.md)
  is the historical first verified specialization: a text-only Hermes agent using
  `auxiliary.vision` backed by the canonical Spark public-beta AEON Gemma 12B
  multimodal route through `finite-specialization-worker`.
  The reusable fragment is
  [config/working/vision-input.spark-qwopus.hermes-fragment.yaml](config/working/vision-input.spark-qwopus.hermes-fragment.yaml).

## Non-Goals For Now

- No hard-coded claim that a live Spark alias is currently healthy unless it is
  part of a dated working checkpoint with smoke evidence.
- No automatic patching of `~/.hermes/config.yaml`.
- No model health checking.
- No router service.
- No committed Hermes secrets, traces, or machine-local state.

## Priority Rule

Use a dedicated Hermes config when the capability transforms I/O, calls a
service, uses local software, touches the OS, or needs a tool.

Use MoA when the capability is another model giving private text advice to the
acting model.

That means STT, TTS, vision input for text-only models, web, browser, image
generation, MCP, terminal, and approvals are not MoA-first features. They are
Hermes capability configs. MoA belongs later for roles like code reviewer,
security critic, architecture reviewer, plan skeptic, or documentation editor.

## Spark Backing

Prefer Spark-backed provider aliases for capabilities that are model inference:

- `model`, `fallback_providers`, `auxiliary.vision`, `auxiliary.web_extract`,
  `auxiliary.compression`, `auxiliary.approval`, and MoA reference models.
- TTS when Spark exposes an audio speech route, such as an OpenAI-compatible
  `/v1/audio/speech` alias.
- STT only when Spark exposes a real transcription route. Until then, use a
  local or dedicated STT provider and feed the transcript to Spark-backed text
  models afterward.
- Image generation when Spark exposes a healthy image route.

Do not force Spark into capabilities that are not model inference: browser
state, terminal execution, MCP services, web search service adapters, file
editing, cron delivery, and local OS integration remain tool/service configs.

Live Spark route names drift. Example configs should use provider aliases like
`spark-public-beta` or `spark-operator-frontdoor` and placeholder model IDs
until the route is verified from current front-door metrics and smoke evidence.
