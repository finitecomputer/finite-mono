# Cutting a CLI release (finitechat / fsite / fbrain)

Component-scoped tags on finite-mono build and publish the binaries. Asset
names are product contracts — never rename them (`docs/monorepo-doctrine.md`
§3–4). **Hard-cut model (2026-07-08): finite-mono is the only release host**
— no mirror releases; the legacy repos are archived. Because this repo hosts
several components, the repo-wide `releases/latest` URL is meaningless;
installers use the per-component **rolling alias release**, which the publish
workflow refreshes on every versioned release.

| Component | Tag | Workflow | Builds | Rolling alias |
|---|---|---|---|---|
| finitechat | `finitechat/vX.Y.Z` | `release-finitechat.yml` | `finitechat-cli` → bin `finitechat`; signed + notarized Electron app (macOS arm64) | `finitechat-latest` |
| fsite | `fsite/vX.Y.Z` | `release-fsite.yml` | `fsite-cli` → bin `fsite`; + `finitesitesd` (linux) | `fsite-latest` |
| fbrain | `fbrain/vX.Y.Z` | `release-fbrain.yml` | `finite-brain-cli` → bin `fbrain`; + `finite-brain` server (linux) | `fbrain-latest` |

Install URL shape (what READMEs and agents use):
`https://github.com/finitecomputer/finite-mono/releases/download/<alias>/<asset>.tar.gz`

Assets per release: `<name>-linux-x86_64.tar.gz`, `<name>-macos-aarch64.tar.gz`,
`<name>-macos-x86_64.tar.gz` (+ the linux server binaries for fsite/fbrain),
each with a `.sha256` sibling. Built on GitHub-hosted runners (Rust 1.88.0,
`--locked`) — no self-hosted runner dependency for CLI releases.
Finitechat releases additionally publish
`finitechat-electron-macos-aarch64.zip` and its `.sha256` sibling after the app
passes Device parity, Developer ID signing, Apple notarization, stapling, and
Gatekeeper assessment.

## PRECONDITIONS

- The release commit is on `main` and CI is green.
- You know which fielded versions exist: read the component's `[field.*]`
  block in `compat/matrix.toml` before choosing the version.

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
4. Watch the workflow: `build` (3 targets) → `publish` (versioned release,
   then the rolling-alias refresh step). Both steps must succeed — a
   versioned release without the alias refresh means installers silently
   keep getting the previous version; re-run the publish job if the alias
   step failed.

## VERIFY

1. Alias URL serves the new build with a matching sha256:

   ```sh
   base=https://github.com/finitecomputer/finite-mono/releases/download/finitechat-latest
   curl -fsSLO "$base/finitechat-macos-aarch64.tar.gz"
   shasum -a 256 -c <(curl -fsSL "$base/finitechat-macos-aarch64.tar.gz.sha256")
   ```

2. Field-style install — run the exact install block from the component's
   README on a machine that is not your dev checkout; confirm
   `--version` reports the new version.
3. For finitechat, download the Electron ZIP from the alias, verify its
   checksum, and confirm `codesign`, `stapler`, and Gatekeeper accept the
   extracted app.
4. The versioned release page exists (`finitechat/vX.Y.Z`) and the alias
   release notes name that version (the refresh step writes them).

## First-release-from-mono acceptance test

The first time each component releases from mono, additionally:

- exercise the installed binary against production (e.g. finitechat against
  `https://chat.finite.computer`, respecting the contract gate in
  [deploy-finitechat-server.md](deploy-finitechat-server.md));
- update the component's README install block if anything about it proved
  wrong (it now points at the alias URL);
- **then** Paul archives the legacy repo — a mono-built release installed
  and working is the archive gate (doctrine §2).

## ROLLBACK

Releases are additive; prefer rolling forward with a patch release
(`vX.Y.Z+1`) over deleting — cutting the patch automatically re-points the
alias.

1. If the released binary is actively harmful and a patch can't wait:
   re-run the **previous** version's publish job (re-push its tag or use
   workflow re-run) so the alias refresh step re-clobbers the alias assets
   with the good build; or delete the bad versioned release for hygiene:

   ```sh
   gh release delete finitechat/vX.Y.Z --repo finitecomputer/finite-mono
   ```

   Deleting the versioned release does NOT fix the alias — the alias assets
   are copies. Always re-point the alias explicitly.
2. Update `compat/matrix.toml` to remove the withdrawn version.
