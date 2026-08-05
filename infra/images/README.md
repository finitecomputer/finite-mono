# infra/images — container image definitions

Every first-party image is built by CI from this repo and pushed
digest-pinned to GHCR. Nothing is built on a prod box (the pre-cutover
on-host podman flow died with the k3s control plane).

**Post-cutover note (2026-07-09):** lat1 is NixOS now. `finite-saas-core`
runs from the nix-built binary (the `finite-saas-core` package), NOT the
container image — the core image below is retained for provenance / other
contexts. The dashboard runs as a digest-pinned oci-container (podman) on
lat1. `private-limiter` is the Tinfoil surface; the one Agent Runtime image
targets Kata first and Phala next. See `infra/nixos/` for what lat1 actually
runs.

| Image (ghcr.io/finitecomputer/…) | Definition | Built by | Deployed to |
|---|---|---|---|
| `finite-saas-core` | `core.Dockerfile` (context: repo root) | `service-images.yml` | (retained; lat1 runs the nix binary, not this image) |
| `finite-saas-dashboard` | `dashboard.Dockerfile` (context: repo root; includes the shared Finite Chat UI package) | `service-images.yml` | lat1 (podman oci-container, digest-pinned in `modules/dashboard.nix`) |
| `private-limiter` | `private-limiter.Dockerfile` (context: repo root) | `service-images.yml` | Finite Private Tinfoil CVM (digest pinned in confidential-kimi-k2-6) |
| `deepseek-v4-vllm` | `deepseek-v4-vllm.Dockerfile` (official vLLM 0.25.1 base plus the source-hash-guarded upstream 0731 reasoning fix) | `deepseek-v4-vllm-image.yml` | Prepared Finite Private DeepSeek-V4-Flash-0731 retry candidate; publishing and deployment are manual, separately authorized steps |
| `finite-specialization-worker` | `specialization-worker.Dockerfile` (context: repo root) | `service-images.yml` | shared AEON capability worker on clawland (digest-pinned Kubernetes deployment) |
| `agent-runtime` | `finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile` via `finitecomputer-v2/scripts/build_runtime_image.py` (one staged monorepo + root lockfile) | `runtime-image.yml`, whose build-once smoke proves the exact local image ID before push; `hermes-runtime-smoke.yml` is optional source preflight | local Docker, Kata, Phala, and agent canary lanes |

Legacy package names (`finite-private-limiter`, `finite-agent-runtime`,
`finite-chat-hermes-runtime`) are write-locked to the archived repos that
created them. Decision (Paul, 2026-07-09): no cross-grants — those packages
are FROZEN, kept public so already-deployed pins keep pulling (live Phala
CVMs, the deployed Tinfoil limiter). Mono publishes under the mono-owned
names above; consumers repoint at their next natural roll. Never delete the
frozen packages while any deployed digest references them.

Notes:

- `runtime.Dockerfile` stays next to `build_runtime_image.py` because the
  script assembles its own staged build context and references that path.
- `finitechat/containers/agent/Dockerfile` remains a component test fixture;
  it is not a second publishable product Runtime.
- The self-hosted-runner workflows run on the `finite-lat-2-mono` runner
  (registered 2026-07-09; lat2 is the CI runner box now).
- The DeepSeek vLLM compatibility image is deliberately separate from the
  ordinary service-image matrix because its CUDA base is large and its release
  cadence follows an upstream model/runtime compatibility boundary. The
  workflow verifies vLLM `0.25.1`, the exact upstream reasoning-fix label, and
  the installed patched-source hashes before reporting a usable digest.
- Version tags are date-based for images (`2026-07-08.1`); every push also
  gets a `sha-<git sha>` tag and the workflow summary prints the pinned
  `name:tag@digest` to use in manifests.
