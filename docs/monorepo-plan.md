# Finite Monorepo Plan

This is the working construction plan for moving Finite from several repos into
one monorepo named `finite-mono`. It intentionally starts simple: copy the first
three repos into one repo, preserve their current local loops, and only then
consolidate shared structure.

This plan is based on the Fedimint monorepo analysis in
[`fedimint-monorepo-structure-analysis.md`](fedimint-monorepo-structure-analysis.md).
Before implementing any specific monorepo component, check the corresponding
Fedimint pattern as a reference. Do not copy it blindly, but use it to calibrate
the shape of the Rust workspace, Nix boundaries, `just` commands, scripts,
docs, integration harness, CI, and quality gates before deciding how Finite
should implement that piece.

The first migration should not try to perfect the final structure. The initial
shape should keep each imported repo mostly intact under its own top-level
folder. That makes the move easier to reason about and avoids rewriting every
path before we know the combined repo builds.

The initial scope is:

- `finitecomputer-v2`
- `finitechat`
- `finite-sites`

Everything else is deferred until those three are working cleanly in monorepo
form.

## Decisions

- [x] Destination repository is `finite-mono`.
- [x] Do not preserve git history.
- [x] Do not use `git subtree`, `git filter-repo`, or other history-preserving
      import commands.
- [x] Copy files directly from the source repos.
- [x] Record source commit SHAs in the migration note.
- [x] Do not modify, archive, or delete the existing repos during migration.
- [x] Do not spend time on rollback plans.
- [x] Keep `finitecomputer-v2`, `finitechat`, and `finite-sites` as top-level
      folders at first.
- [x] Do not create root-level service or app folders during the initial
      migration.
- [x] Do not create a separate integration area during the initial migration.
      Integration ownership can stay with the finitecomputer domain for now.
- [x] Keep the root `justfile` simple. Do not adopt Flakebox just generation.
- [x] Allow a root `scripts/` directory, but keep it shallow until there is real
      pressure for more structure.
- [x] Treat the local integration harness as a separate phase after basic
      monorepo import.
- [x] Treat CI as a separate phase after local commands work.
- [x] Treat quality gates as a separate phase after the first import stabilizes.
- [x] Do not build a Nix system for non-Rust artifacts at the start.
- [x] Repurpose `finite-eng-docs` into the monorepo's normal `docs/` role, but
      do not blindly copy stale docs as authoritative truth.
- [x] Defer broad stale-doc cleanup until after the first imported repos are
      working from the monorepo.

## Target Shape

The initial `finite-mono` root should be boring and easy to import into:

```text
finite-mono/
  README.md
  AGENTS.md
  justfile
  Cargo.toml
  flake.nix
  flake.lock
  scripts/
  docs/
  finitecomputer-v2/
  finitechat/
  finite-sites/
```

The source repo folders should initially retain their existing internal
structure. Do not split dashboards, mobile code, deployment files, integration
code, or service wiring into new root folders during the first copy.

Root `Cargo.toml`, `flake.nix`, and `flake.lock` exist from the skeleton phase.
The Cargo workspace starts empty and gets populated only after the source repos
are copied.

The important early boundary is not final folder taxonomy. It is that all three
repos can live in one checkout, keep their current commands working, and then
join one root Cargo workspace.

## Phase 0: Preparation

Goal: pin the source inputs and create a minimal migration record before files
move.

- [x] Set destination repository name to `finite-mono`.
- [x] Use direct file copying as the import method.
- [x] Do not preserve git history.
- [x] Leave all existing source repos untouched.
- [x] Skip rollback-plan work.
- [x] Create `finite-mono`.
- [x] Record the source commit SHA for `finitecomputer-v2`.
- [x] Record the source commit SHA for `finitechat`.
- [x] Record the source commit SHA for `finite-sites`.
- [x] Add `docs/monorepo-migration-log.md`.
- [x] Write the source repo paths and source commit SHAs into the migration
      log.
- [x] Note that the import is a snapshot copy, not a history-preserving import.

Exit criterion: `finite-mono` exists and the source commit SHAs are recorded in
the migration log.

## Phase 1: Monorepo Skeleton

Goal: create only the root files needed to orient the copied repos.

- [x] Create root `README.md` explaining that this is the Finite monorepo.
- [x] Create root `AGENTS.md` with high-level repo navigation and editing
      rules.
- [x] Create root `docs/`.
- [x] Create root `scripts/`.
- [x] Create a minimal root `justfile`.
- [x] Create root `Cargo.toml` for the initial Rust workspace.
- [x] Create root `flake.nix` to pin the Rust development shell.
- [x] Generate root `flake.lock`.
- [x] Add this plan at `docs/monorepo-plan.md`.
- [x] Add the Fedimint analysis at
      `docs/fedimint-monorepo-structure-analysis.md`.

Initial root `justfile` should stay small:

```just
default:
    just --list
```

Do not add root Cargo commands to the `justfile` until source repo crates are
copied and added as workspace members.

Exit criterion: the empty `finite-mono` skeleton exists and clearly says that
source repos will be copied intact first.

## Phase 2: Copy `finitecomputer-v2`

Goal: copy `finitecomputer-v2` into `finite-mono/finitecomputer-v2` without
changing behavior.

Tasks:

- [x] Copy `finitecomputer-v2` into `finite-mono/finitecomputer-v2`.
- [x] Exclude generated or machine-local directories such as `target/`,
      `node_modules/`, `.next/`, and `.local-state/`.
- [x] Keep the repo's internal folder structure intact.
- [x] Run the old Rust check from `finite-mono/finitecomputer-v2`.
- [x] Run the old dashboard install/test command from the copied
      `finitecomputer-v2` tree.
- [x] Record copied commit SHA, ignored local files, validation commands, and
      failures in `docs/monorepo-migration-log.md`.

Exit criterion: `finitecomputer-v2` works from its copied folder using the same
commands it used before migration.

## Phase 3: Copy `finitechat`

Goal: copy `finitechat` into `finite-mono/finitechat` without changing
behavior.

Tasks:

- [x] Copy `finitechat` into `finite-mono/finitechat`.
- [x] Exclude generated or machine-local directories such as `target/`,
      `.finitechat/`, `.state/`, iOS build products, and script caches.
- [x] Keep the repo's internal folder structure intact.
- [x] Run the old Rust checks from `finite-mono/finitechat`.
- [x] Run the old Python smoke tests from `finite-mono/finitechat` if they are
      still expected to pass locally.
- [x] Run the minimum iOS or bindgen smoke that currently exists, if it is
      practical locally.
- [x] Record copied commit SHA, ignored local files, validation commands, and
      failures in `docs/monorepo-migration-log.md`.

Exit criterion: `finitechat` works from its copied folder using the same
commands it used before migration, or any non-portable local checks are
explicitly documented.

## Phase 4: Copy `finite-sites`

Goal: copy `finite-sites` into `finite-mono/finite-sites` without changing
behavior.

Tasks:

- [x] Copy `finite-sites` into `finite-mono/finite-sites`.
- [x] Exclude generated or machine-local directories such as `target/` and
      `.dev-data/`.
- [x] Keep the repo's internal folder structure intact.
- [x] Run the old Rust checks from `finite-mono/finite-sites`.
- [x] Run the old local dev smoke if it is quick and deterministic.
- [x] Record copied commit SHA, ignored local files, validation commands, and
      failures in `docs/monorepo-migration-log.md`.

Exit criterion: `finite-sites` works from its copied folder using the same
commands it used before migration.

## Phase 5: Populate Root Cargo Workspace

Goal: populate the root Cargo workspace with the copied repos' existing crate
paths and keep Rust dependency locking at the monorepo root.

Do this only after the copied repos work independently. Do not flatten crates
or move app/service folders in this phase.

Tasks:

- [x] Check Fedimint's root `Cargo.toml` and workspace dependency pattern
      before writing the Finite root workspace.
- [x] Confirm root `Cargo.toml` still has one `[workspace]`.
- [x] Add `finitecomputer-v2/crates/*` members explicitly.
- [x] Add `finitechat/crates/*` members explicitly.
- [x] Add `finitechat/uniffi-bindgen` if it should remain a workspace member.
- [x] Add `finite-sites/crates/*` members explicitly.
- [x] Decide whether to use a root `[workspace.package]` immediately or later.
- [x] Decide whether to merge `[workspace.dependencies]` immediately or keep
      dependency declarations in member crates temporarily.
- [x] Generate one root `Cargo.lock`.
- [x] Verify the root lockfile before removing nested `Cargo.lock` files.
- [x] Fix path dependencies that break from the new root workspace.
- [x] Resolve duplicate crate names or binary names.
- [x] Resolve dependency version conflicts only when required for the build.
- [x] Run `cargo check --workspace` from `finite-mono`.
- [x] Run `cargo test --workspace` from `finite-mono`.
- [x] Remove nested `Cargo.lock` files only after the root lockfile is working
      and copied repo Cargo commands resolve through the root workspace.

Exit criterion: root `cargo check --workspace` and `cargo test --workspace`
cover all imported Rust crates that are expected to pass locally.

## Phase 6: Minimal Root Commands

Goal: provide a small command surface after the root Cargo workspace exists.

Keep this intentionally smaller than Fedimint's command set.

Tasks:

- [x] Check Fedimint's split between generated root `justfile` and custom
      project justfile before finalizing the Finite root `justfile`.
- [x] Keep `just default` as a discoverable command list.
- [x] Add `just metadata` for root Cargo workspace metadata checks.
- [x] Add `just check` for root Rust workspace checks.
- [x] Add `just test` for root Rust workspace tests.
- [x] Use `just` modules for repo-local command surfaces that should stay
      nested.
- [x] Add `just sites ...` as a module backed by `finite-sites/justfile`.
- [x] Add `just fmt` for Rust formatting only if it is already low-friction.
- [x] Avoid adding dashboard, chat, sites, CI, release, and deploy commands as
      root recipes in this phase unless they are repeatedly needed.
- [x] Move any multi-line logic into root `scripts/` instead of growing the
      `justfile`.

Exit criterion: root commands are discoverable without becoming a second build
system.

## Phase 7: Docs

Goal: turn `finite-eng-docs` into the monorepo's normal root docs without
pretending stale imported docs are current.

Tasks:

- [x] Check Fedimint's `docs/` structure before finalizing root docs
      navigation.
- [x] Move this plan to `docs/monorepo-plan.md`.
- [x] Move the Fedimint analysis to
      `docs/fedimint-monorepo-structure-analysis.md`.
- [x] Move useful `finite-eng-docs` orientation content into root docs, updating
      language from "cross-repo" to "monorepo".
- [x] Keep copied repo docs inside their source repo folders at first.
- [x] Add a root `docs/README.md` that distinguishes current root docs from
      imported repo-local docs.
- [x] Mark stale or unreviewed docs clearly before linking them as canonical.
- [x] Maintain `docs/monorepo-migration-log.md` until migration is done.

Exit criterion: root docs tell readers where to start and which copied docs are
not yet reviewed.

## Phase 8: Normalize Source Repo Folders

Goal: improve navigation while still avoiding a major folder taxonomy rewrite.

Tasks:

- [ ] Add or update `finitecomputer-v2/README.md` for monorepo-local commands.
- [ ] Add or update `finitechat/README.md` for monorepo-local commands.
- [ ] Add or update `finite-sites/README.md` for monorepo-local commands.
- [ ] Update root `README.md` to link to each source repo folder.
- [ ] Remove only duplicated docs or scripts that are actively confusing.
- [ ] Keep dashboard, iOS, deploy files, and integration code inside their
      copied source repo folders unless there is a concrete reason to move them.

Exit criterion: a new engineer can start from the root README and find the
right copied repo folder and commands.

## Phase 9: Local Integration Harness

Goal: build a Finite equivalent of Fedimint's `devimint`, named `devfinity`,
but only after the monorepo can build the pieces independently. Before working
on this harness, use Fedimint's `devimint` crate and `scripts/dev/mprocs`
workflow as the reference point: copy the durable contract, not the product
specific complexity.

`devfinity` should be the Finite-aware launcher. It should own state layout,
generated environment variables, process lifecycle, logs, readiness, and the
test fixture stack handle.

The durable target is a devimint-style Rust harness: one library-owned stack
object starts child processes, exposes generated environment and typed clients,
waits for readiness, runs wrapped commands/tests, and tears the stack down. A
local visual UI should be a small mprocs-style log viewer over devfinity-owned
logs, not the process orchestrator.

Devfinity must not treat the monorepo source checkout as the runtime boundary
for Rust services. Core, Chat, and Sites should be added to
`devfinity/Cargo.toml` as dependencies when their components start the service
through library APIs. If a Rust service lacks a usable library entrypoint, add
one to that service crate first.

Scope devfinity to backend infrastructure and Rust-controllable services. For
non-Rust services such as the dashboard, use an outer dev process composer: it
starts or wraps devfinity for backend infra, waits for devfinity readiness and
env, then runs dashboard/UI `just` commands. Dashboard source paths, npm
install logic, and frontend dev-server details should not live inside the
devfinity crate.

The initial prototype used process-compose as the local runtime. That runtime
has been retired from the compiled harness. Do not grow the old generated
process-compose config or add a generic runtime abstraction unless a real
second backend appears.

Initial backend harness scope should be narrow:

- Start the minimum local backend/control-plane path.
- Start or connect to a local Finite Chat server.
- Start or connect to Finite Sites.
- Create deterministic local state.
- Print the backend URLs and credentials needed for manual smoke testing.

Initial structure:

- Add a top-level `devfinity/` Rust workspace crate.
- Keep generated local state under `.local-state/devfinity/` so the committed
  repo does not hard-code machine-local state paths.
- Add `docs/local-integration-harness.md` for operator/developer usage.
- Add only thin `just dev` module wrappers around `devfinity`.

Target runtime shape:

- `devfinity up`: create state, write env, start managed child processes, wait
  for readiness, and stream or expose logs for local development.
- `devfinity up --headless`: start the same stack without an interactive log
  viewer.
- `devfinity up --headless -- <command>`: start the stack, wait for readiness,
  run the command with generated env, then tear the stack down.
- `devfinity cleanup`: best-effort recovery for stale devfinity-owned state and
  any processes that can be proven to belong to the active run.
- `devfinity status`: read-only status for configured service endpoints and any
  running devfinity-owned stack.

Initial devfinity-managed services:

- Native local Postgres for `finite-saas-core`, using the Postgres binaries from
  the Nix development shell.
- `finite-saas-core` on a deterministic local port.
- Local `finitechat-server` backed by SQLite state.
- Local `finitesitesd` with its data dir under `.local-state/devfinity/`.

Dashboard and other UI/dev-server processes belong to an outer composer that
consumes devfinity-generated env and readiness markers. For local ergonomics,
add that mprocs-style viewer/composer only after lifecycle, generated env,
fixtures, and base backend components are library-owned. The viewer should tail
devfinity and UI logs and provide a developer shell; it should not duplicate
backend service startup order or backend teardown.

Tasks:

- [x] Check Fedimint's `devimint` crate, `just mprocs`, and test harness
      structure before designing the Finite harness.
- [x] Decide whether the harness should be Rust, shell, or a small mixed
      wrapper: `devfinity` is a Rust harness.
- [x] Replace the initial process-compose prototype with devimint-style
      Rust-owned process orchestration.
- [x] Add devimint-style generated vars/env globals for paths, ports, logs, and
      shell exports.
- [x] Add unique fixture run directories and non-conflicting fixture port
      allocation while preserving deterministic local defaults.
- [x] Add devimint-style `run_devfinity_test` so Rust integration tests can
      start and tear down stacks through the library with `DevfinityStack` as
      the test handle.
- [x] Remove Dashboard from devfinity-managed ports, paths, env, readiness, and
      process ownership.
- [x] Move dashboard startup and create-agent e2e smoke out of devfinity.
- [ ] Add dashboard startup and create-agent e2e smoke to an outer composer
      that consumes devfinity env/readiness and runs dashboard just/script
      commands.
- [ ] Audit Core, Chat, and Sites server library surfaces and add small public
      serve entrypoints where they are missing.
- [ ] Add a small devfinity task manager for in-process Rust backend services,
      keeping native Postgres as an external infrastructure process.
- [ ] Add Core, Chat, and Sites crates to `devfinity/Cargo.toml` as their
      components start using library server entrypoints.
- [ ] Migrate Core, Chat, and Sites one at a time away from `target/debug/...`,
      keeping backend smokes passing after each migration.
- [ ] Remove startup `cargo build`, service `target/debug/...` paths, and the
      repo-root fixture workaround only after no backend service needs the
      source checkout as a runtime boundary.
- [ ] Add small process/task shutdown hardening so killed Postgres children and
      stopped Rust service tasks are fully cleaned up before replacement runs.
- [ ] Move base service specs out of `stack.rs` into typed components for
      Postgres, Core, Chat, and Sites.
- [ ] Add a small outer mprocs-style local composer after env, fixtures, and
      base backend components are library-owned.
- [x] Define the first backend smoke scenario: readiness probes cover Core
      `/healthz`, Finite Chat `/health`, and Finite Sites `/api/v1/healthz`.
- [ ] Define the outer dashboard smoke scenario: start dashboard after
      devfinity readiness, submit the create-agent form, and verify Core state.
- [x] Decide where the harness should live in `finite-mono`: a top-level
      `devfinity/` workspace crate.
- [x] Add `process-compose` to the Nix development shell for the initial
      prototype.
- [x] Remove `process-compose` from the Nix development shell after replacing
      the prototype runtime.
- [x] Add Postgres to the Nix development shell and run local Postgres natively.
- [x] Add `devfinity up`, `up --headless`, and `cleanup`.
- [x] Generate process-compose YAML into `.local-state/devfinity/` for the
      initial prototype.
- [x] Add local state layout under `.local-state/` or another ignored root.
- [x] Add a minimal `just dev up` command only after the harness exists.
- [x] Add `just dev up --headless` and `dev cleanup` wrappers.
- [x] Add `just dev status` for read-only local stack status.
- [x] Add `devfinity up --headless -- <command>` for devimint-style wrapped
      integration commands.
- [x] Add `just dev smoke` using the wrapped-command path against real local
      Core, Finite Chat, Finite Sites, and Postgres infrastructure.
- [x] Add an ignored Rust backend integration smoke test that is run through
      `just dev rust-smoke`.
- [x] Add log collection for failed local runs.
- [x] Document the harness in `docs/local-integration-harness.md`.

Later:

- [x] Graduate devfinity Rust smokes into typed fixture tests once
      `run_devfinity_test` exists.

Exit criterion: one command can start the first useful local Finite stack smoke
without requiring the old standalone repo layout.

## Phase 10: CI

Goal: add CI only after local commands and workspace shape are stable.

Start with low ambition:

- [ ] Check Fedimint's CI/Nix relationship before designing Finite CI.
- [ ] Check out `finite-mono`.
- [ ] Run `cargo check --workspace`.
- [ ] Run `cargo test --workspace`.
- [ ] Run dashboard install/test if dashboard has stable tests.
- [ ] Upload basic logs on failure.

Defer:

- [ ] Release packaging.
- [ ] Container builds.
- [ ] Cross-compilation.
- [ ] Nix-built frontend artifacts.
- [ ] Large compatibility matrices.
- [ ] Full integration harness CI.

Exit criterion: CI catches basic breakage without becoming a second build
system.

## Phase 11: Quality Gates

Goal: add repo-wide quality gates once the first migration is not moving files
every day.

Tasks:

- [ ] Check Fedimint's formatting, Clippy, typos, and pre-commit setup before
      choosing Finite gates.
- [ ] Choose root Rust formatting policy.
- [ ] Choose root Clippy strictness.
- [ ] Decide whether `cargo fmt --all -- --check` is required locally, in CI, or
      both.
- [ ] Decide whether Clippy warnings are denied immediately or phased in.
- [ ] Decide whether TypeScript linting is enforced at root.
- [ ] Decide whether Python checks are enforced at root.
- [ ] Decide whether docs link checks are worth adding.
- [ ] Add only the gates that have owners and low false-positive risk.

Exit criterion: quality gates improve confidence without blocking unrelated
migration work on stale style debt.

## Phase 12: Nix Expansion

Goal: add Nix deliberately, starting with Rust/dev-shell needs and expanding
only when useful.

Initial Nix scope:

- [ ] Check Fedimint's `flake.nix`, dev shell, and package outputs before
      designing the Finite flake.
- [ ] Root dev shell for Rust toolchain and common native dependencies.
- [ ] Optional Cachix/substituter policy if build times justify it.
- [ ] Rust package builds for key binaries.

Deferred Nix scope:

- [ ] Dashboard/frontend builds.
- [ ] iOS/mobile support.
- [ ] Container image builds.
- [ ] Release artifacts.
- [ ] Cross-platform package matrices.

Exit criterion: Nix helps local development or release confidence without
becoming mandatory for every non-Rust path too early.

## Phase 13: Stale Docs Audit and Purge

Goal: after the first monorepo milestone is working, comb through all imported
folders and remove or update docs that are stale, misleading, or tied to the
old multi-repo layout.

Do not start this phase until the copied repos build and test from the monorepo.
Expect that many imported docs may be deleted. Until then, stale docs are
acceptable as copied historical context, but they should not be linked as
canonical guidance without review.

Tasks:

- [ ] Confirm the first monorepo milestone is working before changing or
      deleting broad documentation.
- [ ] Check Fedimint's docs layout and ownership boundaries before deciding
      which Finite docs should become canonical root docs.
- [ ] Inventory docs across root `docs/`, copied repo `docs/` folders, root and
      repo-local READMEs, app READMEs, deployment READMEs, and crate-local docs.
- [ ] Classify each doc as keep current, update, delete, or leave repo-local
      but marked unreviewed.
- [ ] Prefer a small set of current root docs over many stale copied docs.
- [ ] Update canonical docs to use monorepo paths and root commands.
- [ ] Remove docs that describe obsolete repos, commands, deployment paths, or
      architecture only after checking for current links and operational use.
- [ ] Preserve operational runbooks only when they still map to live systems or
      clearly mark them as needing review.
- [ ] Search for broken links and stale references after removals.
- [ ] Record major doc removals or retitles in
      `docs/monorepo-migration-log.md`.

Exit criterion: root docs are intentionally small and current, stale copied docs
are removed or clearly marked, and no current docs link readers into known-bad
instructions.

## Later Repo Imports

After the first three repos are stable, import remaining repos one at a time.
Do not batch these until the pattern is proven.

Candidate order:

- [x] `finite-identity`
- [x] `finite-nostr`
- [x] `finite-auth`
- [x] `finite-brain`
- [x] `finite-search`
- [x] `finite-skills`
- [ ] `reporting`
- [ ] legacy `finitecomputer`

For each later import:

- [ ] Record source commit SHA.
- [ ] Copy files into a top-level source repo folder.
- [ ] Add Rust crates to root workspace if applicable.
- [ ] Preserve the old local validation loop first.
- [ ] Normalize paths and docs only after the copy works.

Completed for `finite-identity`:

- [x] Record source commit SHA.
- [x] Copy files into a top-level source repo folder.
- [x] Add Rust crate to root workspace.
- [x] Replace existing pinned git dependency with the local workspace package.
- [x] Preserve and run the old local validation loop.

Completed for `finite-nostr`, `finite-auth`, `finite-brain`, `finite-search`,
and `finite-skills`:

- [x] Record source commit SHAs.
- [x] Copy files into top-level source repo folders.
- [x] Add Rust crates to root workspace where applicable.
- [x] Replace copied nested Rust workspaces with root workspace membership where
      applicable.
- [x] Add root `just` modules for repos with useful local command surfaces.
- [x] Preserve and run the old local validation loops.

## First Milestone Definition

The first successful monorepo milestone is intentionally modest:

- [x] `finitecomputer-v2`, `finitechat`, and `finite-sites` are present in
      `finite-mono` as top-level copied folders.
- [x] Source commit SHAs are recorded.
- [x] Existing repos are untouched.
- [x] Root `docs/README.md` exists.
- [x] Root `docs/monorepo-migration-log.md` exists.
- [x] Rust crates from all three copied repos are members of one root Cargo
      workspace.
- [x] One root `Cargo.lock` exists and is verified.
- [x] The dashboard still installs and runs from the copied
      `finitecomputer-v2` tree.
- [x] Finite Chat core checks still run from `finitechat`.
- [x] Finite Sites checks still run from `finite-sites`.
- [x] Root `just check` works.
- [x] Root `just test` works, or documented exclusions exist for tests that
      need services not yet covered by the harness.
- [x] Docs have a current root starting point and copied stale docs are labeled
      where linked.
