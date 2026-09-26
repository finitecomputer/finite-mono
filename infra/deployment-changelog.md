# Deployment evidence

Read current deployment facts from these authorities. Git history retains past
rollout narratives; outstanding work belongs in Linear.

For production questions, check the checkout revision against current `main`
and the deployed revision before using its documentation. Read `scripts/finite-status`
on the app plane and relevant Runner hosts for current observations. An old
worktree or an `UNKNOWN` result from an operator laptop does not establish
production state.

## Where the facts live

| Surface | Source of truth | How to read it |
|---|---|---|
| Dashboard image | `infra/nixos/modules/dashboard.nix` (`image = …@sha256:…`) | `git log -- infra/nixos/modules/dashboard.nix`; on lat2 `podman inspect finite-saas-dashboard --format '{{.ImageDigest}}'` |
| Agent Runtime image for **new** launches | Core's promoted runtime-artifact record plus `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `/etc/finite/runner.env` on each Kata host (lat3, lat4, lat5) | `scripts/finite-status` reports the pin per host; promotion per [`runbooks/runtime-image.md`](runbooks/runtime-image.md) |
| Agent Runtime image for **existing** Agents | Core's per-Runtime record — Agents pin at launch and never auto-update | `scripts/finite-status`; serial upgrades per [`runbooks/runtime-image.md`](runbooks/runtime-image.md) §4a |
| CLI releases (`finitechat`, `fsite`, `fbrain`) | component-scoped source tags in finite-mono and public rolling alias releases in `finitecomputer/finite-releases` (`finitechat-latest`, `fsite-latest`, `fbrain-latest`) | `gh release list --repo finitecomputer/finite-releases`; `git tag -l 'finitechat/*' 'fsite/*' 'fbrain/*'` |
| Server binaries on lat2 (Core, chat, Hosted Device, Brain, Identity) | the NixOS closure built from `infra/nixos/` at the deployed revision | `readlink -f /run/current-system` on the host; `scripts/finite-status` |
| Finite Sites | the live Fly Machine image digest and configuration; desired configuration in `infra/fly/sites/fly.toml` | Machine inspection and `scripts/finite-status --sites-backup-state` per [`runbooks/deploy-sites.md`](runbooks/deploy-sites.md) |
| Finite Private (Tinfoil) | [`tinfoil/model-inventory.md`](tinfoil/model-inventory.md) plus checked-in candidate configs under `infra/tinfoil/` | `just finite-private-deepseek-contract` |
| Phala canary Runtime | `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `/etc/finite/phala-runner.env` | the worker credential environment supplies the pin; [`runbooks/phala-confidential-runner.md`](runbooks/phala-confidential-runner.md) |

## Compatibility boundaries

Hosted Agents pin their Runtime image at launch and do not auto-update. Guarded
same-volume upgrades preserve the Agent Principal and `/data`.

Server compatibility includes fielded clients, persisted state, and supported
recovery sets. Record required reader/writer compatibility in the owning
contract and tests, rather than duplicating version tables here.
