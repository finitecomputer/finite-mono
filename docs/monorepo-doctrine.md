# Monorepo Doctrine

Adopted 2026-07-08 (Paul + the migration-integration branch). This supersedes
every earlier statement that "Finite is not becoming a monorepo" — in the old
workspace AGENTS.md, WORKSPACE_INVENTORY.md, and finitecomputer-v2's
README/AGENTS/service-dependencies docs. Those statements described the
pre-mono world and are void.

The GitHub/Depot CI boundary was adopted 2026-08-25 in ADR-0007. GitHub remains
the Source Authority while Depot becomes the CI execution and check-reporting
control plane.

## Product safety maxims

1. **Don't Break Chat!** Chat availability and durable history are Finite's
   primary product promise. Any change touching persisted chat state,
   protocols, Device identity, Agent Runtime state, or deployment topology
   must trace the production through-line and prove the relevant
   existing-state and mixed-version compatibility edges. A candidate tested
   only with matching candidate components is not deployment evidence.
2. **Don't break new-user onboarding.** Account enrollment, Agent admission,
   launch, identity readiness, binding, and usable chat are separate causal
   contracts inside one user promise. Runner drain, capacity, and host
   placement are therefore product availability state, not merely operator
   configuration.
3. **Prefer causal simplicity over test volume.** Fewer authoritative
   implementations, explicit writers and readers, production-faithful entry
   points, and small through-line proofs are better than large suites around
   shadow implementations. Say no to features whose compatibility, recovery,
   and observability contracts cannot be made clear and affordable.

The incidents motivating these maxims and the questions future work should ask
are recorded in
[`production-onboarding-chat-causality-2026-07-25.md`](postmortems/production-onboarding-chat-causality-2026-07-25.md).

## The doctrine

1. **finite-mono on GitHub is the single company repository and Source
   Authority.** All first-party code —
   product CLIs, servers, the SaaS control plane, apps (dashboard, iOS,
   Electron), protocols, skills, and infrastructure definitions — lives here.
   Work lands in `finitecomputer/finite-mono` first; there is no "sync back" to
   the old per-component repositories.
2. **The old per-component repos are import provenance, not homes.** Each was
   snapshot-imported (no git history; SHAs recorded in
   `docs/monorepo-migration-log.md` and `scripts/import-sync.toml`). After
   cutover they are archived read-only with a README pointer here. If a stray
   commit lands on one before it is archived, `scripts/import-sync <name>`
   merges it in safely.
3. **Product source releases are component-scoped tags on the Source
   Authority**: `finitechat/vX.Y.Z`, `fsite/vX.Y.Z`, `fbrain/vX.Y.Z`, plus
   dispatch-versioned images
   (`finite-agent-runtime`, `finite-saas-core`, `finite-saas-dashboard`,
   `finite-private-limiter`). The public `finitecomputer/finite-releases`
   Release Repository receives corresponding release-only tags and assets; it
   is not a source repository. Release asset names are product contracts —
   never rename them.
4. **`finitecomputer/finite-releases` is the public Release Host (adopted
   2026-08-25).** Because it hosts several components,
   repository-wide `releases/latest` is meaningless. Installers use
   per-component rolling alias releases (`finitechat-latest`, `fsite-latest`,
   `fbrain-latest`) that Depot refreshes only after the immutable versioned
   assets are verified. GitHub `finite-mono` release assets are backfilled and
   then retained for historical releases; new publication uses the dedicated
   Release Repository (see infra/runbooks/release-cli.md).
5. **`infra/` is the single deploy root.** Nothing is built on a prod box;
   images are CI-built and digest-pinned; deploys are scripts/runbooks in this
   tree. See `infra/README.md`.
6. **The Source Authority is public and remains secret-free.** Git history is
   not a safe secret store. No secret values, ever — names and locations only.
   Rotate first, then delete, if one slips in.

## What stays outside, and why

- **Legacy `finitecomputer`** — runs box1/TRF/the OVH fleet until those users
  migrate. Its Nix fleet pattern is the best IaC we have; we copy the pattern,
  not the content. It also still owns two things mono must eventually take:
  the Finite Private ops script (`finite_private_ops.sh`) and the deployed
  limiter image build (now replaced by `service-images.yml` here).
- **Tinfoil satellite repos** (`confidential-kimi-k2-6`,
  `finite-searxng-tinfoil`, `tinfoil-agent-runtime-canary`) — Tinfoil enclave
  measurement requires `tinfoil-config.yml` at the ROOT of a repo, one config
  per repo, so they cannot fold into mono even though source is centralized.
  They stay thin: their inputs (image digests, configs) are produced and pinned
  by mono CI. See `infra/tinfoil/README.md`.
- **finite-fable** — Paul's meta/strategy notes; not a git repo by design.
- **Spikes and stale checkouts** (hermes-agent forks, darkmatter, finitesmol,
  finitechat-old, finite-site, …) — archive aggressively; nothing imports them.

## For agents (human and AI) working in the old workspace

If you are reading a checkout of a pre-mono repo: stop and check whether the
work belongs in finite-mono. The old `dev/finite/AGENTS.md` orientation file
now points here. Per-repo checkouts remain useful only for reading history.
