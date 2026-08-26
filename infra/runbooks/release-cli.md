# Cutting a CLI release (finitechat / fsite / fbrain)

Cursor Origin is the source authority. Native Depot CI builds the release and
publishes it to the public `finitecomputer/finite-releases` GitHub repository.
The release repository contains metadata and binary assets only; it never
contains the product source.

Asset names are product contracts. Installers use the per-component rolling
alias rather than GitHub's repository-wide `releases/latest` pointer:

| Component | Source tag | Depot workflow | Rolling alias |
|---|---|---|---|
| finitechat | `finitechat/vX.Y.Z` | `.depot/workflows/release-finitechat.yml` | `finitechat-latest` |
| fsite | `fsite/vX.Y.Z` | `.depot/workflows/release-fsite.yml` | `fsite-latest` |
| fbrain | `fbrain/vX.Y.Z` | `.depot/workflows/release-fbrain.yml` | `fbrain-latest` |

The migrated Depot workflows currently publish Linux x86_64 archives and their
`.sha256` siblings only. macOS CLI and Electron publication are paused until a
dedicated macOS release lane is reintroduced. Existing versioned macOS assets
remain available, but a new release must not claim to refresh them.

Install URL shape:

`https://github.com/finitecomputer/finite-releases/releases/download/<alias>/<asset>.tar.gz`

## PRECONDITIONS

- The exact source commit is on Origin `main` and `CI gate` is green.
- Depot holds `FINITE_RELEASES_GITHUB_TOKEN`, scoped only to Contents write on
  `finitecomputer/finite-releases`.
- Depot variable `FINITE_RELEASE_PUBLISH_ENABLED` is exactly `true`. It remains
  unset during Shadow Runs so disposable tags cannot publish.
- The version is newer than `git tag -l '<component>/v*'` and the corresponding
  release-repository tag does not identify different metadata.
- The release does not depend on Electron packaging.

## STEPS

> **TODO (cutover canary):** These steps remain proposed until an Origin tag or
> an explicit tag-ref dispatch has exercised the scoped credential, all build
> rows, the publisher, and the rolling alias end to end. Record that run in
> `docs/migrations/origin-depot/evidence-2026-08-25.md`, then remove the TODO
> labels from the exercised path.

1. **TODO (Origin source authority):** Merge the release changes to Origin
   `main` and prove the required Origin check is green.
2. **TODO (Origin tag event):** Create the component-scoped tag at that exact
   commit and push it to Origin:

   ```sh
   git tag finitechat/vX.Y.Z <origin-main-sha>
   git push origin finitechat/vX.Y.Z
   ```

3. **TODO (Depot dispatch semantics):** If Origin tag events are connected to
   Depot, watch the matching release workflow. If they are not, dispatch that
   workflow at the fully qualified Origin tag. The workflow derives and checks
   both version and source SHA from that ref:

   ```sh
   depot ci dispatch \
     --repo finite-co/finite-mono \
     --workflow release-finitechat.yml \
     --ref refs/tags/finitechat/vX.Y.Z \
     --input publish=true \
     --input alias_only=false
   ```

4. **TODO (publisher canary):** Wait for the Linux build row and `publish` to
   finish. Publication records a metadata commit in `finite-releases`, creates
   the matching component tag, checksum-verifies every versioned asset after
   upload, and only then refreshes the rolling alias. A retry reuses the already
   verified immutable assets rather than rebuilding them.

## VERIFY

1. Download the versioned archive and checksum from `finite-releases`; verify
   the checksum.
2. Repeat through the rolling alias.
3. Run the component README's clean-install block away from this checkout and
   confirm `--version`.
4. Confirm `release.json` names the Origin source SHA and Depot run ID.

Example alias verification:

```sh
base=https://github.com/finitecomputer/finite-releases/releases/download/finitechat-latest
curl -fsSLO "$base/finitechat-linux-x86_64.tar.gz"
curl -fsSLO "$base/finitechat-linux-x86_64.tar.gz.sha256"
sha256sum -c finitechat-linux-x86_64.tar.gz.sha256
```

## ROLLBACK

Versioned releases are immutable. Prefer a patch release. If the rolling alias
must move back before a patch is ready, use the alias-only workflow path. It
runs from `main` (where the workflow exists), fetches and resolves the requested
historical Origin tag, downloads the previous versioned release, verifies every
asset against `release.json` and its checksum sibling, and moves the alias
without rebuilding:

```sh
depot ci dispatch \
  --repo finite-co/finite-mono \
  --workflow release-finitechat.yml \
  --ref main \
  --input publish=true \
  --input alias_only=true \
  --input release_tag=finitechat/vPREVIOUS
```

**TODO (rollback rehearsal):** Exercise this against a non-current historical
version, verify the alias, then return it to the intended current version with
the same alias-only path. Do not delete a versioned release or overwrite its
assets.
