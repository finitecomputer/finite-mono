---
status: accepted
---

# GitHub is the source authority and Depot CI is the CI control plane

`finitecomputer/finite-mono` remains the canonical repository. GitHub owns
branches, tags, pull requests, merge rules, and issues. Native Depot CI consumes
GitHub events, executes the repository-owned workflows in `.depot/workflows`,
and reports checks back to GitHub.

There is no repository-hosting migration, mirror, dual-write period, or second
source authority. Developer remotes continue to point at GitHub. A GitHub
Actions workflow may be removed after its corresponding Depot workflow has
passed its event, permission, artifact, secret, retry, cancellation, and
required-check acceptance gates.

Product releases remain public through the dedicated
`finitecomputer/finite-releases` repository. It owns release-only metadata
commits, component version tags, rolling alias tags, release notes, and assets;
it never contains or accepts product source. The Artifact Registry remains
public GHCR.

Depot release workflows publish Linux x86_64 CLI assets only. macOS CLI builds
and Electron signing/notarization remain disabled until a dedicated macOS
execution path is selected and qualified. Existing immutable macOS assets are
retained.

Cachix reads are available to all CI jobs. Cache writes require the scoped
`CACHIX_AUTH_TOKEN` and are permitted only for trusted `main` executions, never
for untrusted pull-request code.

Production planning and validation may move to Depot, but production mutation
remains disabled until a separate decision replaces the existing deployment
authorization and audit contracts.

## Considered options

- **Move source hosting away from GitHub:** rejected. The attempted hosting
  migration did not provide a reliable enough repository and collaboration
  surface.
- **Keep GitHub Actions with Depot-managed runners:** viable but retains GitHub
  Actions as the orchestration layer. Native Depot CI is preferred where its
  behavior has been proven.
- **Operate both CI control planes indefinitely:** rejected because duplicate
  workflows create ambiguous status, credentials, and operational ownership.
- **Cross-build macOS binaries on Linux:** rejected because it expands the CI
  migration into a separate toolchain and platform-qualification project.

## Consequences

- GitHub outages can affect source collaboration even though Depot executes CI.
- Depot outages can affect CI without changing repository authority.
- GitHub rulesets require the accepted Depot check names after each lane cuts
  over.
- GitHub Actions workflows, secrets, environments, and app grants are removed
  only when their accepted Depot replacements no longer depend on them.
- Release and image publishing remain fail-closed behind explicit variables and
  scoped credentials.
