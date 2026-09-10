# GitHub and Depot CI migration plan

This plan implements [ADR-0007](adr/0007-github-is-the-source-authority-and-depot-is-ci.md).
GitHub remains the Source Authority throughout the migration. Only CI execution
and delivery orchestration move to native Depot CI.

## Invariants

- `finitecomputer/finite-mono` is the only source repository.
- GitHub owns pull requests, branches, tags, merge rules, and issues.
- Depot workflows live in `.depot/workflows` and report checks to GitHub.
- Pull-request jobs may read Cachix; only trusted `main` jobs may write it.
- macOS CLI and Electron release lanes stay disabled until separately restored.
- Production mutation stays disabled.
- Release assets stay public in `finitecomputer/finite-releases`; images stay in
  public GHCR.

## Phase 1: prove GitHub event and check behavior

1. Install the Depot GitHub integration for `finitecomputer/finite-mono`.
2. Configure repository-scoped Depot secrets and variables.
3. Prove pull-request open, synchronize, failure, retry, cancellation,
   concurrency cancellation, artifact, branch push, tag, and manual-dispatch
   behavior.
4. Confirm Depot reports stable check names on the GitHub commit and pull
   request.
5. Require the accepted `CI gate` check in the GitHub `main` ruleset.

## Phase 2: prove cache behavior

1. Confirm Nix is installed with the `finite.cachix.org` substituter and public
   key before builds start.
2. Keep pull requests read-only with respect to the cache.
3. Run the same expensive Nix job twice and retain logs that show substituted
   paths on the second run.
4. Run a trusted `main` build with `CACHIX_AUTH_TOKEN` and retain successful
   upload evidence.

## Phase 3: cut over CI lanes

For each lane:

1. Run the native Depot workflow alongside its GitHub Actions predecessor.
2. Compare commands, environment, permissions, artifacts, check conclusions,
   and cancellation behavior.
3. Make the Depot check required in GitHub.
4. Remove the duplicate GitHub Actions workflow and its Actions-only secrets or
   environment grants.
5. Run one post-removal GitHub event and record the Depot result.

## Phase 4: release and image canaries

1. Prove a non-production Linux release canary with the scoped
   `FINITE_RELEASES_GITHUB_TOKEN`.
2. Prove alias-only rollback without rebuilding immutable release assets.
3. Prove GHCR canary digest equality, platform, provenance, and anonymous pull
   with the bounded package credential.
4. Enable normal publication variables only after the corresponding canary and
   rollback evidence pass.

## Phase 5: close the migration

1. Remove obsolete GitHub Actions workflows and credentials lane by lane.
2. Confirm every required GitHub ruleset check is produced by Depot.
3. Confirm clean clones and developer remotes still use GitHub.
4. Record the final accepted runs in
   `docs/migrations/github-depot/evidence-2026-08-25.md`.
