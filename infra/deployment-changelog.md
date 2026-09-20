# Deployment evidence

Read current deployment facts from these authorities. Git history retains past
rollout narratives; outstanding work belongs in Linear.

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

## Runner diagnostic deployment — 2026-09-20 UTC
Deployed [PR #972](https://github.com/finitecomputer/finite-mono/pull/972),
revision `c1b66ba309e9bfd8b26d08a085c23451f69a87c9`, serially on Lat5, Lat3, then Lat4.
This preserves the original hosted upgrade-stop error; it does not add retries
or recovery. No Runtime image, Core, or dashboard deployment was included.
| Host | CI build | Deployed NixOS system |
|---|---|---|
| lat5 | [Run 35477708599](https://github.com/finitecomputer/finite-mono/actions/runs/35477708599) | `/nix/store/vs697wc6f56009iqlmsb208m59l9151j-nixos-system-finite-lat-5-26.05.20260719.fd14620` |
| lat3 | [Run 35477709610](https://github.com/finitecomputer/finite-mono/actions/runs/35477709610) | `/nix/store/4ry535pqbfsya8spnisgdfwzpyrd2nhg-nixos-system-finite-lat-3-26.05.20260719.fd14620` |
| lat4 | [Run 35477710510](https://github.com/finitecomputer/finite-mono/actions/runs/35477710510) | `/nix/store/n7dl8wyxx1dfyaz4i4x9jnsd9y5i4cqh-nixos-system-finite-lat-4-26.05.20260719.fd14620` |

`finite-status` before/after evidence confirmed 63 running agents ready, with
Chat, recovery, and rollout checks green. Container counts, Runtime pins, drain
settings, host boot IDs, and containerd/networkd process IDs were unchanged;
all Runner timers were active. Hosted API reconciliation returned to serving
on Lat3/Lat4; Lat5 retained its expected no-eligible-routes state. API handoff
interruption duration was not measured. No new end-user chat round-trip was
performed for this Runner-only change.

Dry activation additionally required a D-Bus reload and tmpfiles re-setup:
content comparison proved unchanged D-Bus rules and only the version-metric
symlink changing in tmpfiles rules. Previous system generations were retained
for rollback. Existing Lat3 model-route drift, the legacy artifactless fleet
row, and alternate-container-CLI collector warnings remained unchanged.

## Disposable release canary cleanup — 2026-09-20 UTC

The owner explicitly authorized permanent removal of these five disposable
Agent Runtimes and their data. Their normal Stop operations had already
completed. This was a fixed-scope operator cleanup, not a change to the public
retirement contract or a new purge API.

| Agent | Host | Runtime |
|---|---|---|
| Access Enrollment Canary | Lat4 | `runtime_76dfe71b1b55a8873c90` |
| Lat5 Canary | Lat4 | `runtime_86af3c895170b9d3be4f` |
| Release Launch Canary | Lat4 | `runtime_8d961c0f0d7f76f76f4c` |
| Lat5 Canary Retry | Lat5 | `runtime_304dc2797ddc2e7d4f7c` |
| Onboarding Release Canary | Lat5 | `runtime_9afeb28785d97a723f9e` |

Core offboarding effects were applied in a row-scoped transaction after exact
owner/project/runtime/host/machine checks, offline-state checks, and rejection
of active controls, pending creation/relocation, ambiguous bindings, or retained
retirement snapshots. Active links were deactivated under the same runtime/link
locks used by lifecycle and relocation writers. Room memberships were archived;
Core/hosted credentials and Finite Private keys were revoked; relay credentials
were removed. Core and hosted-route readers already honor these boundaries.
Provider metadata, creation history, and unrelated project state were retained.

On each host, exact container IDs, ownership labels, stopped state, and `/data`
binds were rechecked under the existing per-runtime operation locks. Shared
mounts, symlink roots, nested mounts, and held writer leases failed closed. Only
the five containers, their runtime data directories, and their host metadata
directories were removed; no shared image or container-store prune ran. After
absence was verified, Core advanced these runtimes to `archived`. Separate audit
events record the explicit discard authorization, fencing, and completed purge;
no recovery snapshot or retirement receipt was fabricated.

Validation used all 32 Core migrations on isolated real Postgres: wrong owner,
changed binding, online runtime, active creation, and prior offboarding rejected;
credential/link/membership effects and replay rejection passed, with an unrelated
runtime unchanged. Synthetic filesystem deletion and ownership/mount/symlink
refusals passed before host dry runs and production execution.

Five slots were released (Lat4: three; Lat5: two). Post-cleanup `finite-status`
reported 59 running agents ready, with Chat, recovery, and rollout checks green.
All unrelated container IDs, states, process IDs, mounts, and runtime directories
on the affected hosts were unchanged. Hosted API checks were serving on Lat4 and
correctly empty on Lat5. No runtime image or service configuration was deployed.
Known legacy fleet-status and alternate-backend collector warnings persisted.

The irreversible boundary was deletion of the authorized runtime data: no new
backup of that test data was made. Core retained audit/history rows; a root-only
record of the modified Core metadata is at
`/var/lib/finitecomputer/operator-evidence/canary-cleanup-20260920/core-before.json`
on Lat2 for partial-operation diagnosis. It does not restore the deleted runtime
data. No shared service backup was purged. Private execution receipts remain in
`.local-state/runner-stop-deploy/canary-cleanup/` in the operator checkout.
