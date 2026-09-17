# Finite Private model identities

Current configuration authority is the [Finite Private configuration](confidential-finite-private/README.md).
Inspect live status before operating; this table defines names, not deployed tags.

| Name | Contract |
| --- | --- |
| `finite-private` | Stable container identity and generated route |
| `glm-5-3-flash` | Canonical model name |
| `glm-5.3-flash` | Limiter alias |
| `glm-5-2`, `deepseek-v4-flash-0731` | Retained mixed-version request aliases |
| `kimi-k2-6` | Historical satellite/hostname; retained rollback configuration lives in its directory |

Sealed secret names are `VLLM_API_KEY`, `VLLM_INTERNAL_API_KEY`, and
`FINITE_USAGE_API_SERVICE_KEY`. A degraded allowlist uses separately authorized
configuration. Desired config, live measured release and per-Runtime endpoint
are distinct facts; verify all affected readers before changing a route.
