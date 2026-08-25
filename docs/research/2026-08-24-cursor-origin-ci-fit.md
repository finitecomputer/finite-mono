# Cursor Origin CI fit for `finite-mono`

Date: 2026-08-24

Status: historical research. ADR-0007 supersedes its initial Buildkite/Mac
recommendation after Electron was explicitly deferred and Linux-to-Darwin CLI
cross-compilation became an accepted cutover gate.

## Question

Can `finite-mono` stop relying on GitHub Actions by moving its source of truth to Cursor Origin, and do its remaining macOS jobs prevent that move?

## Conclusion

The repository does not currently need macOS for routine pull-request CI. The only macOS PR job, `electron-alpha`, is deliberately never selected by `scripts/ci/select-harnesses`. macOS is still required at release time for the documented macOS CLI artifacts and, most importantly, for the Developer ID signing, Apple notarization, stapling, and Gatekeeper validation of the FiniteChat Electron application.

That release-only requirement does **not** force the project to retain GitHub Actions. Buildkite integrates directly with Origin and offers hosted macOS agents, so it can run the Apple release stage. Native Depot CI integrates directly with Origin but offers only Linux sandboxes; its GitHub-hosted macOS runner product does not carry over to native Depot CI.

A complete GitHub exit is nevertheless premature for reasons larger than CI:

- Origin is an early beta and currently creates only Internal or Private repositories, whereas `finite-mono` is a public repository.
- GitHub Releases is part of the product distribution contract. The three CLI installers and the Electron updater use GitHub release URLs, and the release workflows maintain rolling aliases there.
- Production images currently publish to GHCR.
- Native Depot CI does not support GitHub Actions `environment` gates, and its generated `GITHUB_TOKEN` cannot publish to GitHub Packages/GHCR.

The lowest-risk path is therefore an Origin pilot, not an immediate detach: run Linux PR CI in Depot, run the release-only Mac stage in Buildkite, and retain GitHub as the public mirror and release/registry host until public repository visibility, binary distribution, container registry, and deployment approvals have explicit replacements.

## Current macOS dependency

### Routine CI: no active dependency

`.github/workflows/ci.yml` defines `electron-alpha` on `macos-14`, but `scripts/ci/select-harnesses` initializes `run_electron_alpha` to false and explicitly leaves it false when selecting all harnesses: "Electron CI remains disabled until the Electron surface is active." No code sets it to true. Consequently, ordinary PR and main-branch CI currently runs on Depot Linux runners, not macOS.

### Releases: active dependency

The three component release workflows build macOS arm64 and x86_64 CLI assets:

- `.github/workflows/release-finitechat.yml`
- `.github/workflows/release-fbrain.yml`
- `.github/workflows/release-fsite.yml`

The FiniteChat workflow additionally uses `macos-14` to import the Apple Developer ID certificate into a Keychain, sign the Electron bundle, submit it with `xcrun notarytool`, staple the ticket, and validate it with `spctl`. Those steps require a real macOS execution environment. Cross-compiling the Rust CLI from Linux could reduce Mac build time, but it would add Apple SDK/linker complexity and would not eliminate the Mac signing/notarization stage.

The repository READMEs advertise the macOS binaries, and `finitechat/README.md` advertises the signed/notarized Apple Silicon desktop application. These are current distribution contracts, not incidental CI artifacts. Removing them should be an explicit product deprecation rather than a CI optimization.

One Apple Silicon hosted Mac can produce the two Rust CLI architectures; there is no need to maintain separate physical arm64 and x86_64 Mac fleets. The Electron artifact is currently arm64-only.

## Provider fit

| Shape | Linux PR CI | Mac release | Origin checks/triggers | Main limitations |
|---|---|---|---|---|
| Origin + Depot only | Yes | No | Direct Origin integration | Native Depot CI has no macOS sandbox; `environment` unsupported; GHCR needs a PAT or registry change |
| Origin + Depot + Buildkite | Depot | Buildkite | Both integrate directly | Two CI systems, plus distribution/registry work |
| Origin + Buildkite only | Buildkite | Buildkite | Direct Origin integration, including tag pushes | More migration away from existing Depot investment |
| GitHub Actions + Depot runners (current) | Depot-backed | GitHub macOS | GitHub | Still coupled to GitHub Actions availability |

### Depot

Depot's Origin integration runs `.depot/workflows` for pushes and pull requests from an Origin-hosted repository and reports checks back to Origin. Native Depot CI provides x86_64 and arm64 Ubuntu sandboxes. Its compatibility documentation states that `runs-on` is not supported in the GitHub Actions sense: non-Depot labels are treated as the default Depot Ubuntu environment. The macOS labels documented for Depot-managed GitHub Actions runners are not available in native Depot CI.

Relevant migration differences include:

- GitHub Actions `environment` is unsupported, affecting the current production and staging approval boundary.
- `secrets.GITHUB_TOKEN` cannot publish to GitHub Packages/GHCR from Depot CI; use a GitHub PAT or move the images.
- GitHub-native release events are unsupported. Manual dispatch by ref/tag is available, but the exact Origin tag-trigger behavior should be proven in the pilot rather than assumed.

### Buildkite

Buildkite's direct Origin source-control integration receives branch pushes, tag pushes, and pull-request events and publishes check results back to Origin. Buildkite-hosted macOS queues currently include Apple Silicon machines for macOS 14 and 15, and its GitHub Actions compatibility layer maps labels such as `macos-14` to those queues. That makes Buildkite a viable home for the current FiniteChat signing/notarization lane and the macOS CLI release matrix.

Buildkite can either host only the Mac release stage, preserving Depot for fast Linux CI, or replace the whole GitHub Actions control plane. The first shape makes the smallest change; the second has a simpler long-term operating model.

## Non-CI blockers to a full GitHub detach

1. **Public source hosting.** Origin currently supports only Internal and Private repository visibility. `finite-mono` is public by policy. A dual-push public GitHub mirror is possible during evaluation, but Origin's direct Depot and Buildkite apps require an Origin-hosted repository, not an Origin view that merely mirrors GitHub.
2. **Release distribution.** Installer scripts and the Electron updater resolve assets under `github.com/finitecomputer/finite-mono/releases/download/...`. Origin's documented beta surface does not include a GitHub-Releases-equivalent binary distribution service. A full exit needs a durable object store/CDN or another release host, preserved asset names, atomic rolling aliases, checksums, and updater compatibility.
3. **Container distribution.** Image workflows publish to GHCR. Either retain GHCR with a narrowly scoped PAT independent of Actions or migrate the image names and deployment configuration to another registry.
4. **Approval and identity model.** GitHub deployment environments currently protect production and staging jobs. Their approvals, secrets, audit trail, and branch restrictions need an equivalent outside Actions.
5. **Public collaboration.** Origin's current visibility limitation also affects anonymous browsing, cloning, issues, and outside contributions. A public mirror strategy needs an explicit source-of-truth and contribution workflow.

## Recommended pilot

1. Dual-push a disposable or non-authoritative branch to an Origin-hosted repository; do not detach the public GitHub repository yet.
2. Move a representative Linux PR workflow to `.depot/workflows` and prove PR triggers, required checks, cancellation, artifacts, secrets, and merge protection.
3. Add a Buildkite Origin pipeline with a harmless macOS canary: build both Rust targets, ad-hoc-sign a test bundle, archive it, and publish its check. Do not expose the production Apple certificate during the canary.
4. Separately design the release host, updater URL migration, registry, and deploy-approval replacement. Test release alias and rollback semantics before switching any production installer.
5. Only make Origin authoritative after the public-repository story and release distribution are settled. If GitHub-outage independence is the aim, queue public-mirror and GitHub-release synchronization for retry rather than making Origin CI depend on GitHub being online.

## Primary sources

- [Cursor: Origin code hosting changelog](https://cursor.com/changelog/origin-code-hosting)
- [Cursor: Origin repository settings and detaching from GitHub](https://cursor.com/docs/origin/settings)
- [Cursor: Create an Origin repository](https://cursor.com/docs/origin/create-repository)
- [Cursor: Origin Git and dual-push workflow](https://cursor.com/docs/origin/git)
- [Depot: Depot CI for Cursor Origin](https://depot.dev/blog/depot-in-cursor-origin)
- [Depot: CI overview and available sandboxes](https://depot.dev/docs/ci/overview)
- [Depot: GitHub Actions compatibility](https://depot.dev/docs/ci/compatibility)
- [Buildkite: Cursor Origin source-control integration](https://buildkite.com/docs/pipelines/source-control/origin)
- [Buildkite: hosted macOS agents](https://buildkite.com/docs/agent/buildkite-hosted/macos)
- [Buildkite: migration from GitHub Actions](https://buildkite.com/docs/pipelines/migration/from-githubactions)
