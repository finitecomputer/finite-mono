# Finite Private model and container inventory

Observed 2026-08-29. This is an organization map, not deployment authority;
re-run the canonical fleet status and Tinfoil read-only status before acting.

| Name | Role now | Production status |
| --- | --- | --- |
| Finite Private | Stable product/service identity | Production |
| GLM-5.3-Flash | Current Finite Private model, canonical API name `glm-5-3-flash` | Live on eight H200s at `control.inf9.tinfoil.sh` as container `finite-private` (`v2026-08-28-glm-5-3-flash-4`) |
| DeepSeek V4 Flash 0731 | Previous Finite Private model; limiter alias `deepseek-v4-flash-0731` | Not the serving workload; rollback tag `v2026-08-13-deepseek-v4-flash-0731-128-2048-1` |
| `kimi-k2-6` | Historical Tinfoil container, satellite repo, and generated hostname | Retired 2026-08-28; generated route is dark. Issued Runtime readers still need the follow-up cutover onto `finite-private` |
| GLM 5.2 | Mixed-version API alias `glm-5-2` and older recovery evidence | Not the canonical product model |
| `glm-5.3-flash` | Dotted Z.ai spelling | Limiter alias only; wire name is hyphenated `glm-5-3-flash` |
| Kimi K2 | Previously served model that gave the container its name | Not current production |
| Laguna S2.1 | Isolated eight-H200 lab candidate | Never production; recipe/evidence preserved on historical [GPU-lab PR #461](https://github.com/finitecomputer/finite-mono/pull/461) |
| Inkling Small | Isolated eight-H200 lab candidate | Never production; recipe/evidence preserved on historical [GPU-lab PR #461](https://github.com/finitecomputer/finite-mono/pull/461) |

## Current production identity

- container UUID: `2aa4d230-0675-4c4a-a7b3-07776b24bfad`;
- current tag: `v2026-08-28-glm-5-3-flash-4`;
- host: `control.inf9.tinfoil.sh`;
- allocation: eight H200s;
- debug mode: false;
- sealed secret names: `VLLM_API_KEY`, `VLLM_INTERNAL_API_KEY`,
  `FINITE_USAGE_API_SERVICE_KEY`, and the temporary
  `FINITE_ADMISSION_ALLOWLIST`;
- generated route:
  `https://finite-private.finite.containers.tinfoil.dev`.
- DeepSeek rollback tag (recreate under the historical name via `--replace`):
  `v2026-08-13-deepseek-v4-flash-0731-128-2048-1`.

Live GLM speed numbers live in
`docs/runs/glm-5-3-flash-degraded-admission.md`. The older DeepSeek
128/2,048 scheduler result (8,373 aggregate tok/s at 1,024 concurrent) is
historical rollback evidence, not the current box.

## Retired temporary state

The stopped, non-production debug container `deepseek-v4-debug`
(`4089eb1d-481b-401c-b171-e75542f5a9af`) was permanently deleted on
2026-08-07 after its exact release/config evidence had been preserved in Git.
It had no production secrets and was never the production route. Tinfoil
metrics showed approximately 1 hour 44 minutes of earlier running time; stopped
containers pause billing, but this repository does not assert an invoice
amount.

## Naming rule

Model names may change; product and route names should not. The GPU
container identity is `finite-private`. The preferred custom route remains
`inference.finite.computer` and is still unattached. The historical
`kimi-k2-6` generated route is retired; the CPU compatibility bridge was
deleted rather than iterated. Issued Runtime readers still need a follow-up
onto `finite-private`. See
[`finite-private-routing-migration.md`](../runbooks/finite-private-routing-migration.md)
and
[`finite-private-glm-5.3-flash-production-cutover.md`](../runbooks/finite-private-glm-5.3-flash-production-cutover.md).
