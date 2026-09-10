# CI cache ownership

The native Depot CI workflow gives each cache mechanism one bounded job:

- **Cachix** substitutes trusted Nix closures across workflow runs. Pull requests
  are read-only; pushes to trusted branches and merge-group runs may publish
  closures when `CACHIX_AUTH_TOKEN` is configured.
- **A same-run Nix handoff artifact** carries a locally built Devfinity closure
  from `nix-service-packages` to `devfinity-smoke`. Its strict manifest and
  artifact name are keyed to the source revision and workflow run. Fully
  substituted runs do not create or download it.
- **Depot Cache through `sccache`** stores Rust compiler results. Depot injects
  the WebDAV endpoint and short-lived cache token; Cargo uses `sccache` as its
  compiler wrapper.
- **The `finite-mono-pnpm-store-v1` Depot cache disk** holds only pnpm's
  content-addressed store. The organization-level name is repository-specific,
  and public fork pull requests receive an empty local directory instead of the
  organization disk.

GitHub Actions cache API consumers such as `actions/cache`,
`Swatinem/rust-cache`, and `actions/setup-node` package-manager caching do not
belong in the native workflow: Depot CI does not expose the GitHub cache service
URL. Production deploys, release assets, and their publication paths are
unaffected by this policy.
