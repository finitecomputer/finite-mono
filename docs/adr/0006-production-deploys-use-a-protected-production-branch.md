# Production deploys use a protected production branch

Status: proposed

Finite production deploys are promoted by merging `main` to a protected
`production` branch, then deploying the branch tip through a GitHub Actions
`production` environment. A pull request to `production` builds and plans the
deploy, while the merge performs the production mutation after approval. The
Deployment Manifest lives at `infra/deployments/production.toml` and records
the environment, surfaces, gates, classification, rollback policy, risky-path
policy version, and evidence policy; exact source revision comes from the
branch tip, and observed versions remain reported through deployment records
and Grafana metrics rather than becoming desired state.

This deliberately avoids deploying every push to `main`, avoids making Grafana
or the handoff queue desired-state authority, and delays a full lat2 staging
clone until host roles are cleaner. The first normal Deploy Principal may still
use root SSH as temporary debt; the target shape is a narrow `finite-deploy`
principal, with root kept for break-glass only.

Production deploys are serialized by environment/host. A deployment may be
cancelled normally before the Mutation Boundary, but once production mutation
has started the attempt is interrupted state: later deploys must wait until the
interruption is inspected and reconciled.

GitHub is the v1 Deployment Record backend: deployments, workflow logs, and
uploaded artifacts record the manifest, pre/post `finite-status` snapshots,
activation output, verification results, and final outcome. Deployments carry a
Deployment Classification. Known schema or persistence paths require
`schema-change` or stronger classification; `schema-change` records a fresh
backup path/checksum and rollback target, while `forward-only` requires explicit
production approval and makes no automatic rollback promise.

The `production` branch is PR-only: direct pushes and force-pushes are blocked,
and the normal path is a `main` to `production` pull request. The planning
workflow validates the manifest, classifies changed paths, verifies the branch
tip's CI result, builds the exact lat1 closure, and emits a Deployment Plan
without production mutation. The merge workflow may rebuild the exact closure
if the plan artifact is unavailable, then runs preflight, waits for protected
environment approval, crosses the Mutation Boundary, verifies lat1, and emits
the Deployment Record. The first implementation may ship as a real workflow
skeleton with production mutation disabled until credentials and branch
protection are installed.

The initial workflows are named `Production Deploy Plan` and `Production
Deploy`. The plan workflow is a pull-request review gate, not a dry deploy: it
answers what would happen if the PR merged, while the deploy workflow is the
only normal path that can mutate production.

The first manifest keeps `mutation_enabled = false` until credentials and
rulesets are deliberately installed. V1 classifications are only `ordinary`,
`schema-change`, and `forward-only`; the manifest pins the risky-path policy
with a value such as `risky_path_policy = "lat1-v1"`. Risky-path detection
compares `production...HEAD`, because the question is what production newly
receives.

Temporary root SSH credentials live only as GitHub `production` environment
secrets, including the deploy key and pinned `known_hosts`; repository-level
secrets are not sufficient production authority. The deploy workflow verifies
the exact production tip's `CI gate` status through GitHub before mutation
rather than trusting branch protection alone. The plan workflow updates one
concise pull-request comment, and the deploy workflow supports manual retry
only for the current `production` branch tip, never for an arbitrary SHA input.
The deploy workflow rebuilds and revalidates when needed; the PR plan is review
evidence, not a runtime dependency.

Initial rollout creates `production` from the current `main` without deploying,
then installs branch protection/rulesets, then enables the deploy path. Dry-run
deploy attempts do not require production environment approval; approval is
required only to cross the Mutation Boundary. V1 artifact names are
`deployment-plan`, `deployment-record`, `finite-status-before`,
`finite-status-after`, and `lat1-nixos-closure-<sha>`. If a deployment is
interrupted after the Mutation Boundary, later deploys refuse automatically
until a reconciliation marker or fresh successful Deployment Record exists for
the observed production state.

V1 is deliberately a thin conductor over the deploy primitives that already
exist. It does not introduce a deploy service, a deployment database, permanent
object storage, a lat2 staging clone, Runtime Artifact Promotion, Runtime
Rollout, arbitrary-SHA deploys, or a new rollback mechanism. The manifest and
record schemas should stay minimal: enough to prove the selected source,
classification, gates, artifact, mutation boundary, and outcome, but not a
parallel model of every production fact.

The v1 manifest is intentionally short: environment, scope, classification,
risky-path policy, `mutation_enabled`, rollback policy, and any immediately
needed gate names. The generated Deployment Record is likewise minimal:
source revision, classification, mutation flag, whether the Mutation Boundary
was crossed, closure system path when known, before/after status artifact
names, outcome, timestamps, and override reason when present. Interrupted
deploy state is derived from GitHub deployment/artifact records rather than a
custom state store. V1 runs the same base lat1 postflight for every Production
Deploy; service-specific gates may be added later only where a single existing
command already makes the boundary obvious.
