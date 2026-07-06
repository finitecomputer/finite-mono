# Carry-Over Manifest

Status: active split manifest.

Date: 2026-07-02

## Copied Into v2

| Path | Source | Why copied | First cleanup |
| --- | --- | --- | --- |
| `apps/dashboard` | `finitecomputer-worktrees/saas-workos-microsandbox` | WorkOS/SaaS dashboard work lives here. | Dashboard chat, dashboard-managed Published Apps, dashboard-managed Connections, OpenRouter fallback, and machine publish/repo API routes were removed. Next cleanup: reduce remaining legacy local-machine model assumptions. |
| `crates/finite-saas-core` | same | Core state/API for SaaS Projects, runtime launches, Finite Private grants. | Remove imports that depend on legacy `finite-core` chat/control-plane models. |
| `crates/finite-saas-runner` | same | Runner worker for launching real Agent Runtimes. | Make Phala the default runner implementation path; remove Docker-only assumptions where wrong. |
| `crates/finite-private-limiter` | same | Private inference limiter currently lives inside finitecomputer. | TODO: extract to its own repo/deploy boundary later. Preserve issued limiter token/grant state; do not preserve old Core deploy lanes around it. |
| `crates/finite-core` | same | Required today by `finite-saas-core`; copied only to keep the initial source slice coherent. | Split/replace with v2-owned DTOs and delete legacy chat/control-plane code. |
| `deploy/finite-computer` | same | Current finite-lat-1/2 deploy/runtime image files for Core, dashboard, runner, limiter. | Rename to v2, add Phala deployment artifacts, and keep legacy finitec runtime-template fallbacks out. |
| `.github/workflows/finite-private-limiter-image.yml` | same | Existing image publishing workflow. | Replace with v2 image workflows for Core, dashboard, limiter, and runtime image. |

## Explicitly Not Copied

| Legacy path | Reason |
| --- | --- |
| `crates/fc` | Contains old broad `finitec` CLI. v2 needs a minimal runtime adapter, not the monolith. |
| `crates/finited` | Legacy host control plane for box1/TRF-style operations. |
| `crates/finite-ops` | Legacy/operator tooling until proven v2 needs a focused subset. |
| `nix/modules/host-*` | Host-specific NixOS/k3s/box operations belong to legacy or new focused infra docs, not v2 app code. |
| `workspaces/*` | Existing production host configs stay with legacy while users are unmigrated. |
| `apps/electron-shell` | Dashboard chat/web shell is not part of v2 launch. |
| `node_modules`, `.next`, `target`, secrets | Build outputs and local state. |

## External Repos, Not Vendored

| Repo | v2 relationship |
| --- | --- |
| `finite-sites` | Deployed/integrated service; owns Project Repositories and publishing through `fsite`. |
| `finitechat` | Deployed/integrated service; owns Finite Chat server, protocol, CLI/core, native client, and Hermes plugin. |
| `finite-skills` | Canonical Finite-specific skill source. Should move to a Finite Sites Project Repository when ready. |
| `finite-brain` | Future deployed/integrated service for knowledgebase sharing. |

## First Acceptance Gate

Before this repo is treated as authoritative:

- `cargo test --workspace` passes or has a documented, bounded failure list.
- `cd apps/dashboard && npm test && npm run lint && npm run build` passes after
  legacy dashboard surfaces are cut.
- v2 docs no longer refer to box1/TRF as the normal deploy target.
- Core can create a Project and runtime launch request.
- Runner can launch the real runtime image in local/remote Docker, then Phala.
- Dashboard can display a Finite Chat invite with no PIN.
- Runtime can chat through Finite Chat Hermes plugin, use Finite Private, sync
  skills, and publish through `fsite`.

## Data Carry-Forward

Only Finite Private limiter state is a mandatory carry-forward from the old Core
world: grants, API-key hashes/tokens, reservations, usage counters, and audit
events. Other legacy Core runtime/deploy state should be deleted, re-created, or
bridged only by an explicit migration doc with a delete condition. The parked
existing-user import bridge is documented in
[`existing-user-import-bridge.md`](existing-user-import-bridge.md).
