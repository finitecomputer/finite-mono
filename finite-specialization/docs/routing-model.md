# Capability Routing Model

Finite specialization is a small vocabulary for deciding when an agent should
use a dedicated Hermes capability config, an auxiliary model, a tool/service
surface, delegation, or MoA.

## Flow

1. A Finite agent receives a request.
2. The agent identifies which capability is needed.
3. The capability resolves to a Hermes config surface or execution mode.
4. Hermes runs the request through that surface.
5. The acting model writes the final response and owns tool calls.

## First Decision

Use dedicated Hermes config when the capability transforms I/O, calls a service,
uses local software, touches the OS, or needs a tool.

Use MoA only when the capability is another model giving private text advice to
the acting model.

Within the chosen Hermes surface, prefer Spark-backed provider aliases for
model inference when a current route is healthy. Spark is a backing plane, not a
replacement for the capability boundary.

## Capability Families

- `main-model`: the default agent brain (`model`).
- `stt`: speech or voice message transcription (`stt`).
- `tts`: spoken output (`tts` and sometimes `voice`).
- `voice-mode`: microphone interaction and auto-TTS behavior (`voice`).
- `vision-input`: images for a text-only main model (`auxiliary.vision` and
  Hermes vision handling).
- `web`: search and page extraction (`web` plus `auxiliary.web_extract`).
- `browser`: interactive browser automation (`browser` tool/config).
- `image-generation`: image generation/editing (`image_gen`).
- `tools`: terminal, files, code execution, MCP, Home Assistant, cron, and
  other toolsets (`terminal`, `code_execution`, `mcp_servers`, etc.).
- `delegation`: helpers with their own tool loops (`delegation`).
- `cognitive-review`: private text advice from specialist models (`moa`).

## Spark Backing Rule

Good Spark-backed targets:

- `model` and `fallback_providers`;
- `auxiliary.vision` for image-to-text context;
- `auxiliary.web_extract`, `compression`, `approval`, and similar side models;
- `tts` when a Spark audio speech route exists;
- `stt` only after there is a Spark transcription route;
- `image_gen` when a Spark image route exists and Hermes can target it;
- MoA reference models for code review, verification, architecture, and
  planning critique.

Do not represent these as Spark specializations:

- browser clicks or session state;
- terminal, file, or process execution;
- MCP services such as GitHub, Linear, databases, or Figma;
- web search and web extraction transport itself;
- cron delivery and platform messaging.

Those can call models afterward, but the capability owner stays the relevant
Hermes tool/service config.

## MoA Is Secondary

MoA is a good fit for:

- code reviewer;
- security critic;
- architecture reviewer;
- math checker;
- plan skeptic;
- product or UX reviewer;
- documentation editor.

MoA is the wrong first layer for:

- speech-to-text;
- text-to-speech;
- raw image input;
- browser clicks;
- web search;
- database or GitHub access;
- file edits;
- image or video generation.

Those belong to dedicated Hermes config surfaces or toolsets.

## Contract Sketch

```text
request kind -> capability id -> Hermes config surface -> optional provider/model
```

The capability record may also point to an optional MoA preset, but only after
the dedicated config surface is chosen.

When the backing is Spark, the capability record should name an endpoint
reference and a verification requirement instead of baking in a stale live
alias. Current live aliases must be checked from the front door before
materializing config.

## First Implementation Bias

Start by producing safe example configs and explicit capability records.
Automatic routing can come later once the inventory is real and the outcomes are
easy to inspect.
