# crate2nix through the repository flake

This draft PR tests two real top-level package outputs alongside current Crane
packaging. The root Cargo.toml and Cargo.lock remain authoritative. Cargo.nix
and crate-hashes.json are upstream-generated build definitions, checked into git
and marked generated for GitHub review. Regenerate with:

```sh
nix develop .#crate2nix -c crate2nix generate
```

Build the candidates directly from the checkout:

```sh
nix build .#finitesitesd-crate2nix .#finite-saas-core-crate2nix
```

Compare with `nix build .#finitesitesd .#finite-saas-core`. Both use the same
packaging compiler and native library pins. Normal package outputs and NixOS
hosts still select Crane. No candidate outputs are published or deployed.

## CI

The workflow verifies that the generated Cargo files are current, then runs the
same four scenarios through `nix build .#...`: initial build, identical rebuild,
Sites source edit, and adding `hex` to the Sites engine. It edits the actual CI
checkout, regenerates Cargo.nix after the dependency edit, and restores those
files on exit. There are no source archives or alternate benchmark Nix imports.
A clean tracked checkout is required to run the script locally:

```sh
nix develop .#crate2nix -c bash infra/nixos/crate2nix-prototype/benchmark \
  crate2nix /tmp/new-crate2nix-benchmark
```

Use `crane` for the other engine. The jobs use matching runner classes and Nix
parallelism settings, public cache substitution for initial builds, and the
local store for rebuilds. This does not measure fresh-runner cache transfers.
Timings and logs are Actions artifacts. Compiler flags, feature resolution,
and scheduling can still differ between implementations.

After timing, CI runs the existing `scripts/devfinity-smoke` against
`.#devfinity-crate2nix`: the normal devfinity wrapper with only Sites and SaaS
Core replaced by the candidate outputs. Other services and CLIs retain their
current builds. This exercises real local services, Postgres, Sites publishing
and viewer sessions, account/Agent creation, and the existing Chat regressions.
It is an integration check on synthetic state, not production migration proof.

The earlier benchmark saved about six minutes on the Sites dependency edit
(~20x including preparation); see PR #892's previous Actions run 35013826299.
Its source-edit timings were approximately equal. Feature differences observed
in hyper-util, icu_provider, and tokio, full package coverage, and cache transfer
costs still need resolution before adopting crate2nix throughout the fleet.

Upstream: [generation](https://nix-community.github.io/crate2nix/20_generating/10_generating/),
[workspace builds](https://nix-community.github.io/crate2nix/30_building/10_building_binaries/).
