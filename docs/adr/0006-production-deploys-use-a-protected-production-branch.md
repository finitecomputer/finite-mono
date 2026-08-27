# Production deploys use a protected production branch

Status: proposed

Finite production deploys are promoted by merging `main` to a protected
`production` branch. The first shipped workflow state is validation-only: a
pull request to `production` builds and comments the deploy plan, while the
merge validates the production branch tip and records that mutation remained
closed. A later PR may enable mutation only by changing the manifest, workflow
guard, credentials, and branch/environment protection together. The Deployment
Manifest lives at `infra/deployments/production.toml` and records the
environment, surfaces, gates, classification, rollback policy, risky-path
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

GitHub is the v1 Deployment Record backend: workflow logs and uploaded
artifacts record the manifest, closure artifact, validation results, and final
outcome. When mutation is later enabled, deployments also record pre/post
`finite-status` snapshots, activation output, and the crossed Mutation
Boundary. Deployments carry a Deployment Classification. Known schema or
persistence paths require `schema-change` or stronger classification;
`schema-change` records a fresh backup path/checksum and rollback target, while
`forward-only` requires explicit production approval and makes no automatic
rollback promise.

The `production` branch is PR-only: direct pushes and force-pushes are blocked,
and the normal path is a `main` to `production` pull request. The planning
workflow validates the manifest, classifies changed paths, verifies the branch
tip's CI result, builds the exact lat1 closure, updates one pull-request plan
comment, and emits a Deployment Plan without production mutation. The first
merge workflow validates the exact production tip, resolves the CI source
commit for production merge commits, verifies that source's `CI gate`, rebuilds
the exact closure, requires `mutation_enabled = false`, and emits a dry-run
Deployment Record with outcome `dry_run_blocked_before_mutation`.

The initial workflows are named `Production Deploy Plan` and `Production
validation`. The plan workflow is a pull-request review gate, not a dry deploy:
it answers what would happen if the PR merged, while the validation workflow
proves the production branch tip without mutating production.

The first manifest keeps `mutation_enabled = false`, and the initial workflows
fail closed if it changes. V1 classifications are only `ordinary`,
`schema-change`, and `forward-only`; the manifest pins the risky-path policy
with a value such as `risky_path_policy = "lat1-v1"`. Risky-path detection
compares `production...HEAD`, because the question is what production newly
receives.

Temporary root SSH credentials, when mutation is enabled, must live only as
GitHub `production` environment secrets, including the deploy key and pinned
`known_hosts`; repository-level secrets are not sufficient production
authority. The validation workflow verifies the exact production tip's `CI
gate` status through GitHub rather than trusting branch protection alone. The
plan workflow updates one concise pull-request comment, and the validation
workflow supports manual retry only for the current `production` branch tip,
never for an arbitrary SHA input. The validation workflow rebuilds and
revalidates; the PR plan is review evidence, not a runtime dependency.

Initial rollout creates `production` from the current `main` without deploying,
then installs branch protection/rulesets, then runs validation-only until a
separate reviewed mutation-enablement change lands. Validation attempts do not
require production environment approval; approval is required only to cross the
Mutation Boundary after mutation is deliberately enabled. Initial artifact
names are `production-plan-<sha>` for PR evidence and
`production-validation-<sha>` for merge validation evidence. If a future
deployment is interrupted after the Mutation Boundary, later deploys refuse
automatically until a reconciliation marker or fresh successful Deployment
Record exists for the observed production state.

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
custom state store. When mutation is enabled, V1 runs the same base lat1
postflight for every Production Deploy; service-specific gates may be added
later only where a single existing command already makes the boundary obvious.
