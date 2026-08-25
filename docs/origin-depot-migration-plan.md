# Cursor Origin and Depot CI migration plan

Status: approved on 2026-08-25; implementation and Shadow Runs are in progress.

This plan implements [ADR-0007](adr/0007-origin-is-the-source-authority-and-depot-is-ci.md).
It targets a same-day Hard Cutover when every required gate passes. A failed
gate stops the affected cutover; the date is not authority to waive evidence.

## End state

- A private Cursor Origin repository is the `finite-mono` Source Authority.
- Native Depot CI owns pull-request, branch, tag/manual, image, release, and
  non-mutating deployment-validation workflows.
- GitHub `finitecomputer/finite-mono` is frozen and private. It is not mirrored
  from Origin and its issue tracker is outside this migration.
- Public `finitecomputer/finite-releases` is the Release Repository.
- Public `ghcr.io/finitecomputer/*` remains the Artifact Registry.
- GitHub Actions has no active workflow, required check, secret, or app grant.
- Production mutation remains disabled.

This state removes GitHub as source and CI authority. It does **not** provide
complete GitHub outage independence because Releases and GHCR remain on GitHub.

## Invariants

1. Product release asset names, checksum names, component versions, and rolling
   aliases do not change.
2. A Shadow Run may not publish a production release, overwrite a rolling
   alias, push a production image tag, or mutate production.
3. Images are built once and promoted without rebuilding. Source and
   destination manifest digests are compared before a tag becomes deployable.
4. Existing public GHCR digests remain anonymously pullable throughout the
   source-repository privacy change.
5. Electron is not advertised or published by the migrated workflows.
6. Production mutation stays disabled until a later ADR defines the Deploy
   Principal, approval gate, interruption handling, and Deployment Record
   backend outside GitHub Actions.
7. Hard Cutover removes superseded execution authority immediately. Frozen
   release and image data is retained until no consumer references it.

## Credentials

Create values outside the repository and record only these names:

| Name | Scope | Storage |
|---|---|---|
| `FINITE_RELEASES_GITHUB_TOKEN` | Fine-grained token selected only for `finitecomputer/finite-releases`, Contents write | Depot CI secret |
| `FINITE_GHCR_USERNAME` | Publisher account name | Depot CI secret or variable |
| `FINITE_GHCR_TOKEN` | Classic PAT with `write:packages` only; no `repo` or `delete:packages` | Depot CI secret |

Prefer a dedicated publisher account. If an existing administrator owns the
same-day tokens, give them short expirations and create a dated follow-up to
rotate both credentials to the publisher account. Verify organization classic
PAT policy, SSO authorization, expiry, and package Write access before use.

## Phase 0: capture the rollback boundary

- Record the exact GitHub `main` and `production` tips, all component tags, and
  the current branch rules and required check integration IDs.
- Export a name-only inventory of GitHub Actions secrets, variables,
  environments, and app grants. Never export secret values into the repo.
- Inventory every existing versioned and rolling-alias Release asset with its
  size and SHA-256 checksum.
- Record the six current public GHCR packages and every deployed digest:
  `finite-saas-dashboard`, `finite-specialization-worker`, `agent-runtime`,
  `private-limiter`, `deepseek-v4-vllm`, and `finite-saas-core`.
- From a clean unauthenticated client, pull every deployed digest and retain the
  command/result as migration evidence.
- Name the rollback commit that can restore the pre-cutover workflow files.
  Restoring execution also requires deliberately restoring credentials and
  rules; old workflows are not left live as a hidden fallback.

## Phase 1: create the external control planes

Human-authorized steps:

1. Create an Origin-hosted private `finite-mono` repository. Do not use an
   Origin view that remains a GitHub-sourced mirror; Depot only attaches to an
   Origin-hosted repository.
2. Push all branches and tags, then compare their object IDs and trees with the
   captured baseline before accepting Origin as authoritative.
3. Install the Depot app for that Origin repository and configure the Depot
   project, secrets, variables, and OIDC trust needed by existing builders.
4. Create public `finitecomputer/finite-releases` with a README stating that it
   contains release metadata and assets only, not product source.
5. Configure the two separate GitHub publication credentials above.
6. Inspect GHCR access inheritance for all six packages. Ensure the publisher
   has Write access after `finite-mono` becomes private; package visibility must
   remain Public.

## Phase 2: make workflows provider-thin

Copy compatible workflow structure into `.depot/workflows/`, but move external
mutation and provider-specific behavior into repository-owned commands that can
be run locally and tested against synthetic targets.

| Current workflow | Migration treatment |
|---|---|
| `ci.yml` | Move the Linux graph to Depot; remove the parked Electron job; recreate `CI gate` in Origin. |
| `hermes-runtime-smoke.yml` | Move to Depot; replace GitHub token assumptions and prove manual dispatch and Depot builder identity. |
| `lat1-nixos-closure.yml` | Move to Depot; replace GitHub runner selection and `gh run` artifact operations. |
| `lat3-nixos-closure.yml` | Move to Depot with a fixed Depot Linux runner and preserve exact-revision closure artifacts. |
| `phala-readonly-preflight.yml` | Replace unsupported GitHub `environment` scoping with explicit Depot secret scope and operator policy. |
| `production-deploy-plan.yml` | Replace GitHub PR comments and `gh run` lookup; publish an Origin check and a Depot artifact. |
| `production-deploy.yml` | Port validation only; force `mutation_enabled = false`; do not emulate GitHub approval with an unreviewed secret. |
| `service-images.yml`, `runtime-image.yml`, `deepseek-v4-vllm-image.yml` | Build/save in Depot and promote the exact manifest to public GHCR with the dedicated package credential. |
| Component release workflows | Cross-compile/package in Depot and publish to `finite-releases`; omit Electron. |

The first pass may continue using supported Marketplace Actions, but remove
GitHub event/API/token assumptions. Vendor or replace critical Actions and
GitHub-hosted build inputs in a later outage-hardening pass. Because Releases
and GHCR deliberately remain on GitHub, this migration must not claim that a
total `github.com` outage leaves delivery operational.

## Phase 3: prove Origin and Depot event semantics

Use a disposable branch and tags before changing required checks:

- Open and update an Origin pull request; observe exactly one Depot run and one
  Origin check suite.
- Prove a successful run, an intentional test failure, cancellation, retry, job
  outputs, matrix behavior, artifacts, and concurrency cancellation.
- Push to a branch and verify the expected branch workflow.
- Test `merge_group` only if an Origin merge queue will be enabled.
- Test manual dispatch through Depot.
- Push a disposable component-shaped tag and verify whether Origin delivers it
  to Depot. If it does not, make `depot ci dispatch --ref <tag>` the documented
  release command; do not route through GitHub Actions.
- Recreate Origin rules: `main` requires pull requests and `CI gate`;
  `production` requires pull requests, `CI gate`, and
  `Plan production deploy`.

Do not make a GitHub check authoritative in Origin or an Origin check
authoritative in the legacy GitHub repository; the two systems do not share
check identity.

## Phase 4: preserve the macOS CLI contracts without a Mac builder

1. Pin Zig and `cargo-zigbuild` in the Nix development/CI environment.
2. Cross-build `aarch64-apple-darwin` and `x86_64-apple-darwin` separately for
   `finitechat`, `fbrain`, and `fsite` on Depot Linux.
3. Remove `fbrain`'s Apple-framework dependency by selecting the kqueue or
   polling watcher backend, with focused watcher behavior tests.
4. Package the six existing thin asset names and checksums unchanged. Do not
   replace them with a universal asset; a universal binary contains the same
   two compiled slices and changes the installer contract.
5. On one real Apple Silicon Mac, execute each arm64 binary natively and each
   x86_64 binary through Rosetta; verify `--version` and representative local
   filesystem behavior for `fbrain`.

If any slice fails, stop the release-lane cutover and explicitly pause that Mac
asset. Do not retain GitHub Actions only to conceal a failed cross-build.

## Phase 5: cut Releases to `finite-releases`

For each component version:

1. Build and checksum assets in Depot.
2. Create a small release-only metadata commit in `finite-releases` containing
   component, version, Origin source commit, build run, asset names, sizes, and
   SHA-256 checksums.
3. Tag that metadata commit with the existing component tag name.
4. Create the GitHub Release and upload assets with their existing names.
5. Refresh the existing component rolling alias release only after all
   versioned assets have been remotely checksum-verified.

Serialize publication per component so two runs cannot race an alias. Backfill
and checksum-verify all existing `finite-mono` Release assets before changing
any installer. Update the three READMEs, release runbook, helper scripts, and
Electron feed constants to the new repository slug; Electron publication
itself remains disabled.

Release cutover gate:

- Install every CLI/architecture from the versioned Release and rolling alias.
- Prove a deliberately corrupted asset fails checksum verification.
- Prove tag retry is idempotent and cannot replace a verified versioned asset
  with different bytes.
- Accept explicitly that copied old `finite-mono` Release URLs stop working
  after that repository becomes private.

## Phase 6: cut image publication to Depot while retaining GHCR

For each image workflow:

1. Build once and save the OCI result in Depot.
2. Authenticate to GHCR with `FINITE_GHCR_USERNAME` and
   `FINITE_GHCR_TOKEN`; promote the saved result with `depot push`.
3. First publish a non-production canary tag to each existing package.
4. Compare the Depot and GHCR manifest digests and inspect the platform list and
   attestations.
5. Pull the GHCR digest without credentials from a clean client.
6. Only then allow production version tags. Deployment inputs remain
   digest-pinned.

New GHCR packages default private. Every future package must be intentionally
linked, made Public, and anonymously pulled before any deployment references
it. Never delete a package or digest still referenced by production, Phala, or
Tinfoil.

## Phase 7: keep production mutation closed

- Move the Deployment Plan calculation and evidence artifact to Depot.
- Verify it uses the exact Origin `production` tip and the same risky-path
  comparison and classification rules.
- Leave `infra/deployments/production.toml` with mutation disabled.
- Do not copy production SSH credentials merely to make the workflow pass.
- Revise ADR-0006 in a separate design session before enabling mutation. That
  decision must replace GitHub environment approval, interruption state, and
  the GitHub Deployment Record backend explicitly.

## Phase 8: Hard Cutover and cleanup

Cut lanes independently as soon as their evidence passes:

1. Make the Origin check required.
2. Disable the corresponding GitHub Actions trigger.
3. Run one authoritative Origin/Depot event and verify its result.
4. Remove the superseded `.github/workflows` file or job, GitHub required
   check, unused secret/variable, and app grant in the same change window.

After CI, Releases, and GHCR publication all pass:

1. Change developer remotes and documentation to Origin.
2. Freeze the GitHub `finite-mono` repository; do not configure ongoing sync.
3. Make GitHub `finite-mono` private.
4. Immediately repeat every anonymous deployed-digest pull and a clean CLI
   install from `finite-releases`.
5. Confirm Origin branch protection and Depot required checks still govern
   `main` and `production`.
6. Revoke any GitHub Actions-only tokens, Apple signing secrets, and inactive
   integrations. Retain only the two bounded GitHub publisher credentials in
   Depot.

## Completion evidence

The migration is complete only when one evidence bundle records:

- authoritative Origin branch/tag inventory and protections;
- Depot PR success, failure, cancellation, retry, branch push, and dispatch;
- component-tag trigger or the documented dispatch fallback;
- Mac CLI cross-build checksums and real-Mac execution results;
- `finite-releases` backfill inventory and clean installs;
- Depot-to-GHCR digest equality and anonymous pull results before and after
  source-repository privacy;
- production mutation still disabled;
- removal of all active GitHub Actions workflows, checks, credentials, and app
  grants; and
- the frozen private state of GitHub `finite-mono`.

## Deferred work

- Electron signing, notarization, updater migration, and a Mac release executor.
- Moving the issue tracker away from GitHub.
- Replacing `finite-releases` and GHCR with GitHub-independent services.
- Full cold-build operation during a total GitHub outage, including vendoring
  or mirroring Marketplace Actions, Nix inputs, Cargo git dependencies, and
  Dockerfile downloads.
- Enabling production mutation after replacing GitHub approval and Deployment
  Record semantics.
