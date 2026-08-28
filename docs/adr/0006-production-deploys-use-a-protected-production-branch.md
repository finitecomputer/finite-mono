# Production deploys use a protected production branch

Status: accepted

Finite production deploys are promoted by merging `main` to a protected
`production` branch. The initial implementation is a hard-cut Production
Bootstrap: any old `production` branch or ruleset from the earlier failed
experiment is treated as residue, not authority. Setup deliberately resets or
recreates `production` from the scaffolded `main` revision, reinstalls branch
rules and the GitHub `production` environment, then uses a tiny later PR to
flip `mutation_enabled = true`. The Deployment Manifest lives at
`infra/deployments/production.toml` and records the environment, surfaces,
gates, classification, rollback policy, risky-path policy version, and
evidence policy; exact source revision comes from the branch tip, and observed
versions remain reported through deployment records and Grafana metrics rather
than becoming desired state.

This deliberately avoids deploying every push to `main`, avoids making Grafana
or the handoff queue desired-state authority, and delays a full lat2 staging
clone until host roles are cleaner. The first normal Deploy Principal may still
use root SSH as temporary debt; the target shape is a narrow `finite-deploy`
principal, with root kept for break-glass only.

Production deploys are serialized by environment/host. A deployment may be
cancelled normally before the Mutation Boundary, but once production mutation
has started the attempt is interrupted state: later deploys must wait until the
interruption is inspected and reconciled.

GitHub is the v1 Deployment Record backend: the environment deployment, workflow
logs, and uploaded artifacts record the manifest, closure artifact, validation
results, and final outcome. When mutation is enabled, deployments also record
pre/post `finite-status` snapshots collected from the checked-out revision's
status scripts, activation output, and the crossed Mutation Boundary.
Deployments carry a Deployment Classification. Known schema or persistence
paths require `schema-change` or stronger classification; `schema-change`
records a fresh backup path/checksum and rollback target, while `forward-only`
requires explicit production approval and makes no automatic rollback promise.

The `production` branch is PR-only after bootstrap: direct pushes and
force-pushes are blocked, and the normal path is a `main` to `production` pull
request. A manual helper workflow may open that PR, but it does not push code
or accept arbitrary SHA inputs. The planning workflow validates the manifest,
classifies changed paths, verifies the branch tip's CI result, builds the
exact lat1 closure, updates one pull-request plan comment, and emits a
Deployment Plan without production mutation. The merge workflow validates the
exact production tip, resolves the CI source commit for production merge
commits, verifies that source's `CI gate`, builds and uploads the exact
closure, and emits a dry-run Deployment Record when `mutation_enabled = false`.
When `mutation_enabled = true`, the deploy job waits on the protected GitHub
`production` environment, reuses the prepared closure artifact when available,
rebuilds the same SHA only if the artifact expired, stages the checked-out
`finite-status` collector on lat1, captures valid JSON evidence before and
after, gates the lat1 host deploy on deploy-critical status sections, and then
runs the existing lat1 closure deploy script. The deploy records whole-platform
status, including Agent Runtime fleet convergence, but that Agent rollout
evidence does not block this NixOS host deploy path.

The initial workflows are named `Open Production Deploy PR`, `Production Deploy
Plan`, and `Production Deploy`. The plan workflow is a pull-request review
gate, not a dry deploy: it answers what would happen if the PR merged. The
deploy workflow is the only normal path that can cross the Mutation Boundary,
and its deploy job is inert until the manifest enables mutation.

The first manifest keeps `mutation_enabled = false`. V1 classifications are
only `ordinary`, `schema-change`, and `forward-only`; the manifest pins the
risky-path policy with a value such as `risky_path_policy = "lat1-v1"`.
Risky-path detection compares `production...HEAD`, because the question is
what production newly receives.

Temporary root SSH credentials, when mutation is enabled, must live only as
GitHub `production` environment secrets, including the deploy key and pinned
`known_hosts`; repository-level secrets are not sufficient production
authority. The deploy workflow verifies the exact production tip's `CI gate`
status through GitHub rather than trusting branch protection alone. The plan
workflow updates one concise pull-request comment, and the deploy workflow
supports manual retry only for the current `production` branch tip, never for
an arbitrary SHA input. The deploy workflow reuses the prepare-stage artifact
when available and otherwise rebuilds the same source revision; the PR plan is
review evidence, not a permanent runtime dependency.

Initial rollout hard-cuts `production` from the current `main` scaffold without
deploying, then installs branch protection/rulesets and the GitHub
`production` environment, then runs validation-only until a separate reviewed
mutation-enablement change lands. Validation attempts do not require production
environment approval; approval is required only to cross the Mutation Boundary
after mutation is deliberately enabled. Initial artifact names are
`production-plan-<sha>` for PR evidence, `deployment-plan`,
`lat1-nixos-closure-<sha>`, `deployment-record`, `finite-status-before`, and
`finite-status-after` for deploy evidence. MVP records interrupted
post-boundary attempts through GitHub's built-in environment deployment and
artifacts, but it deliberately defers automatic lockout/reconciliation state
until after the happy path is proven. If a post-boundary deploy fails before
that hard blocker exists, an operator must inspect lat1, rerun the same deploy
when safe or use the existing manual rollback runbook, and record the outcome
before approving another production deploy.

V1 is deliberately a thin conductor over the deploy primitives that already
exist. It does not introduce a deploy service, a deployment database, permanent
object storage, a lat2 staging clone, Runtime Artifact Promotion, Runtime
Rollout, arbitrary-SHA deploys, automatic rollback, or a new rollback
mechanism. The manifest and record schemas should stay minimal: enough to prove
the selected source, classification, gates, artifact, mutation boundary, and
outcome, but not a parallel model of every production fact.

The v1 manifest is intentionally short: environment, scope, classification,
risky-path policy, `mutation_enabled`, rollback policy, and any immediately
needed gate names. The generated Deployment Record is likewise minimal:
source revision, classification, mutation flag, whether the Mutation Boundary
was crossed, closure system path when known, before/after status artifact
names, outcome, timestamps, and override reason when present. Interrupted
deploy state is derived from GitHub deployment/artifact records rather than a
custom state store. When mutation is enabled, V1 runs the same base lat1
postflight for every Production Deploy; service-specific gates may be added
later only where a single existing command already makes the boundary obvious.
