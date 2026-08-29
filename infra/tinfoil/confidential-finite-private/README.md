# Finite Private GLM-5.3-Flash candidate

This directory is the model-independent `finite-private` Tinfoil container.
The outer infrastructure identity is Finite Private; the internal model
container and API identity remain `glm-5-3-flash`.

`tinfoil-config.glm-5.3-flash.candidate.yml` is the usage-api production
default: checkpoint, MPK, images, and serving command are digest-pinned.
Live `finite-private` currently runs the temporary degraded-allowlist overlay
in `tinfoil-config.glm-5.3-flash.degraded-allowlist.yml` because usage
admission on `finite.computer` is missing. Do not promote the overlay over
the candidate. Revert path:
`docs/runs/glm-5-3-flash-degraded-admission.md`.

The serving command is the official SGLang H200 high-throughput recipe plus
Finite's 393,216-token product ceiling, the LMSYS-measured Hopper DSA pair
(`flashmla_sparse` / `fa3`), and `--chunked-prefill-size 16384` from the
same 8xH200 A/B. Speculative decoding stays off until a separate measured
window. `--mamba-full-memory-ratio` stays at the default until the two pool
sizes are readable off this CVM.

The external Tinfoil container is `finite-private`. Issued Runtimes that
still call the historical hostname need the CPU-only compatibility bridge
in `../confidential-kimi-k2-6/tinfoil-config.compatibility-bridge.candidate.yml`.
Replace procedure: `../../runbooks/finite-private-glm-5.3-flash-production-cutover.md`.
