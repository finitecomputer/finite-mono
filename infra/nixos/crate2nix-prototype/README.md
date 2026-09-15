# crate2nix packaging experiment

The candidate uses upstream's documented workflow: run `crate2nix generate`,
import `Cargo.nix`, and build `workspaceMembers.<name>.build`.
`benchmark.nix` supplies our existing nixpkgs pin and selects two services.
No custom dependency resolver or Python harness is needed.

The draft PR workflow runs separate Crane and crate2nix jobs on the same Depot
runner class. Both build Sites (`finitesitesd`) and unrelated `finite-saas-core`.
The shell script times four cases: initial build, identical local rebuild,
Sites source edit, and adding the existing `hex` dependency to the Sites engine.
Cargo updates the scratch lockfile. Each case starts from committed HEAD.

Results are in Actions job summaries and downloadable logs. Both jobs allow
the public Nix cache, exclude Finite's production cache, and use eight Nix jobs
with eight cores per job. Initial builds can substitute public dependencies;
subsequent cases reuse the local store. This does not measure fresh-runner
binary-cache transfers. Compiler versions match production packaging, but
default codegen flags and scheduling differ between the implementations.

Run on a disposable Linux machine from the repo root:

```sh
nix shell --impure --file infra/nixos/crate2nix-prototype/tools.nix \
  -c bash infra/nixos/crate2nix-prototype/benchmark crate2nix /tmp/new-benchmark
```

Use `crane` for the other implementation. Existing local artifacts affect times.
Production configuration is unchanged; candidate outputs are not published.

The earlier probe preserved 725 of 727 candidate crate derivations after the
Sites dependency addition, while all eight Crane bundles changed. Its code and
raw results remain in git history at `f8be513c`. A Sites library build passed on
macOS, but a feature spot check found differences in hyper-util, icu_provider,
and tokio. Investigate those and test Linux service behavior, embedded assets,
Chat fingerprints, and cache transfers before considering a migration.

Sources: [generation](https://nix-community.github.io/crate2nix/20_generating/10_generating/),
[workspace builds](https://nix-community.github.io/crate2nix/30_building/10_building_binaries/).
