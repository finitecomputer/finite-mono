# Finite Private model and container inventory

Observed 2026-08-07. This is an organization map, not deployment authority;
re-run the canonical fleet status and Tinfoil read-only status before acting.

| Name | Role now | Production status |
| --- | --- | --- |
| Finite Private | Stable product/service identity | Production |
| DeepSeek V4 Flash 0731 | Current Finite Private model, canonical API name `deepseek-v4-flash-0731` | Running on eight H200s at `control.inf9.tinfoil.sh` |
| `kimi-k2-6` | Historical Tinfoil container, satellite repo, and generated hostname | Still the production infrastructure route; it does not identify the current model |
| GLM 5.2 | Mixed-version API alias `glm-5-2` and older recovery evidence | Not the canonical product model |
| Kimi K2 | Previously served model that gave the container its name | Not current production |
| Laguna S2.1 | Isolated eight-H200 lab candidate | Never production; recipe/evidence preserved on GPU-lab PR #461 |
| Inkling Small | Isolated eight-H200 lab candidate | Never production; recipe/evidence preserved on GPU-lab PR #461 |

## Current production identity

- container UUID: `a1220ca5-1064-4b15-99a4-5c6ad0b45e07`;
- current tag: `v2026-08-05-deepseek-v4-flash-0731-retry-2-3`;
- host: `control.inf9.tinfoil.sh`;
- allocation: eight H200s;
- debug mode: false;
- sealed secret names: `VLLM_API_KEY`, `VLLM_INTERNAL_API_KEY`, and
  `FINITE_USAGE_API_SERVICE_KEY`;
- generated route:
  `https://kimi-k2-6.finite.containers.tinfoil.dev`.

The best isolated-rack candidate keeps all identities above except its new
release tag and changes the scheduler from 64/512 to 128/2,048. Its measured
result was 8,373 aggregate output tokens/sec at 1,024 concurrent requests,
with approximately 55 output tokens/sec retained for one session.

## Retired temporary state

The stopped, non-production debug container `deepseek-v4-debug`
(`4089eb1d-481b-401c-b171-e75542f5a9af`) was permanently deleted on
2026-08-07 after its exact release/config evidence had been preserved in Git.
It had no production secrets and was never the production route. Tinfoil
metrics showed approximately 1 hour 44 minutes of earlier running time; stopped
containers pause billing, but this repository does not assert an invoice
amount.

## Naming rule

Model names may change; product and route names should not. The next container
identity is `finite-private`, behind the preferred custom route
`inference.finite.computer`. Migrate the route and every issued Runtime reader
before replacing the historical container. See
[`finite-private-routing-migration.md`](../runbooks/finite-private-routing-migration.md).
