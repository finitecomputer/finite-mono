# Depot vs. Buildkite for Finite's monorepo CI

Date: 2026-08-24

Status: historical decision research. Its initial recommendation was superseded
by ADR-0007 after Electron was explicitly deferred and the operator chose
Origin plus native Depot CI. No CI, release, deployment, or production state
was changed by this research itself.

This comparison uses only first-party sources: the checked-in Finite workflows,
official Depot and Buildkite documentation and pricing, and a representative
Finite GitHub Actions run. Product capabilities and prices change; re-check the
linked pages before signing a contract.

## Executive answer

**Keep GitHub Actions with Depot runners and Depot container builders for now.**
Buildkite is a good product, but it solves a different problem from the one
Finite currently has.

The premise that “Depot is runners” is incomplete as of August 2026. Depot now
has three relevant execution products: drop-in GitHub Actions runners, remote
BuildKit-based container builders, and a separate Depot CI control plane that
runs a substantial subset of GitHub Actions syntax. It also has a remote build
cache, registry, CI observability, and test-results product. These can be used
separately or together. [Depot product overview](https://depot.dev/docs),
[Depot CI overview](https://depot.dev/docs/ci/overview)

Buildkite's center of gravity is different: **Pipelines is a mature CI/CD
control plane**, and agents are the execution layer. Buildkite can manage hosted
Linux/macOS agents or dispatch to agents in Finite's own AWS account,
Kubernetes cluster, on-premises network, Windows machines, Macs, or specialized
hardware. [Buildkite architecture](https://buildkite.com/docs/pipelines/architecture),
[agent comparison](https://buildkite.com/docs/agent)

For Finite, Buildkite becomes compelling if at least one of these turns into a
real requirement:

- run CI in a Finite-controlled VPC/on-premises environment or on GPU, Windows,
  or other specialized machines;
- generate and mutate a monorepo pipeline at runtime rather than selecting from
  a static job graph;
- use Buildkite's signed-pipeline enforcement, queue priority, cross-pipeline
  concurrency, approval/input steps, or mature flaky-test quarantine; or
- make Buildkite the common orchestration layer across several source hosts and
  execution environments.

It does **not** make sense to adopt Buildkite merely to obtain another pool of
hosted Linux runners. That would add a pipeline migration, a second syntax and
secrets model, and user/platform fees while giving Finite a smaller baseline
hosted machine for twice the published per-minute rate at the same vCPU count.
The representative Finite run examined below also had no material runner queue.

The most useful next experiment is much narrower: benchmark the 32-minute Nix
service-package job on larger Depot shapes and/or shard its build graph. A
Buildkite pilot should be gated on a requirement that only Buildkite actually
solves, not on general CI dissatisfaction.

## Finite's current state

The repository currently has 12 GitHub workflow files and 30 job definitions.
Nineteen jobs name `depot-ubuntu-24.04` literally, and two reusable/dispatch
workflows default to that label. The main [CI workflow](../../.github/workflows/ci.yml)
has 14 jobs: 13 Depot Linux jobs and one conditional GitHub-hosted macOS job.
Release matrices also build on Linux and both macOS architectures.

Finite is already using two distinct Depot services:

1. Depot-managed machines execute most ordinary GitHub Actions jobs; GitHub
   still parses workflows, evaluates the DAG, issues tokens, owns environments,
   shows the canonical run UI, and reports branch-protection checks.
2. The [service image](../../.github/workflows/service-images.yml),
   [Agent Runtime](../../.github/workflows/runtime-image.yml), and
   [DeepSeek image](../../.github/workflows/deepseek-v4-vllm-image.yml) lanes
   call Depot's separate remote container-build service through OIDC. These
   builders are not the GitHub Actions runner.

The [Agent Runtime build wrapper](../../finitecomputer-v2/scripts/build_runtime_image.py)
already treats `docker`, `depot`, and Apple's `container` as explicit engines,
which is a useful escape hatch. The other two direct `depot build` workflows
would need a small Buildx-oriented fallback added if Depot container builds
became unavailable.

Finite is also meaningfully coupled to GitHub Actions above the command layer:

- ten workflow files reference `GITHUB_TOKEN` or `github.token`;
- image workflows use `packages: write` and publish to GHCR;
- release workflows use GitHub artifacts and `gh release` extensively;
- [production deploy](../../.github/workflows/production-deploy.yml) uses a
  protected `production` environment and explicitly checks that GitHub's
  `ci.yml` succeeded for the source SHA; and
- the Phala preflight uses a protected `phala-staging` environment.

Those edges mean that changing runner providers is easy, while changing CI
control planes is a release-and-deployment contract migration.

### Measured current bottleneck

A representative full PR run on 2026-08-24 completed in 38m29s, with 71.37
aggregate minutes across non-skipped jobs. After the initial path selector, all
eligible jobs began within roughly two seconds. The `Nix service packages` job
then occupied the critical path for about 32m16s, followed by a roughly 4m47s
Devfinity smoke job. This is compute/build-graph latency, not a queue-capacity
problem. [Finite Actions run 32743526418](https://github.com/finitecomputer/finite-mono/actions/runs/32743526418)

At published overage rates, 71.37 minutes on Depot's 2-vCPU/8-GB label is about
$0.285 before plan inclusions; the same wall minutes on Buildkite's 2-vCPU/4-GB
hosted shape are about $0.571 before plan inclusions and before Buildkite's
per-active-user fee. The machines are not memory-equivalent, so this is a
directional list-price comparison, not a performance forecast. [Depot runner
types](https://depot.dev/docs/github-actions/runner-types), [Buildkite
pricing](https://buildkite.com/pricing/)

## Capability comparison

| Area | GitHub Actions + Depot (current) | Depot CI | Buildkite Pipelines |
| --- | --- | --- | --- |
| What owns orchestration? | GitHub Actions | Depot's Switchyard-based control plane | Buildkite SaaS control plane |
| Compute | Managed ephemeral Depot runners; Business can add dedicated/customer-AWS options | Managed Depot Linux x86-64/Arm64 sandboxes | Managed Linux/macOS, or Finite-operated agents almost anywhere |
| Existing Actions YAML | Unchanged except `runs-on` label | Migration tool copies to `.depot/workflows`; broad but incomplete compatibility | Native migration is a rewrite; Actions compatibility plugin is a public preview |
| Runtime-generated DAG | GitHub graph is fixed once started | No documented equivalent to pipeline upload | First-class dynamic pipeline upload and SDKs |
| Container builds | Depot remote BuildKit, persistent per-project cache, native x86/Arm | Same Depot builder is directly integrated | Hosted remote builders are Enterprise-only; self-hosted users operate their own, or can keep Depot |
| General remote cache | Depot Cache supports Actions, Bazel, sccache, Go, Gradle, Turborepo, Pants, Maven and others | Built in | Best-effort hosted volumes, plugins/object storage, or a third-party remote cache |
| macOS / Windows | Both available as Depot GitHub runners; GPU is Business-only | Neither macOS nor Windows | Hosted macOS; Windows and GPU require self-hosted agents |
| Deployment approvals | GitHub Environments and deployment APIs remain intact | `jobs.<job>.environment` is unsupported | Native block/input steps, team-scoped approvals, and cross-build deployment concurrency |
| Test intelligence | Depot JUnit analytics and timing-based splitting work on Depot runners | Same, integrated | More mature Test Engine: splitting, workflows, state management, quarantine and external-CI ingestion |
| Strongest security distinction | Fresh single-tenant EC2 host for each managed Actions job | KMS-encrypted secrets and managed sandboxes | Customer-controlled execution plus cryptographically signed pipelines |
| Local/uncommitted loop | Commands remain locally reproducible through Nix/`just`; full workflow needs a push | `depot ci run` uploads a patch without pushing | Experimental Preflight creates/pushes a temporary branch; dry-run validates pipeline generation |
| Pricing shape | Flat plan plus per-second runner/builder usage | Included in the same Depot plan, metered by sandbox size | Per active user and self-hosted agent, plus hosted compute |

## GitHub Actions compatibility and migration effort

### Staying on GitHub Actions with Depot runners

Depot's runner product is deliberately a label substitution. GitHub sends a
`workflow_job` webhook; Depot registers a fresh EC2 instance from a standby
pool, executes the job with a GitHub-compatible image, and terminates it when
the job ends. The workflow syntax, Marketplace actions, permissions,
environments, artifacts, checks, and GitHub UI remain GitHub's. Depot documents
no concurrency, cache-size, or network limits for these runners. [Depot GitHub
Actions runners](https://depot.dev/docs/github-actions/overview)

This is the lowest-risk option for the current repository. It is also easy to
reverse: a job can move back to a GitHub-hosted or self-hosted label without
rewriting its steps.

### Moving from GitHub Actions to Depot CI

Depot CI is a real control-plane change, not merely a runner change. Its
migration tool analyzes `.github/workflows`, copies selected workflows and
local actions into `.depot/`, rewrites runner labels and paths, and disables
unsupported jobs with comments. It can run beside GitHub Actions during a
shadow period. [Depot CI migration CLI](https://depot.dev/docs/cli/reference/depot-ci)

The compatibility is broad enough for ordinary Linux test DAGs, including
`needs`, conditions, matrices, services, containers, outputs, reusable
workflows in the same repository, `merge_group`, schedules, and manual
dispatch. However, the documented gaps directly intersect Finite:

- Depot CI has only Linux x86-64 and Arm64 sandboxes; it does not run macOS or
  Windows jobs.
- `jobs.<job>.environment` is unsupported.
- fork-originated pull-request workflows are planned but unsupported, which
  matters for a public repository.
- cross-repository reusable workflows are unsupported.
- `GITHUB_TOKEN` is a GitHub App token in Depot CI, and GitHub Packages/GHCR
  rejects it for package pushes; Depot recommends a PAT or another registry.
- several GitHub-specific event families, including `release`, are unsupported.

[Depot CI compatibility matrix](https://depot.dev/docs/ci/compatibility),
[Depot CI sandbox types](https://depot.dev/docs/ci/overview#depot-ci-sandboxes)

That makes Depot CI suitable for a **shadow pilot of one Linux, non-publishing
test lane**, not a wholesale move of Finite CI, images, releases, or production
deployments today.

#### Finite already tested the native Depot CI path

This is not only a paper compatibility concern. Finite has already tried the
two distinct Depot migration paths, and the repository history records why the
current hybrid won:

- On August 5, PR #420 initially changed Linux runner labels across CI,
  releases, preflights, and image workflows. A follow-up commit about two and a
  half minutes later restored the non-CI labels and limited the rollout to the
  main CI workflow. The merged PR explicitly kept release, image, dispatch,
  macOS, and self-hosted work on their existing labels. [initial label
  change](https://github.com/finitecomputer/finite-mono/commit/904f14ff90129cdfacba6770dfc775742aaceedb),
  [immediate scope reduction](https://github.com/finitecomputer/finite-mono/commit/4ee68a09e996bf80543fb58090e2c85e3265f3f1),
  [PR #420](https://github.com/finitecomputer/finite-mono/pull/420)
- On August 12, an abandoned native conversion was preserved in local stash
  `921cad249931e8cd6699e01fa85a7fd278a8a1a7`. It contained nine new
  `.depot/workflows` files plus `depot.json`. The conversion mechanically
  changed the mandatory `Signed + notarized Electron (macOS arm64)` job from
  `macos-14` to `depot-ubuntu-latest` while leaving its Apple-only `security`,
  `otool`, `codesign`, `xcrun notarytool`, `xcrun stapler`, and `spctl`
  commands intact. The same invalid substitution affected the conditional
  macOS CI job. The source workflows confirm that these lanes really build,
  sign, inspect, and notarize Apple artifacts rather than merely using a macOS
  label incidentally. [CI workflow at the attempted revision](https://github.com/finitecomputer/finite-mono/blob/99d597caa7784047164651790c9555e90daede80/.github/workflows/ci.yml#L184-L226),
  [Finite Chat release workflow](https://github.com/finitecomputer/finite-mono/blob/99d597caa7784047164651790c9555e90daede80/.github/workflows/release-finitechat.yml#L24-L187),
  [fbrain release matrix](https://github.com/finitecomputer/finite-mono/blob/99d597caa7784047164651790c9555e90daede80/.github/workflows/release-fbrain.yml#L20-L45),
  [fsite release matrix](https://github.com/finitecomputer/finite-mono/blob/99d597caa7784047164651790c9555e90daede80/.github/workflows/release-fsite.yml#L21-L45)
- PR #495 then deliberately landed the workable split: GitHub continued to
  orchestrate workflows, eligible Linux jobs used Depot-managed Actions
  runners, container builds used Depot remote builders through OIDC, and
  macOS work stayed in GitHub Actions. Its summary explicitly records that the
  native `.depot/workflows` conversion was omitted because Depot CI does not
  support macOS sandboxes. [PR #495](https://github.com/finitecomputer/finite-mono/pull/495)

The stash can be inspected without restoring or mutating it:

```sh
git show --stat 921cad249931e8cd6699e01fa85a7fd278a8a1a7
git show 921cad249931e8cd6699e01fa85a7fd278a8a1a7:.depot/workflows/release-finitechat.yml
```

The lesson is narrower than “Depot did not work”: **Depot CI could not replace
the mixed-OS control plane, while Depot runners and builders worked well inside
GitHub Actions.** That prior attempt materially lowers the expected value of
retrying a wholesale Depot CI migration unless its macOS boundary changes.

### Moving to Buildkite

Native Buildkite is a different pipeline language and model. A Buildkite step
maps more closely to a GitHub Actions job than to an Actions step; triggers are
usually configured in Buildkite's UI/API; artifacts, caches, expressions,
services, matrices, secrets, environments, and job outputs need explicit
translation. Buildkite's official migration guide presents this as a pipeline
translation and re-architecture, not a runner-label change. [Buildkite GitHub
Actions migration guide](https://buildkite.com/docs/pipelines/migration/from-githubactions)

Buildkite also has a GitHub Actions compatibility runtime for incremental
migration, but the official plugin labels it a **public preview**. The current
compatibility guide supports a useful subset of Linux x86-64 and native macOS
Arm jobs, public/local actions, static matrices, services, artifacts, cache,
OIDC, and opt-in temporary GitHub tokens. Important limitations include no
Windows or Linux Arm, no GitHub environments/approvals/deployment records, no
private actions, incomplete event payloads, no dynamic matrices, and no
enforcement of matrix `fail-fast`. Its runner image is compatible, not an exact
GitHub-hosted image replica. [Official GitHub Actions Buildkite plugin](https://github.com/buildkite-plugins/github-actions-buildkite-plugin),
[v0.26 compatibility guide](https://github.com/buildkite/buildkite-gha/blob/v0.26.0/docs/compatibility.md)

Buildkite therefore has the highest migration cost of the three options for
Finite. It should be adopted for native Buildkite capabilities, not on the
assumption that existing Actions workflows will transparently run there.

## Compute, queues, and autoscaling

Depot's managed Actions runners are fresh, single-tenant EC2 instances and
range from 2 vCPU/8 GB to 64 vCPU/256 GB on both x86 and Arm, plus managed
macOS and Windows options. GPU runners and custom AMIs are Business features.
macOS capacity is not fully elastic and may queue because of Apple licensing.
[Depot runner types](https://depot.dev/docs/github-actions/runner-types),
[Depot pricing](https://depot.dev/pricing)

Depot CI itself is narrower: managed Ubuntu 24.04 x86-64 and Arm64 sandboxes
from 2 vCPU/8 GB to 64 vCPU/256 GB. Depot advertises pre-warmed sandboxes and
2–3 seconds from commit to running job. [Depot CI overview](https://depot.dev/docs/ci/overview)

Buildkite hosted agents automatically scale and are destroyed after each job.
Published hosted options are Linux and macOS; Windows is not hosted. Linux Arm
and the largest machines require Enterprise. Hosted Linux jobs run in isolated
virtualized environments on multi-tenant hardware. [Buildkite hosted Linux](https://buildkite.com/docs/agent/buildkite-hosted/linux),
[Buildkite hosted macOS](https://buildkite.com/docs/agent/buildkite-hosted/macos)

Buildkite's distinctive compute capability is the open, cross-platform agent.
Finite can run persistent or ephemeral agents on Linux, macOS, Windows,
FreeBSD, Docker, Kubernetes, its own AWS/GCP accounts, or on-premises systems.
Agents make outbound HTTPS connections, so no inbound agent port is required.
Buildkite publishes autoscaling AWS and Kubernetes stacks; the AWS stack can
scale to zero, use Spot, and split workloads into queues with different
instances and permissions. Finite then owns provisioning, patching, scaling,
isolation, and incident response for that fleet. [Self-hosted install options](https://buildkite.com/docs/agent/self-hosted/install),
[Elastic CI Stack for AWS](https://buildkite.com/docs/agent/self-hosted/aws/elastic-ci-stack),
[network requirements](https://buildkite.com/docs/agent/self-hosted/security/network-requirements)

Buildkite also has more expressive scheduling than the current setup: queues
and tags route jobs to capabilities; job and agent priorities choose scarce
capacity; organization-wide concurrency groups serialize shared resources
across builds and pipelines. [Buildkite queues](https://buildkite.com/docs/agent/queues),
[job priority](https://buildkite.com/docs/pipelines/configure/workflows/job-priority),
[concurrency groups](https://buildkite.com/docs/pipelines/configure/workflows/controlling-concurrency)

These are substantial advantages when capacity is scarce or heterogeneous.
They do not improve Finite's observed run, where every eligible job dispatched
immediately.

## Build acceleration and caching

### Docker and BuildKit

Depot has the clearest technical advantage here. `depot build` is a compatible
front end to optimized remote BuildKit machines with persistent, project-scoped
NVMe layer caches. The default builder has 16 vCPU/32 GB; larger plans offer up
to 64 vCPU/128 GB. It runs x86 and Arm builds natively and concurrently rather
than emulating Arm. Build autoscaling can clone the primary cache to additional
builders, with the documented trade-off that writes on scaled clones do not
flow back into the primary cache. [Depot container builds](https://depot.dev/docs/container-builds/overview),
[autoscaling behavior](https://depot.dev/docs/container-builds/how-to-guides/autoscaling)

Buildkite's hosted agents have remote BuildKit builders and a container
registry only on Enterprise. Its remote builders keep a short-lived local layer
cache and can use persistent container cache volumes. On Pro, ordinary Docker
builds run on the agent unless Finite brings another service. On self-hosted
agents, Finite owns Docker/BuildKit and its cache. [Buildkite remote Docker
builders](https://buildkite.com/docs/agent/buildkite-hosted/linux/remote-docker-builders),
[Buildkite agent comparison](https://buildkite.com/docs/agent)

There is no need to give up Depot container builds if Buildkite wins for
orchestration. Depot officially supports Buildkite OIDC, so a Buildkite job can
continue to run `depot build` without a stored Depot token. [Depot's Buildkite
integration](https://depot.dev/docs/container-builds/integrations/buildkite)

### Rust, Node, Bazel, and Nix

Depot's distributed remote cache supports the GitHub Actions cache protocol as
well as Bazel, sccache, Go, Gradle, Turborepo, Pants, Maven, and moonrepo, and is
usable from CI or developer machines. Depot runners automatically receive
short-lived cache credentials and preconfiguration for supported tools. [Depot
Cache overview](https://depot.dev/docs/cache/overview), [cache authentication](https://depot.dev/docs/cache/authentication)

That benefits Finite's existing `Swatinem/rust-cache` and `actions/setup-node`
cache operations without changing workflow syntax. Finite could separately
evaluate `sccache` for Rust, but that is a build-system change and should be
measured rather than assumed to outperform the current Cargo cache.

Buildkite hosted cache volumes are fast, pipeline-scoped, best-effort volumes
retained for at most 14 days. They are intentionally non-deterministic and
cannot be shared across pipelines. Buildkite also supports object-store cache
plugins and artifacts, but its Bazel guidance points to external remote cache
and execution products. [Buildkite caching guide](https://buildkite.com/docs/pipelines/best-practices/caching),
[hosted cache volumes](https://buildkite.com/docs/agent/buildkite-hosted/cache-volumes)

Neither vendor is Finite's Nix binary cache. The current workflows use Cachix;
that remains the authoritative acceleration layer for Nix outputs. Buildkite
self-hosting could provide warm local Nix stores, while Depot Business custom
images could preinstall tooling, but either introduces cache trust, invalidation,
and maintenance decisions that Cachix already avoids.

## Pipeline model, UI, and deployments

Buildkite's strongest non-compute feature is the ability to generate new steps
at runtime in any language and upload them into the active build. It provides
SDKs for TypeScript/JavaScript, Python, Go, and Ruby, and `if_changed` support
for monorepo diffs. A generator can calculate transitive dependencies, choose
machine queues, alter retries, or expand only the tests relevant to the actual
build result. [Dynamic pipelines](https://buildkite.com/docs/pipelines/configure/dynamic-pipelines),
[monorepo patterns](https://buildkite.com/docs/pipelines/best-practices/working-with-monorepos)

That is materially more flexible than GitHub Actions or Depot CI's declared
workflow graph. Finite already performs path-aware package and harness
selection in repository scripts, so the question is whether that system has
actually hit the ceiling of a static graph. There is no evidence of that in the
measured run.

Buildkite also provides native block and input steps, team-restricted approvals,
cross-pipeline triggers, annotations, and cross-build concurrency groups. These
make it a stronger general deployment orchestrator than Depot CI. It still runs
the deployment commands Finite supplies; it is not a release-state database or
automatic rollback controller. [Buildkite block steps](https://buildkite.com/docs/pipelines/configure/step-types/block-step),
[deployment patterns](https://buildkite.com/docs/pipelines/deployments)

The current Depot runner model preserves GitHub's existing deployment features,
which is an advantage rather than a gap for Finite. Depot CI currently lacks
`jobs.environment`; moving the production or Phala lanes there would remove a
checked-in protection boundary unless Finite designed and proved a replacement.
Buildkite could replace it with block steps and restricted deploy queues, but
the production run lookup, artifact flow, environment authority, GitHub
deployment records, and rollback contract would all need to be migrated and
tested together.

## Local reproducibility and debugging

Finite's most important reproducibility layer is already below CI: Nix,
`scripts/with-dev-env`, and root `just` recipes keep build commands provider
independent.

Depot CI has the best uncommitted-change loop of the two control planes. `depot
ci run` detects local changes, uploads a patch through Depot Cache, injects it
after checkout, and can run one selected job without pushing a branch. Depot CI
also supports CLI/API logs, retries, status, metrics, and SSH into a live
sandbox. [Local Depot CI runs](https://depot.dev/docs/ci/how-to-guides/manage-workflow-runs),
[SSH debugging](https://depot.dev/docs/ci/how-to-guides/debug-with-ssh)

Buildkite can validate generated pipeline YAML locally with `pipeline upload
--dry-run`. Its current Preflight feature runs an uncommitted snapshot remotely,
but it is experimental and creates a temporary branch on `origin`; it is not an
offline pipeline executor. Hosted agents also provide terminal access, and
hosted macOS can provide browser-based desktop access. [Buildkite Preflight](https://buildkite.com/docs/platform/cli/preflight),
[terminal access](https://buildkite.com/docs/agent/buildkite-hosted/terminal-access),
[macOS desktop access](https://buildkite.com/docs/agent/buildkite-hosted/desktop-access)

## Secrets, OIDC, and security

With the current architecture, GitHub issues job tokens and stores workflow
secrets; Depot supplies a fresh single-tenant machine. Depot cache entries are
repository-scoped, encrypted at rest and in transit, and the runner is destroyed
after the job. Depot container builders are also single-organization/project,
and BuildKit connections use build-lifetime mTLS certificates. Depot accepts
GitHub and Buildkite OIDC in exchange for temporary project-scoped credentials.
[Depot security model](https://depot.dev/docs/security)

Depot CI has its own secret store using a KMS-generated organization key and
AES-256-GCM envelope encryption. Its cloud OIDC issuer can authenticate to AWS,
GCP, and Azure, but moving from GitHub Actions requires adding Depot's issuer and
claim format to each cloud trust policy. [Depot CI OIDC](https://depot.dev/docs/ci/oidc)

Buildkite's self-hosted architecture is attractive for high-assurance or private
network workloads: source checkout and externally managed secrets can remain in
Finite's account, while the agent polls the SaaS control plane over outbound
HTTPS. Buildkite supports OIDC with job/pipeline/cluster claims and recommends
external Vault or cloud secret stores for the strongest boundary. It also has a
convenient encrypted, cluster-scoped secret store, but those values are
decrypted by Buildkite's application servers before delivery to the agent.
[Buildkite OIDC](https://buildkite.com/docs/pipelines/security/oidc),
[external secret-store guidance](https://buildkite.com/docs/pipelines/security/secrets/managing),
[Buildkite secrets](https://buildkite.com/docs/pipelines/security/secrets/buildkite-secrets)

Buildkite's unusual, decisive security capability is **signed pipelines**:
agents can cryptographically verify pipeline instructions and refuse tampered
steps. Depot does not document an equivalent agent-side signature enforcement
mechanism. [Buildkite signed pipelines](https://buildkite.com/docs/agent/self-hosted/security/signed-pipelines)

Both vendors offer enterprise identity and audit features. Depot's published
pricing places SAML/SCIM and audit logging on paid add-ons/Business; its public
2024 announcement documents SOC 2 Type I, so the current report and type should
be confirmed during procurement. Buildkite states that it maintains an annual
SOC 2 Type II report; Pro includes SSO, while Enterprise adds SCIM/custom SAML,
audit exports, private logs/artifacts, and advanced governance. [Depot pricing](https://depot.dev/pricing),
[Depot SOC 2 announcement](https://depot.dev/blog/depot-soc-2),
[Buildkite security](https://buildkite.com/about/security/),
[Buildkite pricing](https://buildkite.com/pricing/)

## Observability and test analytics

Depot's runner dashboard provides live job state, searchable logs across
repositories, step timing, CPU/memory history, failure rates, usage/cost views,
and automatic runner right-sizing recommendations. Depot CI adds per-job
metrics, SSH, CLI/API access, and AI-generated failure diagnosis. [Depot GitHub
Actions analytics](https://depot.dev/docs/github-actions/observability/github-actions-metrics),
[Depot CI metrics](https://depot.dev/docs/ci/observability/depot-ci-metrics)

Depot Test Results became generally available in July 2026. It ingests JUnit,
shows inline and organization-wide failures, identifies possible flakes and
slow tests, and can split future shards using historical timings. This is
already available on Depot runners without moving to Depot CI. [Depot test
results](https://depot.dev/docs/ci/observability/depot-ci-test-results),
[timing-based splitting](https://depot.dev/docs/ci/how-to-guides/split-tests)

Buildkite's Test Engine is older and materially deeper. It can ingest results
from non-Buildkite CI, has framework-specific collectors (including Rust,
Jest, Playwright, pytest, Go, Swift, Android, and others), timing-based splitting,
test ownership and performance history, automatic flaky-test workflows, and
test states that mute or skip quarantined tests. Workflows can notify Slack,
open Linear issues, or change quarantine state. [Buildkite Test Engine](https://buildkite.com/docs/pipelines/configure/tests),
[flaky-test workflows](https://buildkite.com/docs/pipelines/reduce-flaky-tests),
[test quarantine](https://buildkite.com/docs/pipelines/configure/tests/test-suites/test-state-and-quarantine)

This capability is separable from a pipeline migration: Buildkite explicitly
accepts results from other CI systems. If flaky-test management becomes a pain,
Finite can pilot Test Engine from the existing Actions workflow before changing
orchestration.

Buildkite has broader export options for pipeline telemetry: EventBridge,
Datadog, Honeycomb, and vendor-neutral OpenTelemetry, plus APIs and agent/queue
metrics for Prometheus, CloudWatch, and Grafana. Several of the richest exports
and cluster dashboards are Enterprise features. [Buildkite observability](https://buildkite.com/docs/pipelines/integrations/observability/overview),
[monitoring decision guide](https://buildkite.com/docs/pipelines/best-practices/monitoring-and-observability)

## Pricing and cost model

Published prices as of 2026-08-24:

| Cost | Depot | Buildkite |
| --- | --- | --- |
| Base plan | Developer: $20/month, 1 user; Startup: $200/month, unlimited users; Business: custom | Free: up to 5 users; Pro: $30 per active user/month; Enterprise: custom with a 30-user minimum |
| Included managed Linux | Developer 2,000 runner minutes; Startup 20,000 runner minutes | Free 2,000 Linux vCPU-minutes; Pro 4,000 Linux vCPU-minutes |
| Smallest Linux overage | 2 vCPU/8 GB: $0.004 per wall minute | 2 vCPU/4 GB: $0.008 per wall minute ($0.004 per vCPU-minute) |
| Larger Linux | 4/16 GB: $0.008; 8/32 GB: $0.016; through 64/256 GB: $0.128 per minute | 4/16 GB: $0.016; 8/32 GB: $0.032 per minute; larger hosted shapes require Enterprise |
| Managed macOS | M2, 8 vCPU/24 GB: $0.08/min | M4, 6 vCPU/28 GB: $0.12/min; 12 vCPU/56 GB: $0.24/min |
| Remote container builder | 16 vCPU/32 GB: $0.04/min; 500 minutes Developer or 5,000 Startup included | Hosted remote Docker builder is Enterprise-only, without a public standalone price |
| Self-hosted compute | Business offers dedicated/customer-AWS deployment options; custom quote | Own cloud bill **plus** Pro/Enterprise user fees and agent capacity; Pro includes 10 agents, then $3.50/agent/month |
| Tests | 1M passing results included; then $1 per 1M passing results; failed/errored/skipped free | Free 250k; Pro 1M then $15 per 1M executions; richer quarantine workflow on Pro/Enterprise |

[Depot pricing](https://depot.dev/pricing), [Depot runner rates](https://depot.dev/docs/github-actions/runner-types),
[Depot builder rates](https://depot.dev/docs/container-builds/overview#pricing),
[Buildkite pricing](https://buildkite.com/pricing/)

The models reward different things. Depot's Startup plan bundles large blocks of
runner, Depot CI, container-builder, and storage usage for a flat team price.
Buildkite has a useful free tier for a very small team, but Pro scales with
active users and then adds hosted compute or Finite's own infrastructure. A
hybrid Buildkite-plus-Depot design also retains both subscriptions.

Before comparing invoices, export one month of Finite's actual runner minutes,
builder minutes, macOS minutes, peak parallelism, cache storage, and number of
people/bots that Buildkite would count as active users. Included minutes make a
single run's marginal cost zero until the allowance is exhausted.

## Lock-in, outages, and escape hatches

### Current Depot model

The current orchestration is GitHub-native and the commands mostly live in
repository scripts and Nix environments. If Depot runners are unavailable,
changing a `runs-on` label moves a job to GitHub-hosted or self-hosted compute.
If Depot's remote builders are unavailable, the Runtime wrapper already has a
local Docker engine and direct `depot build` calls can be translated to Buildx;
the cost is a cold/slower build and loss of Depot's persistent cache, not a
pipeline rewrite.

The current system does depend on both GitHub's Actions control plane and
Depot's runner/control plane. Depot publishes component status, but its public
pricing and documentation reviewed here do not state a numeric uptime SLA; a
required uptime/support commitment should be negotiated rather than assumed.
[Depot status](https://status.depot.dev/), [Depot pricing](https://depot.dev/pricing)

Moving into Depot CI increases orchestration lock-in somewhat (`.depot`, Depot
secrets, Depot OIDC issuer, APIs and UI), but retaining GitHub Actions YAML
limits the syntax rewrite required to leave. Unsupported GitHub semantics and
the separate file tree still make round-trip migration something to test, not
assume.

### Buildkite

Buildkite's agent, AWS/Kubernetes stacks, CLI, Terraform provider, and many
plugins are open source, and artifacts can be placed in customer storage. This
provides excellent **compute portability**. The control plane is still SaaS:
self-hosted agents require Buildkite for registration, job dispatch, pipeline
upload, logs, metadata, OIDC and orchestration, and there is no documented
self-hostable Buildkite control plane. [Buildkite agent repository](https://github.com/buildkite/agent),
[Buildkite network requirements](https://buildkite.com/docs/agent/self-hosted/security/network-requirements)

Native `.buildkite` YAML, dynamic uploads, plugins, metadata, approvals,
concurrency groups, Test Engine, and the UI are Buildkite-specific. The deeper
Finite uses those excellent features, the more expensive a future control-plane
migration becomes. Keeping all work commands in Nix/`just`/ordinary scripts and
treating pipeline generation as a thin adapter is the best escape hatch.

Buildkite publishes a 99.95% platform uptime commitment and 99.5% hosted-agent
commitment for its Enterprise support tiers. Self-hosting agents does not keep
new builds dispatching during a Buildkite control-plane outage. [Buildkite SLA](https://buildkite.com/about/legal/service-level-agreement/)

## Decisive capabilities and material gaps

### Depot is considerably better when

- Finite wants to preserve GitHub Actions semantics and change only compute.
- Fast, cached, native multi-architecture container builds are the main pain.
- Shared Actions/Bazel/sccache-style remote caching is valuable across CI and
  developer machines.
- A managed Windows or managed GPU runner is required without operating a
  fleet.
- Flat team pricing and low-cost small Linux runners matter.
- Depot CI's no-push local patch loop is valuable and its Linux-only
  compatibility envelope is sufficient.

### Buildkite is considerably better when

- Finite needs one pipeline to span managed and Finite-controlled compute,
  private networks, on-prem machines, Windows, GPUs, or arbitrary hardware.
- Runtime pipeline generation, job/agent priority, or cross-pipeline
  concurrency is necessary for a large monorepo or scarce infrastructure.
- Signed pipelines must let the agent reject modified instructions.
- Team-scoped manual approvals and input forms should be first-class pipeline
  nodes independent of GitHub Environments.
- Automated flaky-test state, quarantine, ownership, and remediation workflows
  justify a dedicated Test Engine.
- Enterprise telemetry export and a CI-centric control-plane UI are worth a
  pipeline migration and higher platform cost.

### Important missing features

- **Depot CI:** no macOS/Windows sandboxes, no GitHub environments, no fork PR
  execution, no `GITHUB_TOKEN` package push, and no documented runtime DAG
  upload.
- **Depot generally:** no documented customer-operated, cross-platform generic
  agent model or agent-side signed-pipeline enforcement comparable to
  Buildkite.
- **Buildkite hosted:** no Windows or documented managed GPU SKU; Linux Arm and
  remote Docker builders are Enterprise-only.
- **Buildkite caching:** no Depot-like first-party remote cache for sccache,
  Bazel and other build tools; generic volumes are best effort and
  pipeline-scoped.
- **Buildkite Actions compatibility:** public preview, not a drop-in guarantee;
  native migration is the supported long-term architecture.
- **Both:** neither replaces Cachix for Nix or provides a self-hosted CI control
  plane. Neither is a deployment rollback system by itself.

## Recommendation by scenario

### 1. Keep Depot — recommended now

Keep GitHub Actions as the control plane, Depot as the default Linux runner and
remote image builder, GitHub-hosted or Depot macOS for Apple release work, and
Cachix for Nix.

Then benchmark, in order:

1. `Nix service packages` on `depot-ubuntu-24.04-4` and
   `depot-ubuntu-24.04-8`, recording wall time, billable minutes, CPU/memory,
   Cachix hits and closure outputs. At list rates the 4-vCPU shape is
   cost-neutral if it cuts 32 minutes below 16; the 8-vCPU shape is neutral
   below 8 minutes.
2. Shard independent Nix package groups if the build graph permits it without
   losing shared work or weakening the single-root-lockfile proof.
3. Add Depot JUnit reporting to a representative Rust/Node suite only if its
   failure and splitting data will be acted on.
4. Document an emergency runner-label and local-Buildx fallback; prove it on a
   non-publishing workflow.

### 2. Adopt Buildkite

Adopt native Buildkite only after a pilot proves a Buildkite-only requirement.
The best pilot is an isolated, non-deploying lane whose dynamic generator or
self-hosted special-hardware queue is materially simpler than the current
Actions version. Do not begin with production deploy, releases, GHCR image
publishing, or the mixed-OS main CI workflow.

Success criteria should include total cycle time, queue time, cache hit rate,
agent operations burden, developer debugging time, migration/maintenance lines,
monthly platform plus compute cost, fork-PR security, and a control-plane outage
exercise.

### 3. Hybrid Buildkite + Depot

This is the strongest architecture **if** Buildkite's orchestration becomes
necessary:

- Buildkite Pipelines owns dynamic orchestration, priorities, approvals, test
  workflows, and Finite/self-hosted special queues.
- Depot remains the remote multi-architecture container builder and possibly
  remote cache, authenticated with Buildkite OIDC.
- GitHub Actions temporarily retains GitHub-specific releases, GHCR publication,
  protected production environments, and macOS lanes until each through-line is
  deliberately migrated.
- Buildkite Test Engine can be piloted from the current Actions workflow even
  earlier, without moving Pipelines.

The hybrid is more capable but has three external control planes in the path
(GitHub source/remaining Actions, Buildkite, and Depot). Use it only when its
non-overlapping capabilities pay for that operational and procurement surface.

## Bottom line

Depot is the better **execution and build-acceleration fit** for Finite today;
Buildkite is the stronger **programmable orchestration and customer-controlled
infrastructure platform**. Finite's evidence shows a slow 2-vCPU Nix critical
path, not runner queue pressure or a static-pipeline failure. Optimize that
critical path first. Revisit Buildkite when a concrete dynamic-pipeline,
private-compute, signed-pipeline, or advanced test-quarantine requirement
appears.
