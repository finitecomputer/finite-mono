# Monorepo Doctrine

Adopted 2026-07-08 (Paul + the migration-integration branch). This supersedes
every earlier statement that "Finite is not becoming a monorepo" — in the old
workspace AGENTS.md, WORKSPACE_INVENTORY.md, and finitecomputer-v2's
README/AGENTS/service-dependencies docs. Those statements described the
pre-mono world and are void.

## The doctrine

1. **finite-mono is the single company repository.** All first-party code —
   product CLIs, servers, the SaaS control plane, apps (dashboard, iOS,
   Electron), protocols, skills, and infrastructure definitions — lives here.
   Work lands here first; there is no "sync back to the source repo."
2. **The old per-component repos are import provenance, not homes.** Each was
   snapshot-imported (no git history; SHAs recorded in
   `docs/monorepo-migration-log.md` and `scripts/import-sync.toml`). After
   cutover they are archived read-only with a README pointer here. If a stray
   commit lands on one before it is archived, `scripts/import-sync <name>`
   merges it in safely.
3. **Releases are component-scoped tags on this repo**: `finitechat/vX.Y.Z`,
   `fsite/vX.Y.Z`, `fbrain/vX.Y.Z`, plus dispatch-versioned images
   (`finite-agent-runtime`, `finite-saas-core`, `finite-saas-dashboard`,
   `finite-private-limiter`). One repo, many independently versioned
   artifacts. Release asset names are product contracts — never rename them.
4. **Release-URL compatibility is honored until deliberately retired.**
   Agents in the field install from the legacy repos'
   `releases/latest/download/...` URLs. Release workflows mirror assets to the
   legacy repos (via `RELEASE_MIRROR_TOKEN`). A legacy repo is archived only
   after its mirror is no longer needed — or kept unarchived solely as a
   release mirror (archived repos cannot receive releases).
5. **`infra/` is the single deploy root.** Nothing is built on a prod box;
   images are CI-built and digest-pinned; deploys are scripts/runbooks in this
   tree. See `infra/README.md`.
6. **This repo is public.** No secret values, ever — names and locations only.
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
  per repo, so they cannot fold into mono even though mono is public. They
  stay thin: their inputs (image digests, configs) are produced and pinned by
  mono CI. See `infra/tinfoil/README.md`.
- **finite-fable** — Paul's meta/strategy notes; not a git repo by design.
- **Spikes and stale checkouts** (hermes-agent forks, darkmatter, finitesmol,
  finitechat-old, finite-site, …) — archive aggressively; nothing imports them.

## For agents (human and AI) working in the old workspace

If you are reading a checkout of a pre-mono repo: stop and check whether the
work belongs in finite-mono. The old `dev/finite/AGENTS.md` orientation file
now points here. Per-repo checkouts remain useful only for reading history
and for emergency fixes to already-released artifacts before their mirror
lane exists.
