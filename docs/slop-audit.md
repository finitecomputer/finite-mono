# Slop Audit

> Status: imported from `finite-eng-docs` during Phase 7 on 2026-07-06. This
> document has not been fully revalidated after the monorepo import. Treat it as
> orientation background, not an authoritative current quality report.

Date: 2026-07-02

This is an onboarding risk map for the cloned Finite repos. It is intentionally
blunt, but it is not a full code review, security review, or test run. It is
based on static reading of repo layout, docs, scripts, obvious debt ledgers,
TODO/fake/temporary markers, and sampled implementation files.

Use this to decide where to spend attention before shipping.

## Rating Scale

| Level | Meaning |
| --- | --- |
| 1 | Clean enough: small scope, coherent ownership, low transition risk |
| 2 | Some rough edges: understandable, bounded, mostly local debt |
| 3 | Sloppy but navigable: real architecture exists, but quality depends on docs and discipline |
| 4 | High transition risk: usable, but old and new architectures coexist in product paths |
| 5 | Not shippable as-is: unclear owner, fake core behavior, or missing proof for critical path |

## Snapshot

| Repo | Slop Level | Read | Main Risk |
| --- | --- | --- | --- |
| `finitecomputer-v2` | 4 | Hard-cut SaaS spine is explicit, but newly split | Copied legacy Core/runtime/dashboard pieces can leak old machine/relay assumptions into the v2 Project/Runner product |
| `finitecomputer` | 4 | Legacy whiteglove product is real, but transitional | box1/TRF support, dashboard relay paths, and migration bridge code can be mistaken for the v2 product architecture |
| `finitechat` | 3 | Strong architecture, large debt surface | Good protocol discipline, but fake/echo harness history and very large core files make validation hard |
| `finite-skills` | 3 | Useful baseline, uneven validation | Many user-visible skills with little repo-level test or lint structure |
| `finite-search` | 2 | Narrow ops repo | Remote-host proof is practical, but local reproducibility and Tinfoil path are still rough |
| `finite-nostr` | 1 | Small library | Low jank; main risk is dependency/API maturity |
| `reporting` | 2 | Small reporting repo | Generator ownership is split with legacy `finitecomputer`; outputs rely on manual/live context |

## Evidence Heuristics

These counts are directional only. They exclude obvious build output where
practical, but marker counts still include false positives in docs and examples.

| Repo | Files | Markdown files | Test-file heuristic | Risk-marker hits |
| --- | ---: | ---: | ---: | ---: |
| `finitecomputer-v2` | 210 | 15 | 16 | 194 |
| `finitecomputer` | 342 | 80 | 10 | 106 |
| `finitechat` | 211 | 51 | 34 | 116 |
| `finite-search` | 44 | 27 | 0 | 8 |
| `finite-skills` | 268 | 119 | 0 | 101 |
| `finite-nostr` | 16 | 7 | 0 | 0 |
| `reporting` | 59 | 5 | 0 | 0 |

Rust inline tests are undercounted by the test-file heuristic. For example,
`finitechat` has many inline test annotations, and `finite-nostr` appears to use
inline tests despite having no separate `tests/` directory.

## Highest Shipping Risks

1. `finitecomputer-v2` is the intended self-serve SaaS product, while
   `finitecomputer` remains the shipped legacy whiteglove product. Docs, code
   labels, and tests must keep that split sharp or engineers will route new
   product work into the wrong repo.
2. The v2 product chat path is native Finite Chat with a no-PIN invite.
   Dashboard relay chat in legacy `finitecomputer` is still useful, but it
   should not drift into the v2 launch contract.
3. `finitecomputer-v2` intentionally carries copied legacy Core/runtime pieces.
   Machine/control-plane/relay assumptions need named delete conditions before
   they become permanent SaaS architecture.
4. Real Hermes proof versus echo/fake proof needs constant attention.
   `finitechat` has an explicit audit for this, which is good, but it means the
   team has already been bitten by overclaimed demos.
5. Several important modules are too large for easy new-developer confidence.
   Examples include `finitechat-core/src/lib.rs`, `finitechat-client/src/lib.rs`,
   `finitechat-server/src/lib.rs`, `finitecomputer/crates/fc/src/main.rs`,
   `finitecomputer-v2/crates/finite-core/src/relay.rs`,
   `finitecomputer-v2/crates/finite-saas-core/src/lib.rs`, and
   `finitecomputer-v2/crates/finite-saas-core/src/store.rs`.
6. Local development has prescribed paths, but none are the same as the full
   hosted production substrate. That is fine, but docs and tests need to keep
   the distinction sharp.
7. Skills are likely user-visible product quality, but the repo looks more like
   a curated content dump than a validated package set.

## `finitecomputer-v2`

Slop level: 4.

### Good Architecture Choices

- The hard-cut rule is clear: v2 is the self-serve SaaS product, while existing
  box1/TRF users stay on legacy `finitecomputer` until migrated.
- Product vocabulary is explicitly defined in `CONTEXT.md`: Project, Agent
  Runtime, Runner, Core, Hosted Pairing, Finite Chat Invite, and Runtime-Scoped
  Finite Private Key.
- Dashboard chat, OpenCode, dashboard-managed Published Apps, broad `finitec`
  publish/repo/gateway/hermes/chat commands, and legacy machine control-plane
  operations are named as non-goals.
- Deployment boundaries are now documented: Core/dashboard, hosted Finite Chat,
  Agent Runtime, Finite Sites API/serving, and coordinated release order.
- The Hermes runtime proof ladder is product-shaped: real Hermes, native Finite
  Chat, no PIN, local Docker, Kata, Phala, then dashboard-controlled
  SaaS launch.
- The carry-over and legacy cleanup manifests make copied code and delete
  conditions visible instead of pretending the split is already clean.

### Slop And Bad Choices

- The repo intentionally starts too large. `crates/finite-core` is copied only
  to keep the first source slice coherent, but it still contains legacy
  chat/control-plane/relay models.
- Dashboard code still contains machine routes, labels, and compatibility paths
  even though v2 docs say the product language should be Project and Agent
  Runtime.
- `deploy/finite-computer` still carries legacy naming and runtime-template
  residue. The active image path now packages `finitechat`, the Hermes
  `finitechat` plugin, `fsite`, and `fbrain`, but some template docs and
  healthcheck expectations still mention older plugin/identity shapes.
- There is no root contributor facade yet. Local checks are discoverable through
  Cargo and dashboard npm scripts, but product acceptance requires reading the
  runtime matrix.
- The Finite Private limiter is correctly owned by v2 for now, but the docs say
  it should eventually move behind its own repo/deploy boundary.
- Runtime proof is still aspirational until the local Docker, remote Docker,
  Phala, and dashboard-controlled rungs have recorded evidence.

### Obvious Placeholder Or Transitional Hacks

- The `finite-private-limiter` extraction is a TODO with an explicit stability
  condition.
- `deploy/finite-computer` needs renaming and pruning.
- Runtime-template docs and healthchecks need to be brought back in line with
  the active image path: shared `FINITE_HOME` identity, the `finitechat` Hermes
  plugin, and packaged `fbrain`.
- Some dashboard and Core code still exposes legacy machine/control-plane
  concepts as compatibility bridge code.

### Before Shipping

- Make every v2 user-facing flow use Project, Agent Runtime, Runner, Hosted
  Pairing, and Finite Chat Invite language.
- Remove or cordon off copied `finite-core` relay/control-plane models behind
  v2-owned DTOs.
- Prove the runtime image through the documented Docker, Kata, Phala,
  and dashboard-controlled launch rungs with real Hermes and native Finite Chat.
- Ensure dashboard/Core checks are a single named command or root facade before
  external contributor onboarding depends on them.
- Keep dashboard chat and `finitec publish`/`finitec repo` out of v2 unless a
  migration document names the bridge and delete condition.

## `finitecomputer`

Slop level: 4.

Read as: legacy whiteglove product and migration bridge, not the default home
for new self-serve SaaS work.

### Good Architecture Choices

- The legacy platform has a real local harness: dashboard plus `finited` relay
  plus MicroSandbox runtime. This is much better than pure mocks for legacy UI
  iteration.
- The docs name the intended boundary: browser talks to Finite, machine connects
  outward, and SSH/Kubernetes should be break-glass or hosted-runner admin.
- `finitec`/relay migration is tracked in an active ledger with statuses and
  delete conditions.
- Runtime operations, backups, fleet behavior, Hermes baseline, and local chat
  development have runbooks instead of living only in shell history.
- The dashboard has conventional scripts for `dev`, `build`, `lint`, and tests.

### Slop And Bad Choices

- Before the v2 split, this repo mixed future SaaS work with the already-shipped
  product. New docs must now keep those paths separate.
- There are many half-moved surfaces: chat messages, attachments, published app
  listing, connection status, model selection, org/user skills, local chat dev,
  and Kubernetes pod admin.
- The current relay is explicitly file-backed, polling-based, plaintext, and not
  the final durable/E2EE protocol.
- Dashboard/control-plane code still knows too much about runtime internals in
  several areas, especially connection state, published apps, and hosted admin.
- Some repo-level docs are current runbooks, while others are historical plans
  or root-level brain dumps. New developers need the doc index to avoid stale
  guidance.
- Large implementation files make ownership boundaries harder to see:
  `crates/fc/src/main.rs` is over 6k lines, and the dashboard chat component is
  nearly 3k lines.
- The local harness depends on MicroSandbox availability and still has a
  host-Hermes fallback. That fallback is useful for break-glass debugging but
  dangerous if it becomes normal acceptance coverage.

### Obvious Placeholder Or Transitional Hacks

- Finite Private model/provider support is listed as a placeholder in the
  migration ledger.
- The host-Hermes local loop exists as a temporary escape hatch.
- Tinfoil/Phala/state backup docs include fake-data canary paths. Those are
  fine as spikes, but they are not proof for real user data.
- Some state and secrets paths live close to the repo structure. The files read
  as templates/runbooks, but this deserves periodic secret hygiene checks.

### Before Shipping

- Keep this repo focused on box1/TRF/smoke support and migration bridge work.
- Prevent new dashboard features from reading or mutating runtime-local files
  directly.
- Turn each remaining `Half-Moved` or `Legacy` row in the transport ledger into
  either a delete plan or a named hosted-runner API.
- Run the MicroSandbox local loop as the normal acceptance target and demote
  host-Hermes fallback to troubleshooting only.
- Split the largest CLI/dashboard modules after behavior stabilizes enough that
  extraction is mechanical.

## `finitechat`

Slop level: 3.

### Good Architecture Choices

- The repo has a clear protocol architecture: Rust owns state, server ordering,
  typed route contracts, OpenMLS helpers, and client projection.
- The trust split is well documented: server orders opaque bytes, clients own
  cryptography and membership truth.
- The repo has ADRs, protocol docs, scenario coverage, canary loops, and an
  explicit technical debt ledger.
- Debt entries are unusually disciplined: they include observed source, risk,
  first proof, and delete condition.
- Local server, simulator, phone, Docker, and Hermes gateway paths are
  documented.

### Slop And Bad Choices

- The system is ambitious and currently carries a lot of transition debt around
  v2/legacy Finite Computer integration, app state, attachment behavior, and
  Hermes proof.
- The repo already has an "Oops I Faked It" audit. That is good culturally, but
  it is also a strong signal that demos/tests have previously overclaimed what
  they proved.
- Some core files are very large: `finitechat-core/src/lib.rs` is over 13k
  lines, `finitechat-client/src/lib.rs` is over 10k lines, and
  `finitechat-server/src/lib.rs` is over 7k lines.
- There are still fake-MLS, fake-daemon, mock blob, and echo-handler concepts in
  the docs/tests. Many are labeled honestly, but they increase cognitive load.
- Product app state pollution has been a known problem across simulator,
  physical phone, launch automation, and test paths.

### Obvious Placeholder Or Transitional Hacks

- Echo-handler Hermes tests prove adapter transport, not real Hermes reasoning.
- Temporary canary backup secrets are documented for Docker/Tinfoil canaries.
- Kotlin binding placeholder files exist in the RMP generator path.
- Swift UI has areas intentionally ahead of Rust projection, especially around
  byte-level transfer progress.

### Before Shipping

- Do not accept any "Hermes works" claim unless it satisfies the real-Hermes
  definition in `docs/oops-i-faked-it-audit.md`.
- Keep the technical debt ledger active; do not let pre-release compatibility
  shims become permanent product behavior.
- Extract large Rust modules when stable boundaries are obvious, especially runtime
  app state, storage/projection, and server route families.
- Ensure hosted web copy does not imply E2EE when server-side device secrets are
  involved.
- Make the `finitecomputer-v2` SaaS deploy handoff explicit for any app release
  that depends on hosted server behavior, and name any remaining legacy
  `finitecomputer` exposure separately.

## `finite-skills`

Slop level: 3.

### Good Architecture Choices

- The repo has a clear ownership rule: it is the platform-owned managed skill
  baseline, not the place for machine-specific customizations.
- Skills use a naming convention intended to avoid collisions with built-in or
  user-local Hermes skills.
- There is real content breadth: productivity, research, software development,
  finance, Nostr, publishing, and creative workflows.

### Slop And Bad Choices

- The repo is mostly content, examples, and embedded reference material. There
  is no obvious repo-level validation harness for skill metadata, examples, or
  referenced scripts.
- Risk-marker counts are high, though many are expected because skills teach
  users how to handle TODOs, placeholders, mocks, and temporary files.
- Some skills include large third-party schemas/templates and detailed
  references. That is useful, but it makes quality hard to audit by inspection.
- User-visible behavior depends on model interpretation of Markdown contracts,
  which is inherently softer than typed code.

### Obvious Placeholder Or Transitional Hacks

- Several research/writing skills explicitly allow placeholder citations when
  marked for human verification. That is acceptable only if never confused with
  final output.
- Website/game guidance repeatedly warns against placeholder assets, implying
  this has been a recurring failure mode.
- Some skill examples use dummy IDs such as `xxx`, which is fine for examples
  but noisy for automated quality checks.

### Before Shipping

- Add a lightweight skill linter for required frontmatter, naming conventions,
  related skill references, broken reference paths, and forbidden unmanaged
  names.
- Add a smoke suite for the most important skills inside a real Hermes runtime.
- Separate vendored templates/reference assets from first-party skill contracts
  in the audit view.
- Define which skills are production baseline versus experimental.

## `finite-search`

Slop level: 2.

### Good Architecture Choices

- The repo has a narrow purpose: self-hosted search and extraction for agents.
- It separates SearXNG and Firecrawl and explicitly avoids over-designing
  Tinfoil before plain Docker works.
- The README is honest that this is ops/integration, not a product app.
- Static checks, doctor scripts, smokes, and benchmark scripts exist and are
  easy to understand.

### Slop And Bad Choices

- The happy path is mostly remote-host oriented. Local smokes depend on SSH
  tunnels into `lat2`, so a new developer without that access cannot fully
  reproduce the current proof.
- There are no conventional test files. Confidence comes from shell smokes and
  deployed-service checks.
- The Tinfoil path still contains placeholder image references and follow-up
  docs.
- Firecrawl is wrapped from upstream rather than deeply owned here, so its
  security/resource behavior is mostly an ops concern.

### Obvious Placeholder Or Transitional Hacks

- `tinfoil/searxng-public/tinfoil-config.yml` still contains an owner/repo image
  placeholder.
- Docs include a placeholder digest/tag for the future Tinfoil image.

### Before Shipping

- Decide whether "shipping" means hosted `lat2` service stability or a reusable
  package others can deploy.
- Add local Docker Compose smoke instructions that do not require SSH, if local
  reproducibility matters.
- Replace Tinfoil placeholders only after auth, image publishing, and digest pin
  policy are decided.
- Add a minimal CI smoke for config syntax and script behavior beyond static
  placeholder checks.

## `finite-nostr`

Slop level: 1.

### Good Architecture Choices

- The scope is small and product-neutral.
- The README clearly says FiniteBrain/FiniteChat policy should stay out.
- Code is split into modest modules around identity, auth, events, NIP-44, and
  NIP-59.
- There are no obvious TODO/fake/hack markers from the static scan.

### Slop And Bad Choices

- The repo depends on an alpha Nostr crate version, which may be fine but should
  be treated as API churn risk.
- There are no separate integration-test files visible, though Rust inline tests
  appear to exist.
- Because the crate is intended for reuse, breaking API changes can ripple into
  larger repos quickly.

### Obvious Placeholder Or Transitional Hacks

- None obvious from this initial pass.

### Before Shipping

- Run the standard `cargo fmt --check`, `cargo test`, and `cargo clippy`
  workflow.
- Add cross-crate integration tests when `finitechat` or another repo depends on
  a specific Nostr behavior.
- Track upstream `nostr` crate version changes deliberately.

## `reporting`

Slop level: 2.

### Good Architecture Choices

- The repo is small and clearly scoped to durable reporting outputs and notes.
- It keeps raw secrets and private conversation content out of the reporting
  contract.
- Generated CSV snapshots are treated as immutable evidence.
- The site-data builder and test are small enough to inspect directly.

### Slop And Bad Choices

- The current generator lives in legacy `finitecomputer`, not `reporting`, so
  ownership is split.
- Live data depends on optional SSH/env configuration, so local runs can silently
  skip important probes unless the evidence log is read.
- Manual name mapping is explicitly part of the process. That is reasonable for
  reporting, but not mechanically trustworthy.
- The repo contains generated CSV output, so diffs can be noisy and reviewers
  need to distinguish source changes from snapshots.

### Obvious Placeholder Or Transitional Hacks

- None obvious from marker scans, but skipped live probes and manual mappings
  are the main soft spots.

### Before Shipping

- Decide whether the generator should move into `reporting`, move to
  `finitecomputer-v2`, or remain owned by legacy `finitecomputer`.
- Add a top-level command for "build report from available local context" that
  makes skipped probes obvious.
- Keep generated run folders immutable and avoid editing historical snapshots.

## Suggested First Cleanup Pass

1. Keep `finitecomputer-v2`, legacy `finitecomputer`, and `finitechat` aligned
   on the chat truth table: v2 native Finite Chat invite, legacy dashboard
   relay, hosted web trust mode if it returns, and Hermes bridge proof.
2. Promote `finitecomputer-v2/docs/carry-over-manifest.md`,
   `finitecomputer-v2/docs/legacy-cleanup-manifest.md`,
   `finitecomputer/docs/finitec-transport-migration-ledger.md`, and
   `finitechat/docs/technical-debt-ledger.md` into the monorepo navigation docs
   as required debt ledgers.
3. Turn `local-dev-matrix.md` from a static inventory into a verified matrix by
   running each documented local command on this machine and recording dates,
   expected output, and known access gaps.
4. Add lightweight validation for `finite-skills`.
5. Split only the large modules that are actively blocking comprehension or
   change safety. Do not refactor them just to make the file sizes nicer.
