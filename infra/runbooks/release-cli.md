# Cutting a CLI release (finitechat / fsite / fbrain)

GitHub `finitecomputer/finite-mono` is the source authority. GitHub Actions
builds the release from component-scoped source tags and publishes metadata and
binary assets to the public `finitecomputer/finite-releases` repository. The
release repository never contains product source.

Asset names are product contracts. Installers use the per-component rolling
alias rather than GitHub's repository-wide `releases/latest` pointer:

| Component | Source tag | GitHub Actions workflow | Rolling alias |
|---|---|---|---|
| finitechat | `finitechat/vX.Y.Z` | `.github/workflows/release-finitechat.yml` | `finitechat-latest` |
| fsite | `fsite/vX.Y.Z` | `.github/workflows/release-fsite.yml` | `fsite-latest` |
| fbrain | `fbrain/vX.Y.Z` | `.github/workflows/release-fbrain.yml` | `fbrain-latest` |

The release workflows publish CLI archives for linux-x86_64, macos-aarch64,
and macos-x86_64, each with a `.sha256` sibling. `fsite` also publishes
`finitesitesd` for linux-x86_64; `fbrain` also publishes `finite-brain` for
linux-x86_64. The Electron experiment is on hold and is not part of the CLI CD
release path.

Install URL shape:

`https://github.com/finitecomputer/finite-releases/releases/download/<alias>/<asset>.tar.gz`

## Preconditions

- The exact source commit is on GitHub `main` and `CI gate` is green.
- `FINITE_RELEASES_GITHUB_TOKEN` is available to GitHub Actions and scoped only
  to Contents write on `finitecomputer/finite-releases`.
- Repository variable `FINITE_RELEASE_PUBLISH_ENABLED` is exactly `true`. Leave
  it unset for shadow runs so disposable tags cannot publish.
- The version is newer than `git tag -l '<component>/v*'`, and the matching
  release-repository tag does not already identify different metadata.
- The release does not depend on Electron packaging.

## Release-host backfill

The `finite-releases` cutover is backfilled from the old finite-mono releases
with:

```sh
python3 scripts/backfill_releases.py
```

The script copies versioned non-Electron assets first, then refreshes each
rolling alias once to the newest copied version. Use `--dry-run` before a
mutation and repeated `--tag <component>/vX.Y.Z` arguments for a targeted
repair.

## Steps

1. Pick the version `vX.Y.Z` against the latest existing `<component>/v*` tag.
2. If the release changes the server-compatibility story, record that promise
   in `infra/deployment-changelog.md` in the same PR as the final release
   changes. A release that changes nothing about compatibility needs no record
   beyond its tag.
3. Merge the release changes to GitHub `main` and prove the required CI check
   is green.
4. Tag the exact merge commit and push:

   ```sh
   git tag finitechat/vX.Y.Z <main-sha>
   git push origin finitechat/vX.Y.Z
   ```

   Use the same shape with `fsite/` or `fbrain/` prefixes.
5. Watch the matching GitHub Actions release workflow. The workflow derives and
   checks both version and source SHA from the tag. Publication records
   `release.json`, creates the matching component tag in `finite-releases`,
   checksum-verifies every versioned asset after upload, and only then refreshes
   the rolling alias. A retry reuses already verified immutable assets rather
   than rebuilding them.

## Verify

1. Download the versioned archive and checksum from `finite-releases`; verify
   the checksum.
2. Repeat through the rolling alias.
3. Run the component README's clean-install block away from this checkout and
   confirm `--version`.
4. Confirm `release.json` names the source SHA and GitHub Actions run ID.

Example alias verification:

```sh
base=https://github.com/finitecomputer/finite-releases/releases/download/finitechat-latest
curl -fsSLO "$base/finitechat-linux-x86_64.tar.gz"
curl -fsSLO "$base/finitechat-linux-x86_64.tar.gz.sha256"
sha256sum -c finitechat-linux-x86_64.tar.gz.sha256
```

## Rollback

Versioned releases are immutable. Prefer a patch release. If the rolling alias
must move back before a patch is ready, use the alias-only workflow path. It
runs from `main`, fetches and resolves the requested historical GitHub tag,
downloads the previous versioned release, verifies every asset against
`release.json` and its checksum sibling, and moves the alias without rebuilding:

```sh
gh workflow run release-finitechat.yml \
  --repo finitecomputer/finite-mono \
  --ref main \
  -f publish=true \
  -f alias_only=true \
  -f release_tag=finitechat/vPREVIOUS
```

Do not delete a versioned release or overwrite its assets.
