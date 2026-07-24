# Rustgrep integration proposal and initial findings

Status: proposed, 2026-07-24.

Evidence snapshot:

- `finite-mono` at `7ed110e32715e0a3ba463fa28e2074bd0bf250c7`
  (`origin/main` when this evaluation was refreshed).
- [`rustgrep` 0.1.0][rustgrep-project] at
  `cef69c3ce54c8c5803bddea9ab688c9109ff660f`.
- 36 Cargo workspace members and 205 checked-in Rust source files.

## Decision sought

Add a commit-pinned `rustgrep` package to the root Nix development shell as an
optional source-comprehension tool. Do not make it a CI gate, production
dependency, or replacement for `rg` or rust-analyzer.

The first integration should expose the upstream `rustgrep` command directly in
the shell. A root `just` wrapper or agent-specific guidance can follow only
after normal use proves that another command surface would help.

## Problem

The monorepo's Rust architecture spans 36 crates and several product verticals.
Some of the most important implementation files are intentionally large while
their boundaries stabilize. Exact symbol comprehension therefore often starts
with a textual search that mixes:

- definitions with references and imports;
- real Rust syntax with Rust-looking strings and templates;
- production items with test-local helpers;
- unrelated same-named concepts in different products; and
- contracts with implementation details that are far from the first match.

`rg` is still the right tool for text, references, and quick implementation
discovery. Rust-analyzer is still the right tool for compiler-backed semantic
navigation. The missing middle is a small CLI that prints the original source
for exact Rust definitions, methods, and implementation ranges without first
requiring an editor or a fully indexed workspace.

## What Rustgrep does

Rustgrep parses Rust files with `ast-grep` and selects exact syntactic
constructs for one or more symbol names. An unfiltered query can return:

- type, trait, function, and method definitions;
- inherent and trait implementation ranges selected by their self type;
- explicit import renames;
- attached documentation and attributes; and
- conservative type-alias expansion when `--follow-aliases` is requested.

It preserves the original source rather than reconstructing or formatting it.
Traversal follows normal ignore rules and searches only Rust files unless an
explicit Rust file is supplied.

It is deliberately syntax-aware rather than compiler-semantic. It does not
resolve general imports, expand macros, infer types, solve traits, return call
sites, or independently select tests and references.

## Proposed integration

### Packaging

Package Rustgrep with the root repository's pinned Nix toolchain instead of
importing the upstream flake wholesale:

1. Add `nix/dev-tools/rustgrep.nix`.
2. Fetch the upstream Git repository at the exact evaluated commit and verify
   the source hash.
3. Build it with `pkgs.rustPlatform.buildRustPackage`, using the upstream
   `Cargo.lock`.
4. Add the resulting package to `devShells.default.packages` in `flake.nix`.

This matches the monorepo rule that dependencies and toolchains are supplied by
Nix. It also avoids installing Cargo binaries into a user-global prefix.

The evaluation used the upstream flake to avoid changing the repository. On a
cold machine that path requested 157 derivations, approximately 316 MiB of
downloads, and 1.47 GiB unpacked, largely because it brought its own Rust/Nix
toolchain closure. Packaging the small Rust binary with the root toolchain
should avoid that parallel toolchain cost.

Do not vendor the Rustgrep source into this repository. The source revision,
source hash, Cargo lockfile, and build output should be sufficient to make the
dependency reviewable and reproducible. If the Radicle Git endpoint proves too
unreliable for routine shell realization, stop and resolve source availability
explicitly rather than adding an unpinned fallback.

### Command surface

After the package is in the shell, the normal interface should be:

```console
rustgrep AppState
rustgrep RuntimeSpecV1 RuntimeControlLease
rustgrep recover_known_good_chat_runtime --only method
rustgrep KataLauncher::recover_known_good_chat_runtime
rustgrep RuntimeSpecV1 --path finitecomputer-v2/crates/finite-saas-core/src/lib.rs
```

Related symbols should be batched into one query. In the evaluation, a warm
default-parallel full-repository query took 0.22-0.25 seconds. The same query
scoped to one known source file took 0.05 seconds. `--threads 1` took 1.05
seconds and is useful for debugging worker behavior, not as the normal default.

Do not add Rustgrep to `just check`, CI, release builds, runtime images, or
production NixOS closures. Its output is an investigative aid, not a correctness
proof.

### Recommended search workflow

Use the tools together:

1. Use `rg` to discover an unknown name, reference, or trait implementation.
2. Use Rustgrep to read the exact definition or method body without surrounding
   reference noise.
3. Use an owner-qualified query after discovering a concrete self type.
4. Use rust-analyzer or Cargo when semantic identity or compilation behavior
   matters.
5. Re-open source context whenever a result could be test-local or nested under
   a conditional module.

For example, Rustgrep does not enumerate the concrete implementors when asked
for `RuntimeLauncher`. The reliable sequence is:

```console
rg 'impl RuntimeLauncher for' finitecomputer-v2/crates/finite-saas-runner/src
rustgrep KataLauncher::runtime_capabilities
rustgrep PhalaLauncher::runtime_capabilities
```

## Acceptance criteria for the integration PR

The follow-up implementation is acceptable when:

- `scripts/with-dev-env rustgrep --version` reports the pinned version on each
  supported development system;
- `scripts/with-dev-env rustgrep --list-categories` succeeds;
- a full-repository query and an explicit `--path` query both succeed;
- the source and dependency inputs are locked and hash-verified;
- the package reuses the root development toolchain rather than importing a
  second Nixpkgs/Rust environment;
- no production package, image, NixOS configuration, or CI gate depends on the
  tool;
- no user-global Cargo or system installation is required; and
- removing the package expression and its dev-shell entry completely rolls the
  integration back.

The implementation PR should record realized closure size on macOS and Linux.
If adding the tool makes the ordinary development shell materially expensive
or unavailable on a supported system, prefer a separate opt-in development
shell over weakening reproducibility.

## Initial findings

The findings below demonstrate the kinds of questions Rustgrep answered
quickly. They are orientation evidence, not architectural decisions by
themselves.

### Core owns desired Runtime state; Runner owns provider execution

`rustgrep RuntimeSpecV1 RuntimeControlLease` isolates the immutable,
provider-neutral launch and lifecycle inputs in
[`finite-saas-core`][runtime-spec]. `rustgrep RuntimeLauncher` isolates the
provider port in [`finite-saas-runner`][runtime-launcher].

A textual follow-up finds concrete implementations for Docker, Kata, Phala,
Enclavia, Apple Container, boxed launchers, and a test fake. The shape confirms
an intentional authority boundary:

- Core chooses the project, placement, artifact, image digest, state schema,
  durable state identity, endpoint contract, boot intent, public environment,
  and secret references.
- Runner adapters execute that complete contract for a provider.
- A provider adapter is not supposed to choose a different durable state or
  product release while processing the operation.

This is a stronger structural signal than the directory split alone.

### Advertised capabilities are the lifecycle truth

`rustgrep recover_known_good_chat_runtime --only method` finds the trait
default and the backend-specific method bodies without returning ordinary call
sites. Owner-qualified queries then show:

- [`KataLauncher`][kata-recovery] has a 336-line recovery workflow.
- Docker, Enclavia, and Phala each have a seven-line override delegating to
  their restart behavior.
- Apple Container has no override and inherits the fail-closed trait default.

The capability constructors explain why method presence is not product
support. [`state_preserving_runtime_capabilities`][runtime-capabilities]
advertises restart but not known-good Chat recovery. Kata's capability
constructor advertises recovery only for the adapter with the substantive
recovery implementation. Artifact bounds can narrow that advertised support
again.

The important discovery is not that multiple methods share a name. It is that
capability advertisement, Core-bound artifacts, and the provider implementation
form one support contract.

### Finite Chat has a distinct delivery port

`rustgrep RuntimeDelivery` isolates a narrow client-side protocol in
[`finitechat-client`][runtime-delivery] for key-package inventory, key-package
claims, commit submission, Room listing, Welcome handling, and event sync.

The production implementation is the generic HTTP delivery adapter; a
test-local implementation exercises pending-commit behavior. This makes the
transport seam visible without reading the much larger client module from the
top.

### Same names do not imply shared concepts

A textual `AppState` search returned 142 lines across 16 files. Rustgrep
returned four actual definitions:

- the Devfinity WorkOS fixture;
- the Finite Sites server state;
- the Finite Chat product projection; and
- the Finite Private limiter state.

The apparent fifth definition in
[`finitechat-rmp/src/init.rs`][appstate-template] is Rust source embedded in a
generated-project string. Rustgrep correctly excludes it from the structural
result.

Similarly, `rustgrep BlobStore` reveals that the name denotes a concrete
filesystem content-addressed store in
[`finitesites-blob`][sites-blob-store], but a storage trait in
[`finitechat-blob`][chat-blob-store]. The collision is navigational, not proof
that the two products share a storage abstraction.

### Shared primitives coexist with product-local adapters

`rustgrep IdentityAuthorityClient` returns two private client surfaces, one in
the Brain CLI and one in the Sites CLI. A method-only search for
`send_signed_json` shows nearly identical signed HTTP transport code in both:

- [`finite-brain-cli`][brain-identity-client]
- [`fsite-cli`][sites-identity-client]

`rustgrep Mailer`, `rustgrep DevMailer`, and `rustgrep HttpMailer` likewise
show separate email interfaces and transports in Finite Identity and Finite
Sites.

This is evidence of repeated mechanics worth tracking, but it is not permission
to create a universal identity adapter. [ADR 0004][bounded-identity-adapters]
requires each product to own its bounded identity-provider contract and policy.
Any future deduplication must be limited to policy-free mechanics such as an
HTTP request helper or provider transport, with product intents, validation,
authorization, and response contracts remaining product-owned.

## Limitations observed

### Trait-name search does not list implementors

Searching for `RuntimeLauncher` returns the trait definition and its attached
source, but not every `impl RuntimeLauncher for ...`. Rustgrep selects trait
implementation ranges by the implementation's self type. Use `rg` to discover
implementors, then query `KataLauncher` or an owner-qualified method.

### Test locality can be invisible

`rustgrep kata_runtime_capabilities` returns three same-named helpers. All
three are test helpers, including two nested in large `#[cfg(test)]` modules.
The selected function source does not include the enclosing module's
conditional attribute, so an isolated result can look production-relevant.

Always inspect enclosing source context when test/production identity matters.
Path scoping alone does not remove inline test modules from a production source
file.

### It is not a reference or call search

Rustgrep intentionally omits ordinary references, plain imports, calls, tests
as a category, associated constants, macro output, type inference, and general
module resolution. Those omissions are why its output is focused, but they also
make it unsuitable for impact analysis by itself.

### Generic full-repository queries still mix product verticals

Exact syntax eliminates reference noise, not legitimate same-named
definitions. Names such as `AppState`, `Mailer`, `BlobStore`, `StoreError`, and
`CliError` still require path or owner context.

## Evaluation and rollout

Adopt in two stages:

1. **Tool availability:** land the pinned Nix package and dev-shell entry with
   the acceptance checks above.
2. **Workflow adoption:** use it during several real architecture,
   debugging, and review tasks. Record corrections when Rustgrep output alone
   would have produced a wrong conclusion.

After that trial, keep the integration only if it:

- routinely reduces source-navigation noise;
- remains cheap enough for the default shell;
- works on supported macOS and Linux development systems; and
- is consistently described as syntax-aware evidence rather than semantic
  proof.

Only then consider adding one sentence to `AGENTS.md` recommending Rustgrep for
exact Rust symbol source. Do not mandate it for ordinary textual searches, and
do not make agent instructions depend on a tool before the Nix integration is
available everywhere.

## Non-goals

- Replacing `rg`, rust-analyzer, Cargo, Clippy, or compiler diagnostics.
- Generating an architecture graph or dependency graph.
- Turning the initial findings into automatic refactors.
- Centralizing product identity policy contrary to ADR 0004.
- Adding a new production, release, or CI dependency.
- Sending repository source to an external service. Rustgrep runs locally and
  performs no network operation during a query.

[appstate-template]: ../finitechat/crates/finitechat-rmp/src/init.rs#L786
[bounded-identity-adapters]: adr/0004-products-own-bounded-identity-adapters.md
[brain-identity-client]: ../finite-brain/crates/finite-brain-cli/src/identity_authority.rs#L8
[chat-blob-store]: ../finitechat/crates/finitechat-blob/src/lib.rs#L520
[kata-recovery]: ../finitecomputer-v2/crates/finite-saas-runner/src/kata.rs#L1761
[runtime-capabilities]: ../finitecomputer-v2/crates/finite-saas-runner/src/lib.rs#L68
[runtime-delivery]: ../finitechat/crates/finitechat-client/src/lib.rs#L3911
[runtime-launcher]: ../finitecomputer-v2/crates/finite-saas-runner/src/lib.rs#L1213
[runtime-spec]: ../finitecomputer-v2/crates/finite-saas-core/src/lib.rs#L199
[rustgrep-project]: https://radicle.network/nodes/radicle.dpc.pw/rad%3Az3wPTYCEHukxHQNU2fQ2b3eASNw8a
[sites-blob-store]: ../finite-sites/crates/finitesites-blob/src/lib.rs#L36
[sites-identity-client]: ../finite-sites/crates/fsite-cli/src/api.rs#L37
