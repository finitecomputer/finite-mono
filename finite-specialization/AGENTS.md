# Agent Notes

This repo is a loose scaffold for Finite capability specialization through
Hermes configuration.

## Guardrails

- Do not copy live `~/.hermes/config.yaml`, `.env`, `auth.json`, tokens, raw
  traces, or machine-local logs into this repo.
- Keep example configs generic until actual provider aliases, model names,
  endpoints, and gateway choices are confirmed.
- Prefer dedicated Hermes config surfaces before MoA:
  `stt`, `tts`, `voice`, `auxiliary.vision`, `web`, `browser`, `image_gen`,
  `mcp_servers`, `terminal`, `code_execution`, `approvals`, `delegation`,
  `goals`, and `curator`.
- Prefer Spark-backed OpenAI-compatible provider aliases for model-backed
  surfaces when a current healthy Spark route exists. Keep route names
  placeholder/generic unless you have just verified live availability.
- Use MoA only for cognitive specialists that return text advice to the acting
  model. Do not model raw audio, raw image handling, browser automation, search,
  tool execution, or service integrations as MoA reference models.
- Do not force Spark into non-model capabilities. Browser automation, terminal
  execution, MCP, web-search adapters, file edits, cron delivery, and OS
  integration remain tool/service configs even if their outputs feed Spark
  models later.
- Treat Hermes as the acting runtime. The main model or MoA aggregator owns the
  final response and tool calls.
- Prefer capability names and route IDs over hard-coded machine names.
- Keep this repo light until the interface firms up. Docs and config examples
  are the point for now.

## Local Check

```bash
./scripts/check.sh
```
