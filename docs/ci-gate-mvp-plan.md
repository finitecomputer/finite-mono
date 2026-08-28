# CI Gate MVP Hard-Cut Plan

Status: PROPOSED

Scope: make the existing pull-request workflow path-aware while preserving one
fail-closed merge decision. This is intentionally a narrow first cut. It skips
work only for path groups whose ownership is already clear, and runs the full
suite for everything else.

## Outcome

The existing [CI workflow](../.github/workflows/ci.yml) will have one required
status check named `CI gate`:

```text
changes -> selected existing jobs -> CI gate
```

The workflow starts for pull requests to `main` and relevant pushes. Production
promotion pull requests are intentionally handled by the separate Production
Deploy Plan workflow, which verifies that the promoted source SHA already has a
successful `CI gate` result instead of launching a second full CI run against
the same tree.

Within the `main` pull-request workflow, jobs are skipped with job-level `if`
conditions rather than workflow-level path filters, because a required workflow
skipped by a path filter can remain pending.

## MVP Boundary

The `changes` job has one responsibility: determine which existing test
harnesses need to run. It emits one `run_*` boolean per existing CI job. There
is no change mode, generic classifier, dependency graph, or separate reporting
layer.

For the MVP, files in these documentation paths select no test harness:

- `docs/**`
- `README.md`
- `CONTRIBUTING.md`
- `AGENTS.md`
- `CONTEXT-MAP.md`

Files in these paths select `dashboard`, `nix-checks`, and `devfinity-smoke`:

- `finitecomputer-v2/apps/dashboard/**`
- `finitechat/packages/finitechat-chat-ui/**`

Any other touched file selects all eight existing jobs. This includes
component-local documentation, mixed changes, root manifests, `ci.yml` itself,
Nix, infrastructure, skills, and any path that has not been explicitly moved to
a narrower harness after the MVP. Non-CI workflow files use the narrower Nix
static contract lane.

The MVP does not:

- split the root Rust workspace test into per-component Rust jobs;
- add detailed dependency routing for Chat, Brain, Sites, Identity, or Infra;
- alter cache keys, runner placement, toolchain versions, or build commands;
- change release, image, deploy, or manually dispatched workflows;
- introduce a second legacy or advisory CI path.

## Implementation Checklist

### 1. Detect the required harnesses

- [x] Add a `changes` job with checkout and one short Bash step; do not add a
  classifier program or third-party changed-files action.
- [x] Use `git diff --name-only` with the pull request base/head revisions or
  the push before/current revisions.
- [x] Start all eight `run_*` outputs as `false`.
- [x] Ignore the explicitly listed documentation paths.
- [x] Turn on `run_dashboard`, `run_nix_checks`, and `run_devfinity_smoke` for
  dashboard or shared chat-UI paths.
- [x] Turn on every `run_*` output and stop checking when any other path is
  present.
- [x] Fail the `changes` job when Git cannot produce the diff.

### 2. Check the three MVP cases

- [x] Prove a docs-only change selects no existing test/build jobs.
- [x] Prove dashboard-only, shared chat-UI, and allowed documentation changes
  select exactly `dashboard`, `nix-checks`, and `devfinity-smoke`.
- [x] Prove any other path, including one mixed with dashboard or docs paths,
  selects all eight existing jobs.

Validated against historical repository changes under both `push` and
`pull_request` diff ranges: docs-only selected zero jobs; dashboard, shared
chat UI, and docs-plus-dashboard selected three; Rust and dashboard mixed with
an unowned path selected all eight. An invalid Git range failed detection.

### 3. Make the existing jobs conditional

- [x] Add `needs: changes` to each of the eight existing jobs.
- [x] Add a job-level condition matching that job's `run_*` output.
- [x] Preserve every existing command, environment, runner label, timeout, and
  artifact behavior in the MVP.
- [x] Keep main pull-request path decisions inside the job-level selector while
  routing production promotion pull requests through the Production Deploy Plan
  workflow.

### 4. Add the authoritative gate

- [x] Add a final job with ID `ci-gate` and display name `CI gate`.
- [x] Give it `needs` entries for `changes` and all eight conditional jobs.
- [x] Use `if: ${{ always() }}` so it evaluates after upstream failures,
  cancellations, and skips.
- [x] Keep the gate lightweight: an Ubuntu runner, no checkout, no Nix, and no
  dependency installation.
- [x] Require `changes` to have result `success`.
- [x] Fail the gate when `changes` fails; a diff error must not create a
  partial green run.
- [x] Require every selected job to have result `success`.
- [x] Allow unselected jobs to be skipped.
- [x] Fail when a selected job fails, is cancelled, or is skipped.

### 5. Validate the hard cut

- [x] Open a docs-only fixture pull request and verify only `changes` and
  `CI gate` execute successfully.
- [x] Open a dashboard-only fixture pull request and verify only `dashboard`,
  `nix-checks`, and `devfinity-smoke` execute between `changes` and `CI gate`.
- [x] Open a fixture pull request touching Rust or workflow code and verify all
  eight existing jobs execute.
- [x] Deliberately fail one selected job and verify `CI gate` fails.
- [x] Verify cancellation of a selected job cannot produce a green gate.
- [x] Record the wall time and aggregate job-minutes for the three MVP cases.

Live validation used temporary PR #390, which was closed without merging.
Docs-only completed in 17 seconds wall time and 12 seconds of aggregate job
execution. Dashboard-only completed in 7 minutes 21 seconds wall time and 13
minutes 5 seconds of aggregate execution. The full route selected and executed
all eight harnesses; its first attempt took 65 minutes 58 seconds wall time and
41 minutes 21 seconds of aggregate execution. Seven harnesses passed, while the
Brain matrix twice failed an existing Apple Container runtime readiness check;
`CI gate` failed closed both times. A separate deliberately failing Dashboard
fixture also made `CI gate` fail. Cancellation was covered by the isolated
gate-result validation because GitHub does not expose single-job cancellation.

### 6. Switch merge enforcement

- [x] Confirm the exact check name emitted by GitHub is `CI gate`.
- [ ] Enable the repository ruleset or branch protection requiring only
  `CI gate` for pull requests to `main`.
- [ ] Do not retain the eight individual jobs as required checks in parallel.
- [ ] Confirm a new docs-only pull request can merge after its gate succeeds.
- [ ] Confirm a failed full-suite pull request cannot merge.
- [x] Document rollback as removing the job conditions so all eight jobs run;
  do not weaken the gate's success rules.

Activation order is part of the safety contract: prepare the `main` ruleset
with only the GitHub Actions `CI gate` check while leaving it disabled, merge
the workflow so `main` can emit that check, and only then activate the ruleset.
Activating it earlier would block every pull request whose base workflow cannot
yet emit `CI gate`. Rollback removes the eight job-level conditions to restore
the legacy full-suite behavior while keeping `CI gate` fail-closed.

## MVP Exit Criteria

The MVP is complete when:

- [ ] `CI gate` is the single authoritative required check.
- [ ] Docs-only changes reliably avoid all eight heavy jobs.
- [ ] Dashboard-only changes run only the three named jobs.
- [ ] Unknown and cross-component changes reliably run the complete suite.
- [ ] Selected failures, cancellations, and unexpected skips fail the gate.
- [ ] The three fixture cases pass before branch protection is switched.

## Post-MVP Module Onboarding Checklist

For each item below, "moved" means its owned paths, shared inputs, generated
inputs, test commands, integration consumers, and mixed-change behavior have
been documented and added to the `changes` job. A module must continue to
select all jobs until that work is complete. Moving a module does not mean
blindly creating one job per crate; closely coupled crates may remain one lane
when that avoids duplicate compilation.

- [ ] **Dashboard and shared chat UI:** finalize ownership for
  `finitecomputer-v2/apps/dashboard` and
  `finitechat/packages/finitechat-chat-ui`, then replace the provisional MVP
  rule with the reviewed long-term rule.
- [ ] **Finite Computer v2 services:** onboard `finite-core`,
  `finite-private-limiter`, `finite-saas-core`, `finite-saas-local`,
  `finite-saas-runner`, and `finite-specialization-worker`, including their
  dashboard and devfinity consumers.
- [ ] **Devfinity:** onboard the local stack harness, generated configuration,
  readiness tests, and every service whose binary it starts.
- [ ] **Finite Chat Rust and UniFFI:** onboard the protocol, MLS, transport,
  delivery, client, daemon, server, hosted-device, CLI, blob, RMP, HTTP,
  Hermes, and UniFFI packages without weakening durable-history or
  mixed-version coverage.
- [ ] **Finite Chat Electron:** onboard the macOS device-parity checks,
  packaging, signing inspection, daemon embedding, and production-dashboard
  boundary.
- [ ] **Finite Chat Hermes integration:** onboard its Rust smoke, Python
  formatting/type checks, adapter regression report, and platform-adapter
  tests together.
- [ ] **Finite Brain:** onboard the five Brain crates, Product Client tests,
  language/API contracts, managed-skill integration, and the full-product
  matrix.
- [ ] **Finite Sites:** onboard the six Sites crates and the `fsite` CLI,
  including identity, storage, publish, and devfinity integration edges.
- [ ] **Finite Identity:** onboard authority tests and the cross-product
  identity-conformance suite spanning Runner, Chat, Sites, Brain, devfinity,
  and the production Nix configuration.
- [ ] **Finite Agentd:** onboard its crate, Nix package, runtime boundary, and
  service-image consumers.
- [ ] **Finite Nostr:** onboard the shared primitives and require all direct
  product consumers when its public interfaces change.
- [ ] **Finite Skills:** onboard skill validation, runtime delivery contracts,
  bundled baselines, and Brain/Chat product-matrix consumers.
- [ ] **Finite Search:** onboard static checks and any service/runtime contract
  consumers.
- [ ] **Finite Specialization:** onboard specialization configuration,
  worker tests/package builds, image inputs, and deployment contracts.
- [ ] **Infrastructure and Nix:** map `flake.nix`, `flake.lock`, `infra/**`,
  service packages, NixOS evaluation, runtime-image, rollout, healthcheck,
  status, Stripe, recovery, secret-bootstrap, and runner-guardrail contracts.
  Keep broad changes in this group selecting all jobs unless independence is
  proven.
- [ ] **Root Rust workspace inputs:** define how root manifests, the lockfile,
  workspace dependency changes, toolchain configuration, and shared scripts
  fan out to component lanes. These inputs should normally continue to select
  every Rust consumer.
- [ ] **Release and image lifecycle checks:** identify only the validation that
  belongs in pull-request CI. Keep publishing, signing, deployment, and
  environment-mutating operations in their existing tag- or dispatch-driven
  workflows.
