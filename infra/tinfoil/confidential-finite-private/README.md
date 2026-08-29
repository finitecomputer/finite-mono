# Finite Private GLM-5.3-Flash candidate

This directory prepares the first model-independent `finite-private` Tinfoil
container. The outer infrastructure identity is Finite Private; the internal
model container and API identity remain `glm-5-3-flash`.

`tinfoil-config.glm-5.3-flash.candidate.yml` is deliberately not release-ready.
It fails closed on three external artifacts that can only be created after
review:

1. the modelwrap MPK and matching root hash for the pinned Hugging Face commit;
2. the CI-built GLM-5.3-Flash SGLang wrapper image digest; and
3. the CI-built limiter image digest containing the model-alias router.

The serving command is the official SGLang H200 high-throughput recipe with one
Finite constraint: context is initially capped at the currently proven
393,216-token product ceiling. Speculative decoding is intentionally absent
from the first correctness and capacity run.

The external Tinfoil container must be created as `finite-private`. Because
that changes the generated hostname, issued Runtimes keep working through the
separate CPU-only compatibility bridge prepared in
`../confidential-kimi-k2-6/tinfoil-config.compatibility-bridge.candidate.yml`.
Do not replace the current GPU container until both releases exist and the
rollback procedure in the production cutover runbook is ready.
