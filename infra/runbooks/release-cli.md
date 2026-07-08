# Cutting a CLI release (finitechat / fsite / fbrain)

Component-scoped tags on finite-mono build, publish, and mirror the CLI
binaries. Asset names are product contracts — never rename them
(`docs/monorepo-doctrine.md` §3–4).

| Component | Tag | Workflow | Builds | Mirror repo (tag `vX.Y.Z`) |
|---|---|---|---|---|
| finitechat | `finitechat/vX.Y.Z` | `.github/workflows/release-finitechat.yml` | `finitechat-cli` → bin `finitechat` | `finitecomputer/finitechat` |
| fsite | `fsite/vX.Y.Z` | `.github/workflows/release-fsite.yml` | `fsite-cli` → bin `fsite` | `finitecomputer/finite-sites` |
| fbrain | `fbrain/vX.Y.Z` | `.github/workflows/release-fbrain.yml` | `finite-brain-cli` → bin `fbrain` | `finitecomputer/finite-brain` |

Assets per release: `<name>-linux-x86_64.tar.gz`, `<name>-macos-aarch64.tar.gz`,
`<name>-macos-x86_64.tar.gz`, each with a `.sha256` sibling. Built on
GitHub-hosted runners (Rust 1.88.0, `--locked`) — no self-hosted runner
dependency for CLI releases.

## PRECONDITIONS

- The release commit is on `main` and CI is green.
- Org secret `RELEASE_MIRROR_TOKEN` is configured on finite-mono.
  **Required scope: `contents: write` on the legacy repos**
  (finitecomputer/finitechat, finite-sites, finite-brain) — it creates
  releases and uploads assets there. If unset, the mirror job skips with a
  notice and the field install URL silently goes stale — treat a skipped
  mirror as a failed release.
- The legacy mirror repo is **not archived** (archived repos cannot receive
  releases — doctrine §4).
- You know which fielded versions exist: read the component's
  `[field.*]` block in `compat/matrix.toml` before choosing the version.

## STEPS

1. Pick the version `vX.Y.Z` (semver against the previous entry in
   `compat/matrix.toml`).
2. In the same PR as any final release changes, update `compat/matrix.toml`:
   append the new version to the component's `released` list (create the
   list entry for fbrain's first release) and adjust `notes` if the
   server-compatibility story changed. Merge to `main`.
3. Tag the merge commit on `main` and push:

   ```sh
   git tag finitechat/vX.Y.Z <main-sha>
   git push origin finitechat/vX.Y.Z
   ```

   (same shape with `fsite/` or `fbrain/` prefixes.)
4. Watch the workflow: `build` (3 targets) → `publish` (release on
   finite-mono at tag `finitechat/vX.Y.Z`) → `mirror` (release on the legacy
   repo at tag `vX.Y.Z`, notes link back to the canonical mono release).
5. Confirm the mirror job did **not** print
   `RELEASE_MIRROR_TOKEN not configured; skipping mirror release.` If it
   did, fix the secret and re-run the mirror job.

## VERIFY

1. sha256s match between the two repos and the archive:

   ```sh
   curl -fsSLO https://github.com/finitecomputer/finitechat/releases/latest/download/finitechat-macos-aarch64.tar.gz
   curl -fsSL  https://github.com/finitecomputer/finitechat/releases/latest/download/finitechat-macos-aarch64.tar.gz.sha256
   shasum -a 256 -c <(curl -fsSL https://github.com/finitecomputer/finitechat/releases/latest/download/finitechat-macos-aarch64.tar.gz.sha256)
   ```

2. Field-style install — exactly what agents in the field do (the
   `releases/latest/download/` URL on the **legacy** repo, per
   `compat/matrix.toml` `install_url`): extract and run the binary
   (`./finitechat --version`, or the component's doctor/health subcommand)
   and confirm it reports the new version.
3. Check the legacy repo's `releases/latest` actually points at the new
   mirror release (an old release edited later can steal `latest`).

## First-release-from-mono acceptance test

The first time each component releases from mono, additionally:

- run the field-style install above on a machine that is not your dev
  checkout, from the legacy `releases/latest/download/` URL;
- exercise the installed binary against production (e.g. finitechat against
  `https://chat.finite.computer`, respecting the contract gate in
  [deploy-finitechat-server.md](deploy-finitechat-server.md)).

**Do NOT archive a legacy repo until a mono-built release has been installed
in the field through its URL.** Until then the legacy repo may still need an
emergency release of its own (doctrine §2, §4).

## ROLLBACK

Releases are additive; prefer rolling forward with a patch release
(`vX.Y.Z+1`) over deleting.

1. If the released binary is actively harmful: delete the release (and its
   tag if needed) on **both** finite-mono and the mirror repo so
   `releases/latest` falls back to the previous good version:

   ```sh
   gh release delete finitechat/vX.Y.Z --repo finitecomputer/finite-mono
   gh release delete vX.Y.Z --repo finitecomputer/finitechat
   ```

2. Update `compat/matrix.toml` to remove the withdrawn version.
3. TODO: verify during the first withdrawal that deleting a mirror release
   makes `releases/latest/download/` resolve to the previous release
   immediately (GitHub caching behavior unproven here).
