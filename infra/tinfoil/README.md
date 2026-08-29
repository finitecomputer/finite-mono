# Tinfoil satellite repos

Tinfoil confidential-compute deploys are measured against a
`tinfoil-config.yml` at the root of a public GitHub repo. There is one config per
repo. That means each enclave keeps a thin public "satellite" repo even
though finite-mono itself is public: the measurement is per-repo-root, so
multiple enclaves cannot share this repo.

Mono's job is to produce and pin the satellites' inputs; the satellites' job
is to be measurable.

The current model/container/alias map and retired lab state are recorded in
[`model-inventory.md`](model-inventory.md).

## The satellites

| Repo | Enclave | Inputs pinned from mono |
|---|---|---|
| `finitecomputer/confidential-finite-private` | Finite Private GLM-5.3-Flash inference (8×H200) + finite-private-limiter shim (:8002) | Live as external container `finite-private` (`v2026-08-28-glm-5-3-flash-4`). Canonical model `glm-5-3-flash`; aliases `deepseek-v4-flash-0731`, `glm-5-2`, and dotted `glm-5.3-flash`. |
| `finitecomputer/confidential-kimi-k2-6` | Historical Finite Private satellite (DeepSeek V4 Flash 0731, 8×H200) | The generated hostname is retired. DeepSeek rollback still recreates from this satellite under the historical name. Measured 128/2048 scheduler candidate lives under `infra/tinfoil/confidential-kimi-k2-6/`. |
| `finitecomputer/finite-searxng-tinfoil` | Token-gated SearXNG | Config/proxy sources under `finite-search/tinfoil/searxng-public/` in this repo (that dir mirrors the satellite's content, including its release workflows). |
| `finitecomputer/tinfoil-agent-runtime-canary` | Agent runtime canary | The same `ghcr.io/finitecomputer/agent-runtime@sha256:...` digest proved and published by the canonical mono workflows; no Hermes-only rebuild. |

## Update flow (limiter example)

1. Change limiter code in `finitecomputer-v2/crates/finite-private-limiter`.
2. Run the `Service Images` workflow (image=`private-limiter`) → CI pushes
   `ghcr.io/finitecomputer/private-limiter:<version>@<digest>` (mono-owned
   package; the legacy finite-private-limiter package stays frozen).
   Do not reuse the earlier 2026-07-09 mono image: its source predates the
   legacy-parity import. Build a fresh digest from the exact merged SHA.
3. Update the digest pin in `confidential-finite-private`'s config; its measured
   release workflow produces the new enclave release.
4. Follow `infra/runbooks/finite-private-limiter-mono-switch.md`. The ops
   wrapper now lives at `infra/runbooks/finite-private-ops.sh` and requires an
   exact approved tag in `FINITE_PRIVATE_RELAUNCH_APPROVED` before it will run
   the mutating relaunch command. Expect about 35 minutes of downtime.

Do not revert `FINITE_ADMISSION_MODE=allowlist` while `FINITE_USAGE_API_URL`
(`https://finite.computer` in the measured config) is an HTML outage origin.
Public `GET /internal/finite-private/v1/health` 307s to the Vercel outage
page; Core still answers that path as JSON from lat2 at `64.34.80.19` when
the hostname is pointed there. Restore that origin — or split API paths off
the outage page — before switching back to `usage-api`. The limiter must
treat only Core's JSON `{"ok": true}` as usage-API health, never an HTML 200.

## Secrets

Tinfoil sealed secrets (`FINITE_USAGE_API_SERVICE_KEY`, `VLLM_INTERNAL_API_KEY`,
`VLLM_API_KEY`) are set through the Tinfoil deployment surface, never in git.
