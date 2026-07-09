# Service Dependencies

Status: active planning note.

v2 is the SaaS control plane and dashboard. It does not absorb every Finite
service into one repo.

## Services

| Service/repo | Runtime role | v2 responsibility |
| --- | --- | --- |
| `finite-sites` | Project Repositories, `fsite`, website/document publishing, hosted Git HTTP. | Deploy or point users/runtimes at the production Finite Sites API. Track required features such as bare repos and managed-skills hosting. |
| `finitechat` | Encrypted chat server, protocol, native clients, CLI/core, Hermes plugin. | v2 owns hosted server deployment mechanics and runtime integration. The `finitechat` repo owns source, protocol compatibility, app/client release gates, and the Hermes plugin. Display invites; never reimplement chat in dashboard. |
| `finite-skills` | Finite-specific Hermes skill content. | Ensure runtime sync points to the canonical source and skills match installed binaries. The dashboard skills catalog reads `FC_FINITE_SKILLS_SOURCE_DIR` locally, a sibling `../finite-skills` checkout during development, or the finite-skills GitHub tree/raw files in hosted mode. |
| `finite-brain` | Agent knowledgebase sharing. | Integrate after the core SaaS launch path works. |
| Finite Private limiter | Private inference gate. | v2 owns source and deploy now. Preserve issued limiter token/grant state across the v2 hard cut. TODO: extract to its own repo/deploy boundary after the v2 Core grant contract and image release path are stable. |
| Phala | Confidential Runner implementation. | Launch and inspect Agent Runtimes through the runner boundary. |

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
- Do not use dashboard chat as a substitute for Finite Chat.
- Do not use `finitec repo` or `finitec publish` as migration shortcuts.
