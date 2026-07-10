# Service Dependencies

Status: active planning note.

v2 is the SaaS control plane and dashboard. It does not absorb every Finite
service into one repo.

## Services

| Service/repo | Runtime role | v2 responsibility |
| --- | --- | --- |
| `finite-sites` | Project Repositories, `fsite`, website/document publishing, hosted Git HTTP. | Deploy or point users/runtimes at the production Finite Sites API. Keep publishing and repository features in this service rather than adding Runtime Management commands. |
| `finitechat` | Encrypted chat server, protocol, Hosted Web Device, native clients, CLI/core, Hermes plugin. | v2 owns Hosted Web Device and server deployment mechanics plus runtime integration. The `finitechat` tree owns source, protocol compatibility, device behavior, and the Hermes 0.18.2 resident sidecar. Production uses the held inbound stream with bounded reconnect and no Python polling or CLI-per-message fallback. |
| `finite-skills` | Finite-managed Hermes baseline. | Treat the monorepo tree as the only editable source and bundle one tested snapshot into the canonical Runtime image. A fresh agent installs it once; restart and image replacement do not overwrite it. Existing agents update only through explicit `finite skills sync`. Core, Runner, RMP, and automatic polling have no skills role. The dashboard's current local/sibling/GitHub catalog fallback is migration debt, not Runtime status. |
| Canonical Agent Runtime image | Hermes, Finite CLIs, Finite Chat plugin, and fresh-agent skills baseline. | Build and promote one source-SHA/digest through `.github/workflows/runtime-image.yml` and `finitecomputer-v2/scripts/build_runtime_image.py`. Docker, Kata, and Phala consume that same digest; no service or Runner gets a feature-specific image lane. |
| `finite-brain` | Agent and user knowledgebase sharing. | Deploy on lat1 under Account Auth and expose the smoke-proven Brain experience in the dashboard as part of the launch path. |
| Finite Private limiter | Private inference gate. | v2 owns source and deploy now. Preserve issued limiter token/grant state across the v2 hard cut. TODO: extract to its own repo/deploy boundary after the v2 Core grant contract and image release path are stable. |
| Kata/containerd | First production Runner implementation. | Launch isolated Agent Runtimes through the generic Runner contract while treating the privileged host operator as trusted. |
| Phala | Confidential Runner fast-follow implementation. | Launch and inspect Agent Runtimes through the same runner/recovery boundary; claim stronger privacy only after attestation, key release, volume, backup, and restore evidence pass. |
| Enclavia | Confidential Runner evaluation target. | Push the promoted Agent Runtime image to a pre-created enclave and inspect the hosted proxy/attestation surface before considering it for production scheduling. |
| Future off-host Recovery Snapshot storage and recovery-key service | Provider-independent copies of Agent Runtime and service Recovery Sets. | Post-MVP TODO/open question: evaluate Restic and alternatives, key backup, Recovery Authorities, and empty-target restore without turning the Runtime Management Pipe into a backup API. |

## Finite Private Routing Debt

GLM 5.2 is currently deployed behind the historical
`https://kimi-k2-6.finite.containers.tinfoil.dev/v1` limiter URL. Keep that URL
as the runtime default until the limiter has a renamed GLM route and the issued
Finite Private token population has a recorded rollout. Do not "clean up" the
URL name in v2 defaults as a cosmetic change.
The domain name is historical only: the endpoint now serves model `glm-5-2`
(limiter `/health` reports `defaultModel: glm-5-2`), and `glm-5-2` is the
runner's `DEFAULT_FINITE_PRIVATE_MODEL`.

## Core Continuity Boundary

Do not preserve old Core/deploy-lane state for its own sake. The only old Core
data that must survive the v2 hard cut is Finite Private limiter state:
grants, API-key hashes/tokens, reservations, usage counters, and audit events
needed to keep already issued private-inference keys valid.

## Non-Goals

- Do not copy Finite Sites code into this component. (Since the finite-mono
  cutover, `finite-sites/` is a sibling directory in the same repo — depend on
  it through the root Cargo workspace, never by duplicating its code here.)
- Same for Finite Chat: depend on the sibling `finitechat/` workspace crates,
  do not duplicate them here.
- Dashboard web chat must be a Finite Chat Hosted Web Device, not a substitute
  transport.
- Runtime Management Pipe v1 is outbound generic health and Product Release
  telemetry only. Do not put product features, credentials, chat, skills, or
  lifecycle commands on it.
- Do not use `finitec repo` or `finitec publish` as migration shortcuts.
