# Finite Mono Level-Up Plan

This document is a prioritized backlog for improving the maturity,
testability, operability, and maintainability of `finite-mono`.

The highest-leverage theme is to eliminate false confidence and reduce the
number of implementations that appear authoritative. The repository already
has sophisticated tests, compatibility fixtures, and thoughtful postmortems;
the main gaps are that important guarantees are not consistently enforced by
the normal developer and CI paths.

## Audit snapshot

- 36 Rust workspace packages.
- Approximately 269,000 lines of Rust.
- `just check` passed during the audit.
- One full `just test` run failed three Runner tests; an immediate isolated
  package rerun passed all 135 tests.
- Some Core Postgres tests report success without executing when
  `FC_CORE_POSTGRES_TEST_URL` is absent.
- No source files were changed during the audit.

## Priorities

| Priority | Candidate | Leverage | First useful slice |
| --- | --- | --- | --- |
| P0 | Make the root test gate truthful and deterministic | Very high | 1–3 days |
| P0 | Ship the Hosted Device machine-readable error contract | Very high | 1–3 days |
| P0 | Eliminate Core's shadow in-memory business implementation | Very high | 1–2 weeks |
| P0 | Trace and retire the legacy `finite-core`/archived-Git surface | Very high | 3–10 days |
| P1 | Replace concatenated SQL migrations with a migration ledger | High | 3–5 days |
| P1 | Turn compatibility fixtures into an executable release contract | High | 3–7 days |
| P1 | Add a normal-CI full product-spine test | High | 1–2 weeks |
| P1 | Monitor product readiness, not just process health | High | 2–5 days |
| P1 | Make toolchain and lint policy real | Medium-high | 1–3 days |
| P2 | Establish a supply-chain baseline | Medium-high | 1–3 days |
| P2 | Generate and validate cross-language wire contracts | Medium-high | 3–7 days |
| P2 | Decompose mega-modules along ownership boundaries | Medium | Incremental |
| P2 | Add causal review and documentation ownership | Medium | 1–3 days |

## P0 — Immediate confidence and simplification

### 1. [x] Make `just test` truthful and deterministic

Completed as a hard cut on 2026-07-29. The root [`just test`](justfile) command
now runs `cargo test --workspace --locked` through `devfinity run`, and CI calls
that same recipe. Devfinity always starts only an isolated Nix-provided
Postgres 16 instance for the command; there is no optional infrastructure flag
or parallel CI-only Postgres definition.

During the audit, a full run failed three Phala Runner tests while an immediate
package rerun passed all 135 tests. The affected fake HTTP servers had
scheduler-sensitive polling and one-second deadlines. Their blocking fixture
lifecycles are now deterministic under parallel load, including the analogous
Kata fixture exposed by the repeated-run gate.

Core's real-Postgres harnesses now fail loudly when their required maintenance
URL is missing instead of returning success without running:

- [`finite-saas-core/src/store.rs`](finitecomputer-v2/crates/finite-saas-core/src/store.rs)
- [`launch_code_migration.rs`](finitecomputer-v2/crates/finite-saas-core/tests/launch_code_migration.rs)

The intended ownership boundary is:

- Nix supplies the exact Postgres version.
- Devfinity owns test-service lifecycle, temporary state, ports, readiness,
  environment, and cleanup.
- The Core test harness owns creation, migration, and deletion of an isolated
  database per test.
- `just test` defines the authoritative repository test command and asks
  devfinity to run it inside managed test infrastructure.

Tasks:

- [x] Add a general `devfinity run -- <command> [args...]` interface that runs
  an argument-vector child command by reusing devfinity's existing
  wrapped-command lifecycle.
- [x] Give `devfinity run` a lightweight baseline profile that always starts
  only an isolated Postgres 16 instance, waits for readiness, and adds its
  maintenance URL as `FC_CORE_POSTGRES_TEST_URL`; do not expose Postgres as an
  optional user-facing flag or start the complete product stack.
- [x] Isolate every invocation's temporary state and port, forward
  interruption, preserve the child's exact exit status, and tear infrastructure
  down on every exit path. Callers that need shell evaluation can explicitly
  use `bash -lc`.
- [x] Change `just test` to run
  `devfinity run -- cargo test --workspace --locked`; make CI invoke that same
  recipe and remove its separately defined GitHub Actions Postgres service.
- [x] Make Core's Postgres test harness fail loudly if it is invoked without
  the required maintenance URL; required tests must never return success
  without executing.
- [x] Fix the affected Phala and analogous Kata tests by replacing
  scheduler-sensitive polling fixture servers with bounded blocking I/O and
  deterministic shutdown.

Definition of done:

- [x] `devfinity run -- <command>` can wrap arbitrary commands and always
  provisions their baseline Postgres test infrastructure.
- [x] `just test` expands to
  `devfinity run -- cargo test --workspace --locked`; local and CI callers use
  that identical entry point without selecting or preconfiguring
  infrastructure.
- [x] Core's database-backed tests cannot report success without executing.
- [x] A failing or interrupted test run tears down its Postgres process and
  temporary state while preserving the test command's failure status, and
  parallel worktrees do not share ports, databases, or filesystem state.
- [x] The affected Runner package passes at least 20 repeated parallel runs.

### 2. [ ] Close the Hosted Chat P0 contract gap

The July Hosted Chat postmortem requires a stable machine-readable error code
and a real cross-service compatibility test:

- [`boss-hosted-chat-recovery-2026-07-16.md`](docs/postmortems/boss-hosted-chat-recovery-2026-07-16.md)

Hosted Device still returns only a human-readable `error` string:

- [`finitechat-hosted-device/src/lib.rs`](finitechat/crates/finitechat-hosted-device/src/lib.rs)

The dashboard still branches on exact status-plus-prose matches:

- [`hosted-web-chat.ts`](finitecomputer-v2/apps/dashboard/src/lib/hosted-web-chat.ts)

Tasks:

- [ ] Define a bounded error envelope containing a stable `code`, a
  human-readable `message`, and an optional `correlation_id`.
- [ ] Define stable codes as an explicit Rust type rather than ad hoc strings.
- [ ] Return the code from the real Hosted Device HTTP boundary.
- [ ] Update the dashboard to branch on the stable code.
- [ ] Keep the current prose fallback for exactly one compatibility window.
- [ ] Add a planned removal condition for the prose fallback.
- [ ] Add a cross-service test using the actual Rust Hosted Device producer and
  the actual dashboard classifier.
- [ ] Cover fresh, already-migrated, and N-1 control state.

Definition of done:

- [ ] Human copy can change without changing dashboard control flow.
- [ ] Browser fakes are not the only proof of the HTTP contract.
- [ ] The P0 items in the July postmortem can be marked complete with linked
  test evidence.

### 3. [ ] Converge Core tests on the production Postgres path

Core currently exposes `CoreStore::{Memory, Postgres}` and keeps a full
`BridgeCoreState` business implementation behind the memory arm:

- [`finite-saas-core/src/store.rs`](finitecomputer-v2/crates/finite-saas-core/src/store.rs)

There are currently 29 `CoreStore::memory()` call sites, mostly in API tests
and dry-run paths. This conflicts with Core's persistence contract, which says
every store operation must be tested against real Postgres because the memory
model cannot enforce database constraints:

- [`PERSISTENCE.md`](finitecomputer-v2/crates/finite-saas-core/PERSISTENCE.md)
- [`production-onboarding-chat-causality-2026-07-25.md`](docs/postmortems/production-onboarding-chat-causality-2026-07-25.md)

Tasks:

- [ ] Inventory every `CoreStore::memory()` caller and classify it as test,
  dry-run, production-reachable, or dead.
- [ ] Move Core API tests to isolated ephemeral Postgres databases.
- [ ] Extract reusable policy calculations into small pure functions used by
  the production Postgres implementation.
- [ ] Use rollback transactions or explicit planning types for dry-run
  operations.
- [ ] Prevent new API tests from using `CoreStore::memory()`.
- [ ] Delete business rules that exist only in `BridgeCoreState`.
- [ ] Reduce the memory implementation to an unmistakably narrow fault
  injection fake, or delete it entirely.

Definition of done:

- [ ] No authoritative API behavior is tested only through the memory store.
- [ ] Database constraints, conflicts, and transaction behavior are exercised
  by the normal Core test suite.
- [ ] There is one business implementation for every production operation.

### 4. [x] Trace and retire the legacy `finite-core` surface

Completed by deleting the `finite-core` crate and every relay-backed route.

Evidence and safety work:

- [x] Enumerate every production route that reaches `finite-core`.
      All were relay-backed `/api/finite/v1/*` routes in
      [`finite-saas-core/src/api.rs`](finitecomputer-v2/crates/finite-saas-core/src/api.rs);
      the two heartbeat routes that survive are Postgres-backed and never
      touched `finite-core`.
- [x] Name every writer, reader, persisted file, and active client.
      No in-tree caller of any relay route existed. Stronger: no runtime
      could authenticate to one — every launcher generates the bootstrap
      token, registers only its hash with Core, and discards the plaintext,
      so the runtime-token routes were unreachable in every environment.
- [x] Confirm whether the relay state is authoritative, transitional, or
      dead. Dead: Finite Chat owns ordered ciphertext and durable history;
      the relay ledgers could not be written by anyone.
- [x] Assign an owner and delete condition to the remaining artifact: the
      on-disk relay state under `/var/lib/private/finite-saas-core/relay`
      is unreferenced by code; archive or delete it during a normal ops
      pass (see the `StateDirectory` note in
      [`finite-saas-core.nix`](infra/nixos/modules/finite-saas-core.nix)).

Removal work:

- [x] Remove routes proven unreachable from production.
- [x] Delete legacy relay/chat/control-plane code (the entire `finite-core`
      crate) and the dashboard's unused relay client and `/api/finite`
      public proxy.
- [x] Confirm that the root `Cargo.lock` contains no archived Finite Chat
  dependency generation.

Definition of done:

- [x] No production crate depends on the archived Finite Chat repository.
- [x] Every remaining compatibility bridge has a documented owner and delete
  condition (only the inert on-disk relay state remains).
- [x] Existing users and durable chat state remain readable throughout the
  transition (Finite Chat state was never stored in the relay).

## P1 — Durable compatibility and product-level proof

### 5. [ ] Introduce an append-only database migration ledger

Core currently concatenates migrations `0001` through `0016` into
`CORE_SCHEMA_SQL` and executes the entire string at startup:

- [`finite-saas-core/src/lib.rs`](finitecomputer-v2/crates/finite-saas-core/src/lib.rs)
- [`finite-saas-core/src/store.rs`](finitecomputer-v2/crates/finite-saas-core/src/store.rs)

Tasks:

- [ ] Add a migration history table containing identifier, checksum, and
  applied timestamp.
- [ ] Apply each migration in its own transaction.
- [ ] Refuse startup when the checksum of an applied migration changes.
- [ ] Establish an append-only rule for committed migrations.
- [ ] Test migration from a pinned prior-production schema fixture.
- [ ] Test idempotent restart after all migrations are applied.
- [ ] Document expand/contract, rollback, and mixed-version deployment rules.
- [ ] Add a recovery test for interrupted or failed migration application.

Definition of done:

- [ ] Historical migrations cannot be silently edited and reapplied.
- [ ] A current binary can migrate a supported predecessor schema.
- [ ] The supported rollback reader remains proven for the documented window.

### 6. [ ] Make compatibility an executable release contract

Finite Chat already has strong predecessor fixtures containing actual encrypted
SQLite bytes, exact writer commits, hashes, and explicit warnings not to
regenerate them with the current writer:

- [`finitechat-client test fixtures`](finitechat/crates/finitechat-client/tests/fixtures/README.md)

The hand-maintained `compat/matrix.toml` was retired on 2026-08-21 (ownership
audit O7): nothing read it, and it drifted from the pins that actually run.
Fielded versions are read from release tags, Core's runtime-artifact table, and
the NixOS closure; non-derivable release narrative lives in
[`infra/deployment-changelog.md`](infra/deployment-changelog.md).

Tasks:

- [ ] Define a schema for compatibility fixture provenance.
- [ ] Record writer commit/version, reader versions, migration expectation,
  rollback expectation, and fixture checksum.
- [ ] Register fixtures for chat state, Core schema, Device identity, Agent
  Runtime state, and cross-service protocol envelopes as applicable.
- [ ] Require fresh, already-migrated, and N-1 fixtures for changes to durable
  authorization, identity, or protocol state.
- [ ] Add a changed-path classifier that selects relevant compatibility lanes.
- [ ] Record required deployment order and mixed-version behavior.
- [ ] Reject unregistered changes to persisted or wire representations.

Definition of done:

- [ ] Compatibility claims point to executable fixtures and tests.
- [ ] CI runs the relevant predecessor lanes without an all-version
  combinatorial matrix.
- [ ] Release promotion can state exactly which writer/reader combinations
  were proven.

### 7. [ ] Add a normal-CI full product-spine proof

The portable devfinity smoke is services-only and explicitly does not launch an
Agent Runtime:

- [`devfinity/justfile`](devfinity/justfile)

The real Docker Runtime smoke remains dispatch-only on a self-hosted runner:

- [`hermes-runtime-smoke.yml`](.github/workflows/hermes-runtime-smoke.yml)

Required product through-line:

```text
dashboard action
  → Core/Postgres
  → Runner
  → canonical Agent Runtime
  → Identity
  → Hosted Device
  → durable chat
  → restart
  → readable history
```

Tasks:

- [ ] Add a portable Docker-backed full-SaaS profile.
- [ ] Use deterministic local inference at the PR boundary.
- [ ] Keep paid real-model proof in a scheduled or promotion lane.
- [ ] Exercise the actual Core Postgres implementation.
- [ ] Verify Runtime health and `/contact` Agent Principal readiness.
- [ ] Verify Hosted Device binding and first chat.
- [ ] Restart the Runtime without replacing its durable volume.
- [ ] Verify chat history remains readable after restart.
- [ ] Add one existing-Agent wake-up or N-1 state lane.

Definition of done:

- [ ] A normal CI event proves the user-visible product spine.
- [ ] A fresh canary and an existing-state canary are separately represented.
- [ ] Exact-image promotion retains the deeper real-provider and restart proof.

### 8. [ ] Monitor product readiness, not only process health

Current monitoring curls local service endpoints and reports failures to the
journal. External deadman paging remains a TODO:

- [`monitoring.nix`](infra/nixos/modules/monitoring.nix)

Tasks:

- [ ] Add an external deadman signal that pages on silence.
- [ ] Export the number of eligible Agent-creation Runners.
- [ ] Export capacity, drain state, and heartbeat freshness.
- [ ] Alert when no Standard Runner can accept creation outside maintenance.
- [ ] Add a synthetic existing-user chat-readability probe.
- [ ] Add a synthetic new-user admission and chat-readiness probe.
- [ ] Distinguish process alive, dependency ready, protocol compatible, and
  product usable.
- [ ] Expose user-visible maintenance state when creation capacity is
  intentionally zero.

Definition of done:

- [ ] A zero-creator fleet is detected before a user reports it.
- [ ] A broken cross-service composition can alert even when every process
  health endpoint is green.
- [ ] Every critical alert has an owner and runbook.

### 9. [ ] Align toolchains and enforce workspace lints

The repository currently declares Rust 1.88 as its MSRV, uses Rust 1.91.1 in
Nix, and uses Rust 1.93 in CI:

- [`Cargo.toml`](Cargo.toml)
- [`flake.nix`](flake.nix)
- [`ci.yml`](.github/workflows/ci.yml)

Only 7 of 36 workspace packages opt into workspace lints. The root
`unsafe_code = "forbid"` policy is therefore not generally enforced, and the
daemon contains production `from_raw_fd` unsafe blocks:

- [`finitechat-daemon/src/main.rs`](finitechat/crates/finitechat-daemon/src/main.rs)

Tasks:

- [ ] Add a root `rust-toolchain.toml`.
- [ ] Use the same pinned developer/CI compiler in Nix and normal CI.
- [ ] Keep a separate explicit Rust 1.88 MSRV build/test lane.
- [ ] Add `[lints] workspace = true` to every workspace member.
- [ ] Centralize edition, `rust-version`, and common package metadata.
- [ ] Replace raw-FD unsafe blocks with safe ownership conversions where
  possible.
- [ ] Document and narrowly scope any necessary unsafe exception.
- [ ] Incrementally strengthen correctness and suspicious-code lints.

Definition of done:

- [ ] A checkout uses the same default Rust toolchain locally and in CI.
- [ ] Every workspace crate inherits the root lint policy.
- [ ] The MSRV claim is continuously proven rather than inferred from release
  workflows.

## P2 — Guardrails, maintainability, and review quality

### 10. [ ] Establish a supply-chain security baseline

The public repository currently has no root `cargo-deny` policy, automated
dependency-update configuration, or checked-in secret-scanning configuration.
GitHub Actions are version-tag pinned rather than immutable-commit pinned.

Tasks:

- [ ] Add `cargo-deny` policy for advisories, licenses, duplicate versions, and
  allowed sources.
- [ ] Add npm audit policy with documented, expiring exceptions.
- [ ] Add secret scanning suitable for a public repository.
- [ ] Add grouped automated dependency update pull requests.
- [ ] Pin third-party GitHub Actions to immutable commit SHAs.
- [ ] Remove archived Git dependencies before broadly allowing Git sources.
- [ ] Add SBOM and provenance/attestation generation to release workflows.
- [ ] Document the response process for a leaked secret: rotate first, then
  remove.

Definition of done:

- [ ] New dependency sources and license exceptions require explicit review.
- [ ] Known advisories cannot disappear into unaudited lockfile drift.
- [ ] Release artifacts have machine-readable provenance.

### 11. [ ] Generate and validate cross-language wire contracts

Dashboard Core DTOs, Hosted Device responses, and browser fakes duplicate Rust
wire shapes manually. This makes it possible for producers and consumers to
remain internally green while disagreeing at runtime.

Tasks:

- [ ] Select a narrow Rust-to-schema mechanism such as OpenAPI, JSON Schema,
  `ts-rs`, or an equivalent.
- [ ] Generate TypeScript types for public wire DTOs and error envelopes.
- [ ] Add runtime validation at important HTTP boundaries.
- [ ] Add golden contract tests against the actual Rust router.
- [ ] Prevent browser fakes from redefining production DTOs independently.
- [ ] Keep internal domain types out of generated public contracts.
- [ ] Version intentionally breaking wire changes.

Definition of done:

- [ ] Rust and TypeScript cannot independently redefine the same wire enum or
  error code.
- [ ] Consumer tests use artifacts produced from the real producer contract.
- [ ] Invalid or unknown payloads fail at a named boundary with useful
  diagnostics.

### 12. [ ] Decompose mega-modules along ownership boundaries

Notable concentrations include:

- `finitechat-core/src/lib.rs`: approximately 20,694 lines.
- `finite-saas-core/src/store.rs`: approximately 14,189 lines.
- `finite-saas-core/src/lib.rs`: approximately 13,791 lines.
- `finitechat-client/src/lib.rs`: approximately 13,407 lines.

This work should follow authoritative-path cleanup. Splitting a shadow
implementation first would make it harder to delete.

Tasks:

- [ ] Delete dead and shadow paths before moving code.
- [ ] Identify state owners, transaction boundaries, and protocol boundaries.
- [ ] Extract modules along those boundaries rather than arbitrary line counts.
- [ ] Move focused tests and fixtures beside their owning modules.
- [ ] Introduce narrow traits only at actual I/O or nondeterministic
  boundaries.
- [ ] Add a review guard against further growth of existing mega-modules.
- [ ] Apply property, mutation, or fuzz testing selectively to parsers,
  migrations, replay/idempotency, and compatibility readers.

Definition of done:

- [ ] A change to one state owner does not require understanding an unrelated
  10,000-line module.
- [ ] Module boundaries match production ownership and transactional behavior.
- [ ] Test doubles represent real boundaries rather than alternate business
  implementations.

### 13. [ ] Add causal review and documentation ownership

Some repository documentation is explicitly imported and not fully
revalidated, and the Phase 13 stale-doc audit remains pending:

- [`docs/README.md`](docs/README.md)
- [`docs/local-dev-matrix.md`](docs/local-dev-matrix.md)

The repository also lacks root CODEOWNERS and a pull request template.

Tasks:

- [ ] Add CODEOWNERS for chat state/protocols, Core persistence, Agent Runtime,
  Identity, and production infrastructure.
- [ ] Add a risk-oriented pull request template.
- [ ] Require identification of changed writers, readers, and serialized state.
- [ ] Require existing-state, N-1, and partial-deployment analysis where
  applicable.
- [ ] Require the real production entry point and tested implementation to be
  named.
- [ ] Require rollback, recovery, observability, and bridge delete conditions.
- [ ] Create a canonical-document manifest with owner, status, and
  last-verified date.
- [ ] Archive or clearly mark historical plans and imported orientation docs.
- [ ] Add documentation link/reference checks to CI.

Definition of done:

- [ ] High-risk changes automatically prompt a causal compatibility review.
- [ ] Reviewers can distinguish canonical, historical, and unverified docs.
- [ ] Persisted-state and protocol changes have named owners.

## Recommended first ten working days

### Days 1–3

- [x] Add `devfinity run -- <command>` with automatic isolated Postgres
  setup, environment injection, and teardown.
- [x] Wire `just test` and CI through
  `devfinity run -- cargo test --workspace --locked`.
- [x] Make Core's Postgres tests fail closed when their maintenance connection
  is unavailable.
- [x] Fix and repeatedly prove the affected Runner tests.

### Days 2–4

- [ ] Implement the Hosted Device error envelope.
- [ ] Update the dashboard classifier.
- [ ] Add the real Rust-to-dashboard contract test.

### Days 3–5

- [ ] Align the default Rust toolchain.
- [ ] Opt every crate into workspace lints.
- [ ] Add the first lightweight supply-chain gates.

### Week 2

- [ ] Begin moving Core API tests from `MemoryCoreStore` to Postgres.
- [ ] Produce the `finite-core` reachability and deletion report.
- [ ] Land the first proven-dead legacy removal.
- [ ] Add the migration ledger before further Core schema growth.
- [ ] Design the portable full product-spine test and product-readiness
  signals.

## Program-level completion criteria

- [x] `just test` is green, hermetic, and cannot silently skip required tests.
- [ ] Hosted Chat control flow uses stable machine-readable codes.
- [ ] Core has one authoritative business implementation.
- [ ] Production crates no longer depend on archived component repositories.
- [ ] Database migrations are append-only and checksum-verified.
- [ ] Persisted-state and protocol compatibility claims are fixture-backed.
- [ ] Normal CI proves a complete user-visible Agent and chat through-line.
- [ ] Monitoring detects loss of onboarding or chat readiness before users do.
- [ ] Toolchain, lint, dependency, and review policies are mechanically
  enforced.

## Explicit non-goals for the first phase

- [ ] Do not adopt a blanket line-coverage target as the primary maturity
  measure.
- [ ] Do not split large files before deleting shadow and dead paths.
- [ ] Do not add mocks that reproduce another implementation of production
  business behavior.
- [ ] Do not introduce an all-version compatibility matrix when changed-path
  selection and representative predecessor fixtures are sufficient.
- [ ] Do not begin a broad rewrite while production reachability and durable
  compatibility remain ambiguous.
