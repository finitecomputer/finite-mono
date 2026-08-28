# Production CD bootstrap and deploy

This runbook turns the production deploy conductor on without treating the old
`production` branch experiment as authority. The hard cut is deliberate:
bootstrap establishes the branch and GitHub governance from a named `main`
revision, then a later tiny PR enables production mutation.

## What This Does Not Do

- It does not publish CLI releases. Use [release-cli.md](release-cli.md).
- It does not promote or roll Agent Runtime images. Use
  [runtime-image.md](runtime-image.md).
- It does not automate rollback. Use the service runbooks or
  `nixos-rebuild --rollback` for MVP rollback.

## Preconditions

- The production CD scaffold is merged to `main`.
- `infra/deployments/production.toml` still has `mutation_enabled = false`.
- You have GitHub admin access for `finitecomputer/finite-mono`.
- You have the production deploy SSH private key and pinned lat1 `known_hosts`
  entry in off-repo custody.

## One-Time Bootstrap

1. Pick the scaffold commit on `main`:

   ```sh
   git fetch origin main --prune
   BOOTSTRAP_SHA="$(git rev-parse origin/main)"
   echo "$BOOTSTRAP_SHA"
   ```

2. Hard-cut the `production` branch to that commit. If the old failed
   experiment's branch or ruleset blocks the update, disable that stale
   protection first, then recreate it from the steps below. Do not treat the
   old branch state as production evidence.

   ```sh
   git push origin "+$BOOTSTRAP_SHA:refs/heads/production"
   ```

3. Create or replace the GitHub Environment named `production`:

   - Required reviewers: at least one human reviewer.
   - Deployment branch policy: protected branches only, or a custom policy that
     permits only `production`.
   - Environment secrets:
     - `FINITE_PRODUCTION_SSH_KEY`
     - `FINITE_PRODUCTION_KNOWN_HOSTS`

4. Create or replace the repository ruleset named `production`:

   - Target: `refs/heads/production`.
   - Enforcement: active.
   - Block deletion.
   - Block non-fast-forward updates.
   - Require pull requests.
   - Require at least one approving review.
   - Allow merge commits; do not require arbitrary local pushes.
   - Require status checks:
     - `CI gate`
     - `Plan production deploy`

5. Verify setup from a clean checkout:

   ```sh
   scripts/verify-production-cd-setup
   ```

   The verifier is read-only. If it reports missing environment, secret, or
   ruleset setup, fix GitHub configuration and rerun it.

## Enable Mutation

After the verifier passes, open a tiny PR to `main` that only changes:

```toml
mutation_enabled = true
```

in `infra/deployments/production.toml`. Merging that PR is the explicit
boundary where production mutation becomes available, but it still does not
deploy by itself.

## Open A Deploy PR

Use the manual GitHub Actions workflow:

```text
Open Production Deploy PR
```

It opens or reports the normal `main` to `production` pull request. It does
not accept an arbitrary SHA and does not push a branch. Because the helper uses
GitHub Actions' `GITHUB_TOKEN`, GitHub may show an "Approve workflows" banner on
the opened pull request before `Production Deploy Plan` runs; approve that
banner if it appears.

Review the pull request comment from `Production Deploy Plan`. The first
enabled PR after bootstrap is a **bootstrap Production Deploy**: it installs
the current `main` closure and establishes the production branch as the
ongoing review surface. After that, future production PRs are ordinary
`production...main` diffs.

## Merge And Approve

1. Merge the `main` to `production` PR only after:
   - the staged source SHA has a successful `CI gate` result.
   - `Plan production deploy` is green.
   - the deployment plan has been reviewed.
2. The `Production Deploy` workflow starts from the `production` branch tip.
3. If `mutation_enabled = true`, the deploy job waits for approval in the
   GitHub `production` environment before crossing the Mutation Boundary.
4. Approve only after confirming this is the intended source revision.

## Evidence

The workflow records:

- `deployment-plan`
- `lat1-nixos-closure-<sha>` metadata for the exact CI-built, Cachix-published
  NixOS closure
- `deployment-record`
- `deployment-transport.json` inside the deployment record artifact
- `finite-status-before`
- `finite-status-after`

The deploy job stages the checked-out revision's `scripts/finite-status` and
`scripts/finite_status.py` on lat1, runs them through the mono checkout's
pinned `nixpkgs` Python, and requires non-empty valid JSON for the pre/post
status artifacts. Before writing the mutation-boundary marker, it makes lat1
realize and validate the pinned closure from the `finite` Cachix cache with
local builds disabled. Routine activation repeats that validity check, switches
the exact `SYSTEM` path, and then verifies the activated host declares the same
Cachix trust; file-cache transport requires explicit bootstrap/recovery
selection.

## Failure And Unblock

MVP does not implement automatic lockout or automated rollback. If the deploy
fails before the Mutation Boundary, fix the source or setup and retry normally.

If it fails after the Mutation Boundary:

1. Inspect lat1 directly.
2. Decide whether production is on the intended closure, partially switched, or
   rolled back.
3. If safe, rerun the same deploy workflow for the current `production` tip.
4. Otherwise follow the relevant manual rollback runbook, usually:

   ```sh
   ssh root@64.34.82.77 nixos-rebuild switch --rollback
   ```

5. Record the observed system path, action taken, and reason in the deployment
   record/changelog before approving another production deploy.
