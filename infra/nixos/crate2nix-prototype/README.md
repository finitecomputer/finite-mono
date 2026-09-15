# Throwaway crate2nix experiment

## Verdict

**Proceed to a Linux packaging benchmark; do not migrate production yet.**
crate2nix directly addresses the lockfile-wide invalidation in our Crane
packaging. This experiment proves narrower cache identities on real Finite
sources. It does not measure CI speed or establish production equivalence.

Prototype branch: `alex/crate2nix-prototype`. Nothing imports this directory
from the production flake, package definitions, or CI. Keep this on the
prototype branch until a migration decision is made.

## Run

From the repository root:

```sh
infra/nixos/crate2nix-prototype/run
```

Requires the existing Nix installation and network access. Tools come from the
root flake.lock; nothing is installed into a user profile. Uses crate2nix
0.14.1 from the existing nixpkgs pin. Cargo metadata generation uses the root
rust-toolchain.toml (1.93.1 at the tested revision); both evaluated packaging
implementations use the existing package nixpkgs Rust (1.91.1). This existing
shell/package toolchain difference must be accounted for in the next phase.

The command archives committed HEAD into a new temporary directory and prints
its location. `--output /absolute/new/directory` selects the evidence directory.
`--ref COMMIT` selects another committed source snapshot, using that snapshot's
package definitions and the current checkout's Nix input pins. Uncommitted
application edits are intentionally excluded. Scratch copies contain one root
Cargo workspace/lockfile each; the working checkout's manifests are untouched.

Each case generates Cargo.nix, evaluates all 13 Rust package roots for
`x86_64-linux`, and records their derivation paths and transitive Rust crate
variants (including build dependencies and features). devfinity compares its
unwrapped Rust executable, not its runtime/service wrapper. The report includes
the changed crate identities, not just totals. Generated definitions, Git source
hashes, full per-case graphs, logs, and source snapshots remain in the printed
scratch directory. They are not new repository build authorities.

To compile a small candidate using an already generated scratch snapshot:

```sh
nix-build infra/nixos/crate2nix-prototype/build.nix --no-out-link \
  --argstr source /absolute/evidence/directory/baseline \
  --argstr member finitesites-engine
```

This defaults to the local platform. Add `--argstr system x86_64-linux` on a
Linux builder. Other members are available for experimentation, but binaries
that embed sibling-directory assets still require source packaging work.

## Measured invalidation

Source: `f2a10ffe` (full revision in [results.json](results.json)). Evaluated
Linux derivations from macOS. Counts are identities, **not completed builds or
binary-cache hits**. There are 727 distinct candidate Rust crate derivations
across all 13 roots; different feature sets can yield multiple variants of a crate.

| Change from baseline | Crane dependency bundles invalidated | Candidate crate derivations preserved | Candidate crate derivations changed |
| --- | ---: | ---: | ---: |
| Identical source in a different directory | 0 / 8 | 727 / 727 | 0 |
| Comment in Sites engine source | 0 / 8 | 725 / 727 | 2 |
| Add existing `hex` workspace dependency to Sites engine | 8 / 8 | 725 / 727 | 2 |
| Append a comment to root Cargo.lock | 8 / 8 | 727 / 727 | 0 |

The two changed candidate crates are `finitesites-engine` and `finitesitesd`.
Chat, Brain, Identity, SaaS, Agentd, and CLI roots retain their identities in
these candidate probes. The dependency probe adds `hex`, already present in the
canonical lockfile, to the engine; Cargo updates only the scratch graph. It is
a controlled dependency addition, **not a replay of the reported Sites incident**.
It does not test a shared dependency upgrade or new feature activation.

The lock comment is appended after generation because it has no Cargo semantic
effect; this isolates byte sensitivity. The identical-copy control proves that
temporary directory names alone did not change the measured identities.

Generation/evaluation timings in the JSON describe this warm local run. They
exclude tool download/setup and must not be presented as CI build timings.

### Compilation and feature spot check

The unmodified `finitesites-engine` candidate compiled successfully on
`aarch64-darwin`, including its SQLite dependency. This was a library build,
not a Linux service build or an integration test, and was not timed.

The probe also compares the Linux `finitesitesd` normal/build dependency tree
reported by pinned Cargo with the candidate's features. After restricting
comparison to declared feature names, both have 159 distinct name/version/feature
tuples, with three differences: candidate `hyper-util` additionally enables
`tracing`, `icu_provider` enables `zerotrie`, and `tokio` enables `windows-sys`.
See `feature_spot_check` in the JSON. Raw candidate cfg flags also include
additional implicit dependency/default names; the full graphs retain them.

This is a concrete reason to investigate resolver and target behavior before
migration. It is not proof of a runtime bug: Cargo tree is not a complete
host/target compilation-unit oracle, and generation uses a newer Cargo than
the existing packaging toolchain. Do not silently discard these differences
or treat the successful library build as feature-equivalence proof.

## Why it can help CI

### Draft PR benchmark

The `crate2nix prototype benchmark` workflow runs automatically on draft PRs
touching this prototype. Its two independent Depot jobs build `finitesitesd`
and unrelated `finite-saas-core` using Crane and crate2nix, respectively.
Each measures initial build, identical local rebuild, Sites source edit, and
Sites dependency addition. Results appear in the Actions job summaries and
downloadable timing/diagnostic artifacts. The normal repository CI also runs.

Both jobs allow only the public Nix cache, use eight Nix jobs/eight cores per
job, and record runner resources. Production cached service outputs would
otherwise give Crane an advantage over the unseeded candidate. These are
cache-assisted initial builds and subsequent warm-local-store rebuilds, not
fresh-runner candidate binary-cache tests. Compiler versions match production;
the implementations' default compiler flags and scheduling still differ.
No candidate outputs are pushed to the production binary cache.

Run the same benchmark on a disposable local machine with:

```sh
infra/nixos/crate2nix-prototype/benchmark crate2nix --output /absolute/new/directory
```

Use `crane` for the other implementation. Existing local artifacts can affect
timings; CI's separate jobs are the intended comparison. This is a bounded
two-service packaging experiment, not a full-fleet compatibility gate.

### Scope of the expected benefit

`infra/nixos/packages.nix` includes the root Cargo.lock in every dependency
bundle's dummy source and uses a vendor directory derived from that lockfile.
The Nix service package lane builds eight bundles and then 13 package outputs.
crate2nix gives Nix independent crate outputs, so an unaffected dependency can
retain its cache identity across a change elsewhere in the workspace.

The Rust workspace lane runs Cargo clippy/tests with Swatinem's Cargo cache.
Changing Nix packaging does not automatically accelerate that lane. End-to-end
CI improves only when the Nix package lane or dependent smoke lanes are on the
critical path. Source-only edits already preserve Crane dependency bundles.

Cold builds may get slower: evaluation, many sandbox launches, separate cache
objects, compiler settings, and linking all matter. A change to a widely shared
crate, enabled features, compiler, or native library can still invalidate much
of the graph. Preserved derivations only save work if their outputs are retained
locally or uploaded to a binary cache accessible by the next runner.

## Gates for a production decision

1. Benchmark both implementations on the same Linux runner size and source
   revisions. Record compiler/profile flags, generation, evaluation, substitution,
   compilation, linking, upload time, bytes transferred, and peak disk usage.
2. Compare cold builds, identical builds on a **fresh runner with the cache**,
   source-only edits, the actual incident's dependency change, and a shared
   dependency/feature change. Do not label a second local invocation a fresh
   runner cache test. Do not clear a shared machine's Nix store for this test.
3. Keep current package-specific feature resolution. Verify target and host
   build/proc-macro units, native OpenSSL/SQLite dependencies, Cargo release
   profile, codegen flags, and required embedded files. crate2nix uses
   buildRustCrate rather than Cargo to perform compilation; generation alone is
   insufficient evidence of matching behavior.
4. Preserve Sites CLI examples, Chat CLI Hermes files, Chat's scoped source
   fingerprint, installed binary names, and devfinity's runtime wrapper. The
   stock candidate here has not implemented those packaging contracts.
5. Publish the candidate's intermediate crate outputs to an isolated benchmark
   cache. Existing CI explicitly uploads Cargo artifact roots; that step needs
   an equivalent for per-crate artifacts. Final runtime closures alone are not
   a sufficient plan for preserving compilation artifacts.
6. Run the existing service smoke gates with candidate packages. Before any
   production rollout, trace the affected product contracts and prove the
   existing-state/mixed-version cases required by AGENTS.md. This prototype
   neither changes deployed services nor provides that compatibility proof.

Keep Cargo for development, tests, and clippy during evaluation. Keep one
canonical root Cargo.lock. Decide later whether Cargo.nix is checked in with a
regeneration check or generated in a dedicated CI step; avoid hidden generation
during Nix evaluation when measuring the build pipeline.

## Primary sources

- [Crane maintainer on dependency-granularity tradeoffs](https://github.com/ipetkov/crane/discussions/213)
- [Crane workspace example](https://crane.dev/examples/quick-start-workspace.html)
- [crate2nix implementation and documentation](https://github.com/nix-community/crate2nix)
- [crate2nix restrictions, including sibling source access and tests](https://nix-community.github.io/crate2nix/90_reference/20_known_restrictions/)
- [crate2nix feature selection](https://nix-community.github.io/crate2nix/30_building/20_choosing_features/)

Upstream's current documentation can describe newer behavior than the pinned
0.14.1 binary; this experiment uses that binary's generated Nix code as evidence.
