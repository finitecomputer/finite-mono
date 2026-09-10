# infra/images — container image definitions

Every first-party image is built by CI from this repo and pushed
digest-pinned to GHCR. Nothing is built on a prod box (the pre-cutover
on-host podman flow died with the k3s control plane).

Current host roles are defined in [the infrastructure overview](../README.md)
and the [lat2 NixOS host configuration](../nixos/hosts/finite-lat-2/default.nix),
not inferred from image names or historical cutover notes. The production app
plane moved to lat2 on 2026-08-29; lat1 is retired. Core runs from the Nix-built
`finite-saas-core` package, while the dashboard runs as a digest-pinned OCI
container. Use [the Core/dashboard deployment runbook](../runbooks/deploy-core.md)
for the CI-built lat2 closure and dashboard image pin. `private-limiter` is the
Tinfoil surface; the Agent Runtime image has its own rollout lifecycle.

| Image (ghcr.io/finitecomputer/…) | Definition | Built by | Deployed to |
|---|---|---|---|
| `finite-saas-core` | `core.Dockerfile` (context: repo root) | `service-images.yml` | (retained; production Core runs from the lat2 NixOS closure) |
| `finite-saas-dashboard` | `dashboard.Dockerfile` (context: repo root; includes the shared Finite Chat UI package) | `service-images.yml` | lat2 (podman OCI container, digest-pinned in `infra/nixos/modules/dashboard.nix`) |
| `private-limiter` | `private-limiter.Dockerfile` (context: repo root) | `service-images.yml` | Finite Private Tinfoil CVM (digest pinned in confidential-finite-private) |
| `finite-sites` | `sites.Dockerfile` (context: repo root; Rust pin from `rust-toolchain.toml`) | `service-images.yml` (`sites`) | Option B / Fly demo candidate; not a production deployment. See [Sites on Fly](../fly/sites/README.md). |
| `glm-5-3-flash-sglang` | `glm-5.3-flash-sglang.Dockerfile` (context: repo root; wraps the exact upstream amd64 manifest with source labels and fail-closed internal auth) | `glm-5.3-flash-sglang-image.yml` | Live Finite Private GLM-5.3-Flash Tinfoil container |
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
- The Runtime's baseline CLIs are defined by
  `finitecomputer-v2/deploy/finite-computer/images/agent-runtime-toolchains.nix`
  (`.#agent-runtime-toolchains` on the root flake). Its `bins` passthru is the
  single authority for which CLI names the image exposes; the build carries it
  as the `AGENT_RUNTIME_TOOLCHAIN_BINS` build-arg and the Dockerfile and
  workflow probes loop over that list — do not enumerate the names elsewhere.
- `runtime.Dockerfile` is the only Agent Runtime Dockerfile in the tree.
  Component tests that need the image consume `build_runtime_image.py`
  output or a published `agent-runtime` tag; do not add a second
  Dockerfile as a "test fixture" (the former
  `finitechat/containers/agent/Dockerfile` drifted from the product image
  and was deleted in ownership audit O12).
- Image workflows run on Depot-managed GitHub Actions runners and Depot remote
  builders; lat2 is not required for Docker CI. Set `DEPOT_PROJECT_ID` as a
  repository variable or secret, or override by lane with
  `DEPOT_SERVICE_IMAGES_PROJECT_ID`, `DEPOT_RUNTIME_IMAGE_PROJECT_ID`, or
  `DEPOT_DEEPSEEK_VLLM_PROJECT_ID`. The workflows authenticate via
  `depot/setup-action` OIDC.
- Version tags are date-based for images (`2026-07-08.1`). The guarded
  workflows first publish and verify a non-production canary tag from the saved
  OCI build. Production `:<version>` and `:sha-<git sha>` tags are promoted from
  that same saved build only when the explicit production-publish input and
  repository variable are both enabled. Workflow summaries print the pinned
  `name@digest` or `name:tag@digest` to use in manifests.
