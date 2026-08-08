# Tinfoil satellite repos

Tinfoil confidential-compute deploys are measured against a
`tinfoil-config.yml` at the ROOT of a PUBLIC GitHub repo — one config per
repo. That means each enclave keeps a thin public "satellite" repo even
though finite-mono itself is public: the measurement is per-repo-root, so
multiple enclaves cannot share this repo.

Mono's job is to produce and pin the satellites' inputs; the satellites' job
is to be measurable.

## The satellites

| Repo | Enclave | Inputs pinned from mono |
|---|---|---|
| `finitecomputer/confidential-kimi-k2-6` | Finite Private inference (currently DeepSeek V4 Flash 0731, 8×H200) + finite-private-limiter shim (:8002) | Read-only Tinfoil status on 2026-08-08 reported the measured 64/512 baseline tag `v2026-08-05-deepseek-v4-flash-0731-retry-2-3` ready on `control.inf9.tinfoil.sh`. The isolated-lab 128/2048 scheduler winner remains an undeployed candidate; see the [campaign handoff](../../docs/research/2026-08-07-eight-h200-serving-campaign.md). |
| `finitecomputer/finite-searxng-tinfoil` | Token-gated SearXNG | Config/proxy sources under `finite-search/tinfoil/searxng-public/` in this repo (that dir mirrors the satellite's content, including its release workflows). |
| `finitecomputer/tinfoil-agent-runtime-canary` | Agent runtime canary | The same `ghcr.io/finitecomputer/agent-runtime@sha256:...` digest proved and published by the canonical mono workflows; no Hermes-only rebuild. |

## Update flow (limiter example)

1. Change limiter code in `finitecomputer-v2/crates/finite-private-limiter`.
2. Run the `Service Images` workflow (image=`private-limiter`) → CI pushes
   `ghcr.io/finitecomputer/private-limiter:<version>@<digest>` (mono-owned
   package; the legacy finite-private-limiter package stays frozen).
   Do not reuse the earlier 2026-07-09 mono image: its source predates the
   legacy-parity import. Build a fresh digest from the exact merged SHA.
3. Update the digest pin in `confidential-kimi-k2-6`'s config; its measured
   release workflow produces the new enclave release.
4. Follow `infra/runbooks/finite-private-limiter-mono-switch.md`. The ops
   wrapper now lives at `infra/runbooks/finite-private-ops.sh` and requires an
   exact approved tag in `FINITE_PRIVATE_RELAUNCH_APPROVED` before it will run
   the mutating relaunch command. Expect about 35 minutes of downtime.

## Secrets

Tinfoil sealed secrets (`FINITE_USAGE_API_SERVICE_KEY`, `VLLM_INTERNAL_API_KEY`,
`VLLM_API_KEY`) are set through the Tinfoil deployment surface, never in git.
