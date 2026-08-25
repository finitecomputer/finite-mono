---
status: accepted
---

# Cursor Origin is the source authority and Depot CI is the CI control plane

Finite will move the `finite-mono` Source Authority from GitHub to a private
Cursor Origin repository and will move all GitHub Actions orchestration to
native Depot CI. After the Hard Cutover, the GitHub `finite-mono` repository is
a frozen private legacy repository: it is neither synchronized nor accepted as
a source of changes. GitHub issue-tracker migration is outside this decision.

Product Releases remain public through a dedicated
`finitecomputer/finite-releases` Release Repository. It owns release-only
metadata commits, component version tags, rolling alias tags, release notes,
and assets; it never contains or accepts product source. Existing asset names,
checksums, component tag names, and rolling aliases remain contracts. The
Artifact Registry remains public GHCR. These choices deliberately retain
GitHub as the Release Host and Artifact Registry in exchange for a same-day,
low-change migration; this ADR does not claim complete GitHub outage
independence.

Native Depot CI publishes Releases with a repository-scoped fine-grained
GitHub token and publishes images with a separate classic GitHub token scoped
only to `write:packages`. Existing public GHCR packages must remain anonymously
pullable when the source repository becomes private. Publication authority is
proved with canary tags and digest comparisons before repository privacy
changes.

The current macOS CLI asset contracts remain. Depot Linux cross-compiles the
existing arm64 and x86_64 binaries with a pinned SDK-free Zig toolchain and the
binaries are validated on a real Mac. A universal binary may be added later but
does not replace the existing thin assets. The inactive Electron build,
Developer ID signing, and notarization lane are deferred and are not migration
requirements.

Production planning and validation move to Depot, but production mutation
remains disabled until a separate decision replaces GitHub environments and
GitHub-backed Deployment Records. Each migration lane first runs as a
non-mutating Shadow Run. Once its agreed evidence passes, it receives a Hard
Cutover: the Depot/Origin path becomes authoritative and the GitHub Actions
workflow, required check, credential, and app grant are removed rather than
operated indefinitely in parallel.

## Considered options

- **Keep GitHub Actions with Depot-managed runners:** rejected because GitHub
  would remain the event, authorization, and check authority.
- **Use Depot Registry for production images:** rejected for now because Depot
  documents authenticated pulls only, while Finite's deployed consumers use
  anonymous digest-pinned pulls.
- **Use Cloudflare R2 or Finite production infrastructure for Releases:** R2 is
  a plausible later destination, but the operator does not currently control
  the Cloudflare account; serving from the production application host would
  couple release availability to product availability.
- **Retain the Electron Mac lane through Buildkite:** rejected for this
  migration because Electron distribution is inactive and explicitly deferred.

## Consequences

- `docs/monorepo-doctrine.md` no longer treats GitHub `finite-mono` as the only
  release host or as a permanently public repository.
- Origin tag delivery, Origin required checks, Depot manual dispatch, SDK-free
  Mac cross-compilation, GHCR publication, and anonymous pulls are cutover
  gates, not assumptions.
- GitHub remains a runtime dependency for public Releases and containers until
  a later ADR replaces those services.
- The proposed GitHub-based production deployment design in ADR-0006 remains
  non-mutating and must be revised before production deployment is enabled.
