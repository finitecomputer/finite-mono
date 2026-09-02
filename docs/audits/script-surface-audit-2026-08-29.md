# Script Surface Audit: 2026-08-29

Status: source audit and cleanup record. No production commands were run and
no remote hosts were contacted. The 2026-08-29 cleanup removed stale one-off
and retired scripts after reference cleanup.

This audit inventories the repository's command and automation surface: root
scripts, component scripts, CI workflows, `justfile` recipes, package scripts,
runbook shell helpers, and runtime entrypoints. It is meant to answer two
questions:

1. What kinds of scripts exist, and where do they live?
2. Which script patterns are healthy, under-documented, or risky enough to
   deserve a follow-up hardening pass?

## Counting Method

The inventory used tracked files and counted:

- `.github/workflows/*.yml` and `.github/workflows/*.yaml`.
- Every tracked `justfile`.
- Every tracked `package.json` that declares `scripts`.
- Shebang files.
- `.py`, `.sh`, `.bash`, `.zsh`, `.js`, `.mjs`, `.cjs`, and `.ts` files inside
  command-oriented directories such as `scripts`, `ci_scripts`, `bin`, and
  `runbooks`, plus executable files with script-like extensions.

Tests and fixtures were counted separately from primary command surfaces.
Support data inside script directories, such as schema files under managed-skill
helpers, was counted separately as script support data rather than executable
automation.

Because the repo does not yet have a script registry, these initial counts
should be treated as a high-confidence map for documentation and risk review,
not as a compiler-enforced manifest. The deletion section records files removed
after this initial scan.

## Initial Inventory Summary

| Bucket | Count | Notes |
| --- | ---: | --- |
| Primary command-surface files | 219 | Scripts, workflows, command facades, and runtime entrypoints before the cleanup pass |
| Test/helper script files | 40 | Mostly `scripts/tests` and component test fixtures |
| Support data in script directories | 40 | Mostly non-runnable support files, not command entrypoints |

### Initial Primary Files By Language

| Language/surface | Count |
| --- | ---: |
| Python | 97 |
| Shell | 67 |
| GitHub Actions workflow | 17 |
| `just` | 15 |
| Node/JavaScript | 14 |
| npm package scripts | 5 |
| TypeScript | 4 |

### Initial Primary Files By Owner

| Owner area | Count | Role |
| --- | ---: | --- |
| Root command surface | 68 | Platform, fleet, CI harness, deployment, recovery, and local-dev commands |
| `finitechat` | 39 | Hermes proofs, Electron/iOS packaging, runtime container behavior |
| `finite-skills` | 35 | Managed-skill static checks and product helper scripts |
| GitHub Actions | 15 | CI, release, image, and closure workflows |
| `finite-search` | 14 | Search stack smoke tests, local probes, and static checks |
| `finitecomputer-v2` | 12 | Dashboard, Stripe readiness, runtime-image checks |
| `infra/nixos` | 5 | Host and NixOS operational support |
| `infra` | 4 | Image/ops support outside host-specific trees |
| `infra/tinfoil` | 4 | Tinfoil packaging/deploy helpers |
| `infra/scripts` | 3 | Recovery and restore validation helpers |
| `infra/monitoring` | 3 | Monitoring deployment/runtime scripts |
| `finite-sites` | 3 | Site CLI demos and command facades |
| `devfinity` | 2 | Local integration harness facade |
| `finite-brain` | 2 | Brain command facade and route/product checks |
| `finite-identity` | 2 | Identity CLI/edge contract support |
| `finite-specialization` | 1 | Component command helper |
| `infra/hosts` | 1 | Host-local operational script |
| `infra/runbooks` | 1 | Runbook-adjacent operational helper |

### Initial Primary Files By Purpose

| Purpose | Count | Examples |
| --- | ---: | --- |
| Misc/tooling | 52 | One-off analyzers, local helpers, packaging utilities |
| Check/smoke/status | 51 | `finite-status`, canaries, contract checks, static checks |
| Skill helper/tooling | 32 | Managed-skill helper scripts and packaged tool adapters |
| Command facade | 20 | Root/component `justfile`s and package scripts |
| CI/release workflow | 17 | `.github/workflows/*` |
| Deploy/release/rollout | 17 | Closure deploy, runtime rollout, image workflows |
| Local dev/harness | 14 | `devfinity` scenario and smoke scripts |
| Migration/recovery/bootstrap | 14 | Snapshot, restore, credential install, legacy migration |
| Runtime entrypoint/healthcheck | 2 | Agent container entrypoints and health behavior |

## Lifecycle Classification

These labels answer "what is this script for now?" rather than simply "what
language is it?" A script can still be active even when it is niche or manually
operated. The single bucket below is the most useful owner-facing label for
cleanup and maintenance.

| Lifecycle category | Count | Meaning |
| --- | ---: | --- |
| Current command facade | 18 | Preferred human/agent command entrypoints. |
| Current CI/release automation | 15 | GitHub Actions workflows that run CI, release, image, or closure automation. |
| Current operational/deploy/recovery | 52 | Manual or automated operations touching hosts, releases, credentials, status, or recovery. |
| Current component check/smoke/package | 43 | Component-owned validation, smoke, contract, canary, or packaging commands. |
| Current runtime/image/container support | 12 | Scripts copied into or executed by runtime images, containers, Xcode Cloud, or templates. |
| Current niche helper/tool | 36 | Specialized helpers for managed skills, examples, model evaluation, design fixtures, or external-service workflows. |
| Internal support/library | 19 | Imported, sourced, or wrapped by other commands; not intended as the top-level entrypoint. |
| Legacy/migration/retired | 7 | Explicit legacy/backcompat/import tooling, after removing the retired host deploy script. |
| One-off/proof/repro/research | 9 | Scenario scripts, reproductions, benchmarks, hands-on demos, or research proofs that should be promoted or retired after use. |
| Review candidate | 0 | No current owner signal found from path, facade, docs, tests, or header inspection. |

This lifecycle view intentionally differs from the purpose table above: example
app `package.json` files and the `finite-skills` A/B package are command
facades by file shape, but they are lifecycle-classified as niche helpers rather
than repo-level command facades.

### Lifecycle Catalog

#### Current Command Facade

| Path | Surface | Note |
| --- | --- | --- |
| `devfinity/justfile` | just | preferred command surface |
| `finite-brain/justfile` | just | preferred command surface |
| `finite-identity/justfile` | just | preferred command surface |
| `finite-search/justfile` | just | preferred command surface |
| `finite-sites/justfile` | just | preferred command surface |
| `finite-skills/justfile` | just | preferred command surface |
| `finitechat/apps/electron-chat/package.json` | package | preferred command surface |
| `finitechat/justfile` | just | preferred command surface |
| `finitecomputer-v2/apps/dashboard/justfile` | just | preferred command surface |
| `finitecomputer-v2/apps/dashboard/package.json` | package | preferred command surface |
| `finitecomputer-v2/deploy/finite-computer/images/justfile` | just | preferred command surface |
| `finitecomputer-v2/justfile` | just | preferred command surface |
| `infra/justfile` | just | preferred command surface |
| `infra/monitoring/justfile` | just | preferred command surface |
| `infra/nixos/justfile` | just | preferred command surface |
| `infra/secret/justfile` | just | preferred command surface |
| `justfile` | just | preferred command surface |
| `scripts/with-dev-env` | Shell | Nix environment wrapper |

#### Current CI/Release Automation

| Path | Surface | Note |
| --- | --- | --- |
| `.github/workflows/ci.yml` | workflow | workflow entrypoint |
| `.github/workflows/deepseek-v4-vllm-image.yml` | workflow | workflow entrypoint |
| `.github/workflows/glm-5.3-flash-sglang-image.yml` | workflow | workflow entrypoint |
| `.github/workflows/hermes-runtime-smoke.yml` | workflow | workflow entrypoint |
| `.github/workflows/lat1-nixos-closure.yml` | workflow | workflow entrypoint |
| `.github/workflows/lat2-nixos-closure.yml` | workflow | workflow entrypoint |
| `.github/workflows/lat3-nixos-closure.yml` | workflow | workflow entrypoint |
| `.github/workflows/lat4-nixos-closure.yml` | workflow | workflow entrypoint |
| `.github/workflows/phala-readonly-preflight.yml` | workflow | workflow entrypoint |
| `.github/workflows/release-fbrain.yml` | workflow | workflow entrypoint |
| `.github/workflows/release-finitechat.yml` | workflow | workflow entrypoint |
| `.github/workflows/release-fsite.yml` | workflow | workflow entrypoint |
| `.github/workflows/runtime-image.yml` | workflow | workflow entrypoint |
| `.github/workflows/service-images.yml` | workflow | workflow entrypoint |

#### Current Operational/Deploy/Recovery

| Path | Surface | Note |
| --- | --- | --- |
| `finite-search/scripts/bootstrap-firecrawl-upstream.sh` | Shell | ops, release, host, or recovery path |
| `finite-search/scripts/searxng-token-proxy.py` | Python | ops, release, host, or recovery path |
| `finite-search/tinfoil/searxng-public/scripts/deploy-staging.sh` | Shell | ops, release, host, or recovery path |
| `finitechat/scripts/hermes-github-ci-preflight.py` | Python | ops, release, host, or recovery path |
| `finitechat/scripts/hermes-github-publish-gate.py` | Python | ops, release, host, or recovery path |
| `finitechat/scripts/hermes-github-secrets-setup.py` | Python | ops, release, host, or recovery path |
| `finitechat/scripts/hermes-publish-proven-image.py` | Python | ops, release, host, or recovery path |
| `finitechat/scripts/hermes-tinfoil-canary-artifacts.py` | Python | ops, release, host, or recovery path |
| `finitechat/scripts/hermes-tinfoil-canary-evidence.py` | Python | ops, release, host, or recovery path |
| `finitechat/scripts/hermes-tinfoil-canary-result.py` | Python | ops, release, host, or recovery path |
| `finitechat/scripts/hermes-tinfoil-handoff.py` | Python | ops, release, host, or recovery path |
| `infra/monitoring/ubuntu/deploy` | Shell | ops, release, host, or recovery path |
| `infra/nixos/scripts/capture-lat2-host-evidence` | Shell | ops, release, host, or recovery path |
| `infra/nixos/scripts/capture-lat4-host-evidence` | Shell | ops, release, host, or recovery path |
| `infra/nixos/scripts/check-lat-monitoring-secrets` | Shell | ops, release, host, or recovery path |
| `infra/runbooks/finite-private-ops.sh` | Shell | ops, release, host, or recovery path |
| `infra/scripts/restore-hosted-web-chat-snapshot` | Shell | ops, release, host, or recovery path |
| `infra/scripts/test-hosted-web-chat-restore` | Shell | ops, release, host, or recovery path |
| `infra/scripts/test-litestream-restore` | Shell | ops, release, host, or recovery path |
| `scripts/backfill_releases.py` | Python | ops, release, host, or recovery path |
| `scripts/build-lat1-nixos-closure-artifact` | Shell | ops, release, host, or recovery path |
| `scripts/build-lat2-nixos-closure-artifact` | Shell | ops, release, host, or recovery path |
| `scripts/build-lat3-nixos-closure-artifact` | Shell | ops, release, host, or recovery path |
| `scripts/build-lat4-nixos-closure-artifact` | Shell | ops, release, host, or recovery path |
| `scripts/check-lat1-secret-bootstrap` | Python | ops, release, host, or recovery path |
| `scripts/check_finite_status_contract.py` | Python | ops, release, host, or recovery path |
| `scripts/check_lat1_healthcheck_contract.py` | Python | ops, release, host, or recovery path |
| `scripts/check_monitoring_nixos_contract.py` | Python | ops, release, host, or recovery path |
| `scripts/check_nixos_secrets_contract.py` | Python | ops, release, host, or recovery path |
| `scripts/check_runbook_facts_contract.py` | Python | ops, release, host, or recovery path |
| `scripts/check_runner_host_contract.py` | Python | ops, release, host, or recovery path |
| `scripts/deploy-lat1-closure-cache` | Shell | ops, release, host, or recovery path |
| `scripts/deploy-lat2-closure-cache` | Shell | ops, release, host, or recovery path |
| `scripts/deploy-lat3-closure-cache` | Shell | ops, release, host, or recovery path |
| `scripts/deploy-lat4-closure-cache` | Shell | ops, release, host, or recovery path |
| `scripts/finite-status` | Python | ops, release, host, or recovery path |
| `scripts/install-identity-authority-credentials` | Shell | ops, release, host, or recovery path |
| `scripts/install-identity-sites-notification-credential` | Shell | ops, release, host, or recovery path |
| `scripts/install-lat2-from-artifact` | Shell | ops, release, host, or recovery path |
| `scripts/install-lat4-from-artifact` | Shell | ops, release, host, or recovery path |
| `scripts/install-phala-canary-credentials` | Shell | ops, release, host, or recovery path |
| `scripts/nixos_sops_ingest.py` | Python | ops, release, host, or recovery path |
| `scripts/nixos_sops_operator_key.py` | Python | ops, release, host, or recovery path |
| `scripts/nixos_sops_test_decrypt.py` | Python | ops, release, host, or recovery path |
| `scripts/nixos_sops_updatekeys.py` | Python | ops, release, host, or recovery path |
| `scripts/publish-lat1-nixos-cachix-closure` | Shell | ops, release, host, or recovery path |
| `scripts/rollout-lat1-runtime-artifact` | Shell | ops, release, host, or recovery path |
| `scripts/rollout_preflight.py` | Python | ops, release, host, or recovery path |
| `scripts/snapshot-sqlite` | Shell | ops, release, host, or recovery path |

#### Current Component Check/Smoke/Package

| Path | Surface | Note |
| --- | --- | --- |
| `finite-brain/scripts/verify-smoke-alpha-backup-restore.sh` | Shell | component validation or packaging |
| `finite-identity/scripts/identity-edge-contract-gate.py` | Python | component validation or packaging |
| `finite-search/scripts/check-static.sh` | Shell | component validation or packaging |
| `finite-search/scripts/doctor.sh` | Shell | component validation or packaging |
| `finite-search/scripts/probe-stack.sh` | Shell | component validation or packaging |
| `finite-search/scripts/smoke-firecrawl.sh` | Shell | component validation or packaging |
| `finite-search/scripts/smoke-searxng.sh` | Shell | component validation or packaging |
| `finite-search/scripts/smoke-stack.sh` | Shell | component validation or packaging |
| `finite-search/scripts/smoke-tinfoil-searxng-bundle.sh` | Shell | component validation or packaging |
| `finite-skills/scripts/check-static.sh` | Shell | component validation or packaging |
| `finite-specialization/scripts/check.sh` | Shell | component validation or packaging |
| `finitechat/apps/electron-chat/scripts/package-macos-alpha.mjs` | Node | component validation or packaging |
| `finitechat/scripts/electron-local-agent.sh` | Shell | component validation or packaging |
| `finitechat/scripts/hermes-adapter-regression-report.py` | Python | component validation or packaging |
| `finitechat/scripts/hermes-agent-media-e2e.sh` | Shell | component validation or packaging |
| `finitechat/scripts/hermes-branch-publication-readiness.py` | Python | component validation or packaging |
| `finitechat/scripts/hermes-chat-interruption-docker-smoke.py` | Python | component validation or packaging |
| `finitechat/scripts/hermes-durable-home-docker-smoke.py` | Python | component validation or packaging |
| `finitechat/scripts/hermes-hardening-audit.py` | Python | component validation or packaging |
| `finitechat/scripts/hermes-phone-canary.py` | Python | component validation or packaging |
| `finitechat/scripts/hermes-real-gateway-admission-smoke.py` | Python | component validation or packaging |
| `finitechat/scripts/hermes-sidecar-smoke.sh` | Shell | component validation or packaging |
| `finitechat/scripts/ios-device-hermes-agent-media-e2e.sh` | Shell | component validation or packaging |
| `finitechat/scripts/ios-hermes-agent-media-e2e.sh` | Shell | component validation or packaging |
| `finitechat/scripts/ios-local-agent.sh` | Shell | component validation or packaging |
| `finitechat/scripts/ios-xcode-cloud-preflight.sh` | Shell | component validation or packaging |
| `finitechat/scripts/server-contract-gate.py` | Python | component validation or packaging |
| `finitecomputer-v2/apps/dashboard/scripts/check_stripe_price_contract.py` | Python | component validation or packaging |
| `finitecomputer-v2/apps/dashboard/scripts/stripe-billing-test-clock-e2e.ts` | TS | component validation or packaging |
| `finitecomputer-v2/apps/dashboard/scripts/stripe-production-readiness.ts` | TS | component validation or packaging |
| `finitecomputer-v2/deploy/finite-computer/images/scripts/check_runtime_image_contract.py` | Python | component validation or packaging |
| `finitecomputer-v2/scripts/build_runtime_image.py` | Python | component validation or packaging |
| `infra/images/patch_vllm_deepseek_v4_0731.py` | Python | component validation or packaging |
| `infra/monitoring/ubuntu/check_contract.py` | Python | component validation or packaging |
| `infra/nixos/scripts/finite_runtime_metrics.py` | Python | component validation or packaging |
| `scripts/check-brain-api-routes.py` | Python | component validation or packaging |
| `scripts/check-brain-collaboration-smoke-report.py` | Python | component validation or packaging |
| `scripts/check-brain-product-language.py` | Python | component validation or packaging |
| `scripts/ci/affected-rust-packages` | Python | component validation or packaging |
| `scripts/ci/select-harnesses` | Python | component validation or packaging |
| `scripts/devfinity-restart-process` | Python | component validation or packaging |
| `scripts/devfinity-saas-smoke` | Shell | component validation or packaging |
| `scripts/devfinity-smoke` | Shell | component validation or packaging |

#### Current Runtime/Image/Container Support

| Path | Surface | Note |
| --- | --- | --- |
| `finite-search/tinfoil/searxng-public/auth_proxy.py` | Python | runs inside image/container/CI runtime |
| `finite-search/tinfoil/searxng-public/entrypoint-auth-proxy.sh` | Shell | runs inside image/container/CI runtime |
| `finitechat/containers/agent/entrypoint.sh` | Shell | runs inside image/container/CI runtime |
| `finitechat/containers/agent/finite.py` | Python | runs inside image/container/CI runtime |
| `finitechat/containers/agent/health_server.py` | Python | runs inside image/container/CI runtime |
| `finitechat/containers/agent/probe_hermes_vision.py` | Python | runs inside image/container/CI runtime |
| `finitechat/containers/agent/reconcile_hermes_config.py` | Python | runs inside image/container/CI runtime |
| `finitechat/containers/agent/recover_chat_boot.py` | Python | runs inside image/container/CI runtime |
| `finitechat/containers/agent/run_hermes_gateway.sh` | Shell | runs inside image/container/CI runtime |
| `finitechat/ios/ci_scripts/ci_post_clone.sh` | Shell | runs inside image/container/CI runtime |
| `finitecomputer-v2/deploy/finite-computer/runtime-template/healthcheck.sh` | Shell | runs inside image/container/CI runtime |
| `infra/images/glm-5.3-flash-sglang-entrypoint.sh` | Shell | runs inside image/container/CI runtime |

#### Current Niche Helper/Tool

| Path | Surface | Note |
| --- | --- | --- |
| `finite-sites/examples/nextjs-demo/package.json` | package | example app commands |
| `finite-sites/examples/react-bun-spa/package.json` | package | example app commands |
| `finite-skills/ab-testing/package.json` | package | specialized local helper |
| `finite-skills/ab-testing/scripts/build-review.mjs` | Node | specialized local helper |
| `finite-skills/ab-testing/scripts/eval.mjs` | Node | specialized local helper |
| `finite-skills/ab-testing/scripts/open-review.mjs` | Node | specialized local helper |
| `finite-skills/ab-testing/scripts/run-prompt.mjs` | Node | specialized local helper |
| `finite-skills/ab-testing/scripts/run.mjs` | Node | specialized local helper |
| `finite-skills/ab-testing/scripts/serve-review.mjs` | Node | specialized local helper |
| `finite-skills/skills/leisure/find-nearby-finite/scripts/find_nearby.py` | Python | skill-local helper |
| `finite-skills/skills/leisure/goplaces-finite/scripts/google_places.py` | Python | skill-local helper |
| `finite-skills/skills/productivity/google-workspace-finite/scripts/google_api.py` | Python | skill-local helper |
| `finite-skills/skills/productivity/google-workspace-finite/scripts/setup.py` | Python | skill-local helper |
| `finite-skills/skills/productivity/linear-finite/scripts/linear_api.py` | Python | skill-local helper |
| `finite-skills/skills/productivity/ocr-and-documents-finite/scripts/extract_marker.py` | Python | skill-local helper |
| `finite-skills/skills/productivity/ocr-and-documents-finite/scripts/extract_pymupdf.py` | Python | skill-local helper |
| `finite-skills/skills/productivity/powerpoint-finite/scripts/add_slide.py` | Python | skill-local helper |
| `finite-skills/skills/productivity/powerpoint-finite/scripts/clean.py` | Python | skill-local helper |
| `finite-skills/skills/productivity/powerpoint-finite/scripts/office/pack.py` | Python | skill-local helper |
| `finite-skills/skills/research/arxiv-finite/scripts/search_arxiv.py` | Python | skill-local helper |
| `finite-skills/skills/research/domain-intel-finite/scripts/domain_intel.py` | Python | skill-local helper |
| `finite-skills/skills/research/duckduckgo-search-finite/scripts/duckduckgo.sh` | Shell | skill-local helper |
| `finite-skills/skills/research/model-council-finite/scripts/model_council.py` | Python | skill-local helper |
| `finite-skills/skills/research/perplexity-research-finite/scripts/perplexity_research.py` | Python | skill-local helper |
| `finite-skills/skills/research/polymarket-finite/scripts/polymarket.py` | Python | skill-local helper |
| `finite-skills/skills/social-media/x-api-finite/x-api.py` | Python | skill-local helper |
| `finite-skills/skills/social-media/x-search-finite/x-search.py` | Python | skill-local helper |
| `finitechat/apps/electron-chat/scripts/run-electron-web-design-fixture.mjs` | Node | specialized local helper |
| `finitecomputer-v2/apps/dashboard/scripts/web-design-fixture.ts` | TS | specialized local helper |
| `scripts/check_deepseek_v4_0731_quality.py` | Python | model evaluation contract |
| `scripts/check_finite_private_deepseek_candidate.py` | Python | model evaluation contract |
| `scripts/check_finite_private_glm53_candidate.py` | Python | model evaluation contract |
| `scripts/check_finite_private_glm53_capacity.py` | Python | model evaluation contract |
| `scripts/check_finite_private_glm53_protocol.py` | Python | model evaluation contract |
| `scripts/check_finite_private_glm53_quality.py` | Python | model evaluation contract |
| `scripts/check_finite_private_load_comparison.py` | Python | specialized local helper |

#### Internal Support/Library

| Path | Surface | Note |
| --- | --- | --- |
| `devfinity/scripts/agent-run.mjs` | Node | imported/sourced by other commands |
| `finite-skills/ab-testing/scripts/devfinity-cleanup.mjs` | Node | imported/sourced by other commands |
| `finite-skills/ab-testing/scripts/process.mjs` | Node | imported/sourced by other commands |
| `finite-skills/ab-testing/scripts/run-devfinity-agent-turn.mjs` | Node | imported/sourced by other commands |
| `finite-skills/skills/productivity/google-workspace-finite/scripts/_storage.py` | Python | imported/sourced by other commands |
| `finite-skills/skills/productivity/powerpoint-finite/scripts/__init__.py` | Python | imported/sourced by other commands |
| `finite-skills/skills/productivity/powerpoint-finite/scripts/office/helpers/__init__.py` | Python | imported/sourced by other commands |
| `finite-skills/skills/productivity/powerpoint-finite/scripts/office/helpers/merge_runs.py` | Python | imported/sourced by other commands |
| `finite-skills/skills/productivity/powerpoint-finite/scripts/office/helpers/simplify_redlines.py` | Python | imported/sourced by other commands |
| `finitechat/apps/electron-chat/scripts/generate-macos-update-manifest.mjs` | Node | imported/sourced by other commands |
| `finitechat/scripts/seed-local-chat-stress.mjs` | Node | imported/sourced by other commands |
| `finitecomputer-v2/apps/dashboard/scripts/playwright-browser.ts` | TS | imported/sourced by other commands |
| `scripts/ci/changed_paths.py` | Python | imported/sourced by other commands |
| `scripts/delivery.py` | Python | imported/sourced by other commands |
| `scripts/dev-shell` | Shell | just recipe shell and Nix wrapper |
| `scripts/finite_status.py` | Python | imported/sourced by other commands |
| `scripts/hermes_nix_runtime.py` | Python | imported/sourced by other commands |
| `scripts/lib/devfinity-brain-readiness.sh` | Shell | imported/sourced by other commands |

#### Legacy/Migration/Retired

| Path | Surface | Note |
| --- | --- | --- |
| `scripts/import-sync` | Shell | archived repo import helper |
| `scripts/legacy-hermes-migration` | Shell | legacy migration/backcompat |
| `scripts/legacy-hermes-source` | Shell | legacy migration/backcompat |
| `scripts/legacy_hermes_contract.py` | Python | legacy migration/backcompat |
| `scripts/legacy_hermes_migration.py` | Python | legacy migration/backcompat |
| `scripts/legacy_hermes_source.py` | Python | legacy migration/backcompat |
| `scripts/legacy_hermes_target.py` | Python | legacy migration/backcompat |

#### One-Off/Proof/Repro/Research

| Path | Surface | Note |
| --- | --- | --- |
| `finite-search/scripts/benchmark-stack.sh` | Shell | scenario, repro, benchmark, or research proof |
| `finitechat/scripts/hermes-real-gateway-demo.sh` | Shell | scenario, repro, benchmark, or research proof |
| `scripts/prepare_glm53_blind_comparison.py` | Python | scenario, repro, benchmark, or research proof |

#### Review Candidate

None found after source/reference inspection.

## Command Surface Model

### Root `justfile`

The root `justfile` is the best command-surface pattern in the repo. It makes
`scripts/dev-shell` the shell for recipes, which keeps common commands inside
the pinned Nix environment. It also gives the repo a stable outer interface for
formatting, linting, tests, local integration, CI selection, deploy planning,
NixOS host work, runtime images, and product-specific delegations.

This is the pattern to preserve: humans and agents should use root `just`
commands first, and only drop into raw scripts when the command is intentionally
not wrapped or when debugging a script directly.

### Root `scripts/`

Root `scripts/` is the largest command owner. It contains:

- CI harness selection and changed-path logic.
- Fleet status and operational contract checks.
- Runtime rollout and closure artifact deployment.
- Devfinity smoke scenarios.
- Hosted chat recovery helpers.
- NixOS SOPS helpers.
- Legacy Hermes migration tooling.
- Production deployment plan validation.

The folder has two strong local patterns:

- Python modules with unit tests under `scripts/tests`.
- Shell entrypoints with explicit argument validation, `set -euo pipefail`, and
  fail-closed command phases.

The folder also has the largest documentation gap. A reader cannot currently
answer "which of these are supported public commands?" without reading the root
`justfile`, CI workflows, and the scripts themselves.

### GitHub Actions

The workflows are not just CI glue. They are part of the release and operations
surface:

- `.github/workflows/ci.yml` is the broad PR gate.
- Runtime image and service-image workflows build/publish deployable artifacts.
- NixOS closure workflows prepare host-specific deployment inputs.
- Product release workflows publish component-scoped release assets.
- Production deployment workflows plan, deploy, and open PRs.

Most mutating/promoting workflows are `workflow_dispatch`, which is the right
default for production operations. The remaining risk is fact drift between
workflows and shell/Python deployment scripts.

### Component Command Facades

Most major components have their own `justfile` or package scripts:

- `devfinity/justfile` wraps the local integration stack.
- `finitechat/justfile` wraps Hermes and chat checks.
- `finite-search/justfile` wraps search static checks and smoke probes.
- `finite-sites/justfile` wraps the sites service and examples.
- `finite-skills/justfile` wraps managed-skill checks.
- `finitecomputer-v2/apps/dashboard/package.json` owns dashboard build, lint,
  tests, browser tests, design fixture, and Stripe readiness commands.
- `finitechat/apps/electron-chat/package.json` owns Electron packaging/test
  commands.

This is a healthy ownership pattern. Component-owned commands stay near their
source, while root `just` provides the monorepo-level entrypoint.

### Runtime Entrypoints

Container entrypoints are few but important. The agent entrypoint validates
restore and backup environment, keeps durable identity under the mounted state
root, supervises the child process, starts optional Brain sync supervision, and
performs an orphan process-group sweep before final backup.

This script is appropriately explicit because it is runtime-critical. It should
remain small in count and high in test coverage because entrypoint behavior is
hard to inspect once deployed.

## Healthy Patterns

### Nix-First Execution

The repo-wide ground rule is implemented in the root command layer: common
developer and CI commands enter the pinned Nix environment through `just`,
`scripts/dev-shell`, or `scripts/with-dev-env`. This reduces hidden host-machine
dependency drift.

### Explicit Operational Phases

The strongest deploy scripts use explicit phases:

- Validate local and remote prerequisites.
- Prepare or build a manifest/plan.
- Require an exact artifact, revision, tag, host, or plan hash.
- Execute only after the checked input is named again.

Examples include closure-cache deployment scripts, runtime rollout scripts, and
production deploy plan validation. This pattern is the repo's safest answer to
"production repair is never speculative."

### Read-Only Status Authority

`scripts/finite-status` and `scripts/finite_status.py` centralize operational
names, health probes, systemd units, recovery constants, and read-only fleet
queries. `scripts/check_finite_status_contract.py` keeps those constants
aligned with Nix authorities and rejects mutating SQL in the status queries.

This is exactly the right pattern for platform state: one read-only command,
contract-tested against the declarative owners.

### Safe SQLite Snapshot Inspection

`scripts/snapshot-sqlite` is a strong safety pattern. It refuses direct snapshot
mutation by copying a manifested database and sidecars into a private scratch
directory, using `sqlite3 -safe -readonly`, and enabling `PRAGMA query_only=ON`
for ad hoc queries.

Runbooks and restore scripts already route SQLite inspection through this helper.
That is a pattern worth documenting as mandatory for all snapshot SQLite work.

### Source-Inspection Tests

Many operational scripts have source-level tests under `scripts/tests`. These
tests assert exact strings, command ordering, transaction shape, manifest
requirements, and refusal conditions. That style is useful for scripts whose
real external side effects are too expensive or dangerous for a normal test
run.

### Secret-Handling Guards

Several scripts avoid printing secret values and use mode-restricted temporary
files for API credentials or generated tokens. The Identity Authority installer
is a good example: it verifies the remote host, creates a root-owned `0600`
operator environment file, validates token shape, and prints only the install
location and file mode.

### No Python `shell=True` In Primary Script Areas

A search across the primary script areas found no Python `shell=True` usage.
There are many `subprocess` calls, but they generally pass argument arrays and
check exit status explicitly.

## Main Risks

### 1. No Script Registry Or Owner Map

The repo has 219 primary command-surface files, but no durable inventory that
names the owner, supported entrypoint, mutability, required environment, tests,
or runbook for each command.

That makes scripts expensive for agents to navigate. It also makes it easy for
old operational scripts to remain executable after they stop matching the
current platform.

### 2. Script Authority Is Split Across Too Many Surfaces

Authority lives in root `just`, component `justfile`s, package scripts, CI
workflows, shell scripts, Python modules, runbook shell helpers, and Nix
modules. That is normal for a monorepo, but the current documentation does not
state which layer wins when two surfaces overlap.

The practical rule appears to be:

- Root `just` is the human and agent entrypoint.
- Component facades own local component workflows.
- CI workflows own promotion/release automation.
- Nix owns deployed configuration.
- `scripts/finite-status` owns fleet status.
- Runbooks own procedural context, not reusable command logic.

This rule should be written down.

### 3. Some Executable Logic Lives In Runbooks

`infra/runbooks/finite-private-ops.sh` is a real operational command with
status, liveness, health, canary, load, settlement, wait-ready, and relaunch
subcommands. It has important guards: high-concurrency load requires an exact
approval environment value, relaunch requires `FINITE_PRIVATE_RELAUNCH_APPROVED`
to match the requested tag, and credential-bearing curl config files are
temporary and mode-restricted.

The concern is placement, not necessarily implementation. A script under
`infra/runbooks` blurs the boundary between procedure and reusable tooling. If
this is supported tooling, it should be indexed and probably live under an
explicit script directory with the runbook linking to it. If it is a dated
incident artifact, it should say so loudly.

### 4. Retired Host Scripts Should Leave The Tree

The cleanup removed `infra/hosts/lat1/scripts/deploy-finitechat-server.sh`
after current docs were moved to `infra/runbooks/deploy-finitechat-server.md`.
That is the preferred pattern for retired operational scripts: capture the
historical fact in a current runbook or host note, then delete the executable
script so agents cannot mistake it for an available deploy path.

### 5. Executable Bit And Shebang Policy Is Inconsistent

The scan found many Python and shell files with shebangs that are not executable.
Some are intentionally invoked as modules or through explicit interpreters, so
this is not automatically a bug. The problem is that the repo does not state the
policy.

A useful policy would be:

- Direct human/agent CLI entrypoints have a shebang and executable bit.
- Imported modules and test helpers do not need executable bit.
- Shell libraries are not executable and say "source-only" at the top.
- Package-internal helper scripts are invoked through their owner facade.

### 6. Sourced Shell Libraries Need Contracts

`scripts/lib/devfinity-brain-readiness.sh` is sourced by larger devfinity
scripts and relies on parent-provided functions and variables. That is fine for
a shell library, but it should have a short header naming that it is source-only
and listing required globals. Without that, it looks like a malformed standalone
script during static review.

### 7. Devfinity Scenario Scripts Are Large And Stateful

The devfinity scenario scripts do careful local safety checks, including
loopback URL guards and harness-environment assertions. They are also large and
stateful. Their correctness depends on many live local services and durable
state transitions.

They should have a thin source-inspection test layer for argument validation,
loopback refusal, evidence log setup, and cleanup traps, separate from the
expensive end-to-end smoke runs.

### 8. Skill Helper Scripts Need A Separate Quality Gate

Managed-skill helpers live under `finite-skills` and product-specific skill
directories. `finite-skills/scripts/check-static.sh` validates important
frontmatter, routing, and marker contracts, but helper scripts are a separate
runtime surface from prose skill files.

The repo should distinguish:

- Skill prose and routing checks.
- Skill helper script syntax checks.
- Skill helper runtime smoke checks.
- External-service helpers that require credentials and must be opt-in.

### 9. Credential Install Helpers Need Dry-Run Contract Tests

Credential installers are written defensively, but several do not have obvious
direct source tests by name. They should have tests for remote hostname guards,
file modes, refusal behavior, and "do not print secret value" guarantees. These
can be source-inspection tests, not live remote tests.

### 10. Operational Facts Are Still Duplicated

The repo has good contract tests for `finite-status`, runbook facts, NixOS
monitoring, runtime image contracts, and rollout scripts. Even so, hostnames,
service units, ports, model names, artifact labels, release names, and remote
paths still appear in multiple script surfaces.

The next hardening pass should extend fact checks to the highest-risk deploy
and rollout scripts, not only runbooks.

## Deletion Readiness

There are no production scripts in this audit that should be blindly deleted
just because they are old. The safe cleanup set is conditional: remove the
script only after its references are updated and its surviving facts are either
covered by tests, current docs, or no longer needed.

### Deleted In First Cleanup Pass

| Removed script | Why it was removable |
| --- | --- |
| `infra/hosts/lat1/scripts/deploy-finitechat-server.sh` | Its header said the script was written for a mismatched host shape and was no longer executable as written. Current docs now point at the closure/NixOS deploy runbook. |
| `finitecomputer-v2/deploy/finite-chat/lat1/README.md` and `workspace.env.example` | They configured the removed future-lat1 deploy script and repeated stale host assumptions. |
| `scripts/repro-hermes-wedge/fake_chat_server.py` and `scripts/repro-hermes-wedge/run.sh` | They were narrow repro artifacts; the wedge class is now pinned by direct bounded HTTP transport tests. |
| `scripts/devfinity-brain-card-hands-on` and `scripts/devfinity-brain-card-hands-on-up` | They staged a one-off Brain card demo and had no current external references beyond their own headers. |
| `scripts/devfinity-chat-authz-upgrade` | It was a large one-off local acceptance script for a specific chat-authz migration and had no current external references. |

### Deleted In Follow-Up Cleanup Pass

| Removed script | Why it was removable |
| --- | --- |
| `scripts/tests/test_devfinity_restart_process.py` | Unwired source-inspection test. The tested command remains called from `devfinity/justfile`, but no current facade or PR validation runs this test. |
| `scripts/tests/test_ci_changed_paths.py` | Unwired duplicate coverage for CI path normalization; active CI selection is covered by `scripts/tests/test_ci_select_harnesses.py`. |
| `finitechat/scripts/hermes-restic-preflight.py` and `finitechat/tests/container/test_restic_preflight.py` | Standalone restic preflight had no current runner. The remaining active contract is GitHub secret/variable preflight plus S3-backed Docker smoke evidence. |
| `infra/tinfoil/confidential-kimi-k2-6/deepseek-v4-benchmark.py`, `deepseek-v4-context-gate.py`, `deepseek-v4-lab-launch.sh`, and `deepseek-v4-protocol-gate.py` | Dated lab helpers for the 2026-08-07 DeepSeek measurement. Durable facts remain in the research note and current validation lives behind `just finite-private-deepseek-contract`. |

### Remaining Script Removal Candidates

| Candidate | Why it may be removable | Before deleting |
| --- | --- | --- |
| `finitechat/scripts/hermes-real-gateway-demo.sh` | It is documented as a low-level manual runner, and stronger canary/smoke scripts now exist. | Confirm `hermes-phone-canary.py` and `hermes-real-gateway-admission-smoke.py` cover the intended proof, then update Hermes docs and `finitechat/scripts/ios-local-agent.sh` references that still mention the demo. |
| `scripts/prepare_glm53_blind_comparison.py` | It is a one-off model-evaluation helper. | Delete after the GLM comparison is no longer an active decision input, the result is captured in research/docs, and the runbook, `finitecomputer-v2/justfile`, and unit-test references are removed. |

### Keep Until A Named External Condition Closes

| Keep for now | Delete condition |
| --- | --- |
| `scripts/import-sync` | Delete only after `AGENTS.md` no longer requires importing stray commits from unarchived source repos. |
| `scripts/legacy-hermes-*`, `scripts/legacy_hermes_*.py`, and `infra/runbooks/legacy-hermes-box1-to-lat3.md` | Delete only after every legacy Hermes bot migration and rollback window is closed, and the sealed-source evidence is archived elsewhere. |
| `scripts/check_finite_private_*` DeepSeek/GLM checks | Delete each candidate checker only when that model lane is retired from the Finite Private evaluation/release process and the owning `justfile` stops exposing it. |
| `finite-skills/skills/**/scripts/*` | Delete only with the owning skill or after skill packaging/static checks prove the helper is unused. |
| Runtime entrypoints under `finitechat/containers`, `finite-search/tinfoil`, `infra/images`, and runtime templates | Delete only when the image/template no longer ships or references them. |
| `scripts/finite-status`, `scripts/snapshot-sqlite`, closure deploy scripts, restore scripts, and production deploy scripts | Do not delete while they remain the canonical status, recovery, deploy, or production safety boundary. |

## Risk Watchlist

These files are not necessarily broken. They are worth tracking because they
combine production authority, broad state mutation, network calls, or historical
platform assumptions.

| File | Why it matters | Current guard pattern |
| --- | --- | --- |
| `scripts/finite-status` / `scripts/finite_status.py` | Canonical fleet-state command | Read-only subprocess wrapper, contract-tested constants, mutating SQL rejection |
| `scripts/deploy-lat*-closure-cache` | Host activation path | Manifest validation, target host checks, expected unit checks |
| `scripts/rollout-lat1-runtime-artifact` | Agent Runtime rollout path | Prepare/execute split, exact plan hash, host and artifact validation |
| `scripts/snapshot-sqlite` | Snapshot DB inspection boundary | Scratch copy, manifest sidecar checks, read-only SQLite |
| `infra/scripts/restore-hosted-web-chat-snapshot` | Recovery-set validation | Format gates, manifest checks, empty target requirement, SQLite helper |
| `infra/runbooks/finite-private-ops.sh` | Finite Private operational canaries and relaunch | Explicit env approvals for load sweep and relaunch |
| `scripts/devfinity-*` | Large local integration scenarios | Loopback and harness guards, but limited lightweight tests |
| `scripts/install-*credentials` | Secret material installation | Remote hostname checks, mode checks, no value printing |
| `finitechat/containers/agent/entrypoint.sh` | Runtime restore/backup/supervision behavior | State-root checks, restic guards, child supervision |
| `finitechat/scripts/hermes-*` | Chat/Hermes compatibility and hardening proof | Evidence-schema validation and explicit required layers |
| `finite-skills/scripts/check-static.sh` | Managed-skill product contract | Prose/frontmatter marker checks |
| `finite-search/scripts/check-static.sh` | Search stack static contract | Required file checks, `bash -n`, Python compile, placeholder scan |

## Recommended Follow-Up

1. Create `scripts/README.md` as the script registry.
   Include path, owner, facade, purpose, mutates production, mutates local
   durable state, requires secrets, dry-run/validate mode, test coverage, and
   related runbook.

2. Add a standard header convention for operational scripts.
   Suggested fields: `Purpose`, `Authority`, `Mutates`, `Requires`,
   `Safe modes`, `Evidence output`, `Rollback boundary`, and `Owner`.

3. Document the facade hierarchy.
   State that root `just` is the preferred human/agent entrypoint, component
   facades own component workflows, CI workflows own promotion automation, Nix
   owns deployed service configuration, and `scripts/finite-status` owns fleet
   status.

4. Normalize shebang and executable-bit policy.
   Make direct CLI scripts executable. Remove shebangs from pure modules if
   they are never invoked directly. Mark shell libraries as source-only.

5. Quarantine retired operational scripts.
   Add a naming/location convention such as `infra/retired-scripts/` or a
   required `Retired:` header. Apply it first to the lat1 finitechat deploy
   script unless it is actively maintained again.

6. Move reusable runbook shell commands out of `infra/runbooks`.
   Keep procedural prose in runbooks. Keep supported reusable tooling in
   `infra/scripts` or the owning component's `scripts` directory.

7. Add a shell static gate.
   Start with `bash -n` for bash scripts and an allowlisted strict-mode check
   that understands POSIX `sh` scripts and source-only libraries. Add shellcheck
   later if dependency cost is acceptable.

8. Expand source-inspection tests for high-risk shell.
   Cover approval gates, exact-host guards, temporary credential files, cleanup
   traps, refusal modes, and no-secret-output guarantees.

9. Extend fact-contract checks beyond runbooks.
   Pull hostnames, service units, public routes, ports, model names, and
   artifact labels from declarative owners or contract constants where possible.

10. Keep adding probes only to `scripts/finite-status`.
    The `AGENTS.md` rule says `scripts/finite-status` is the only
    platform/fleet status command. New incident probes should extend it or be
    clearly documented as local/product-only checks.

## Bottom Line

The script surface is powerful and mostly disciplined, but under-indexed. The
best patterns are already present: root `just` facades, Nix-first execution,
prepare/execute deploy phases, read-only status collection, manifest-verified
snapshot inspection, and source-inspection tests for operational scripts.

The highest-return work is documentation and consistency, not a rewrite:

- Make a script registry.
- Clarify authority layers.
- Normalize executable policy.
- Quarantine retired deploy scripts.
- Extend existing contract-test patterns to the highest-risk shell commands.
