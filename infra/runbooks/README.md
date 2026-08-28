# Runbooks

Operational procedures for everything Finite runs. The 2026-07-08 files under
`infra/hosts/<name>/` are dated captures and may be historical. Current fleet
roles live in `infra/README.md`; executable NixOS configuration is authority
for declared NixOS state; a fresh read-only inventory is authority for physical
state. These runbooks must name which source they rely on rather than silently
promoting an old capture. Source privacy is not a secret boundary: **no secret
values, ever** — env var names and locations only (`infra/README.md`, secrets
policy).

Every runbook states PRECONDITIONS, STEPS, VERIFY, ROLLBACK. Steps that have
not been exercised yet are marked `TODO:` with what must be learned.

> **Topology as of the 2026-07-09 lat1 consolidation cutover**
> ([lat1-nixos-reinstall.md](lat1-nixos-reinstall.md)): Core, dashboard,
> native Postgres, chat, sites, and Brain all run on finite-lat-1 (now NixOS,
> one Caddy edge, no k3s); production Nix closures are built by the
> Depot-backed `Lat1 NixOS Closure` artifact workflow. Lat2 is pending
> archive/offload and decommission, not ongoing fleet capacity. Docker/image CI
> also uses Depot; smoke is the Brain rollback source during migration; clawland is legacy.
> **The topology runbooks below
> (deploy-core / deploy-sites / deploy-finitechat-server /
> postgres-backup-restore / break-glass) are NOW UPDATED to that reality.**
> The NixOS config (`infra/nixos/`) declares lat1; live inventory proves its
> physical state. The reinstall file is historical cutover evidence, not
> current destructive authority.

## Index

| Runbook | Covers |
|---|---|
| [lat1-nixos-reinstall.md](lat1-nixos-reinstall.md) | **Historical 2026-07-09 lat1 cutover evidence** — destructive reuse is paused; retain its MD / NIC-by-MAC / ACME findings while the finite-lat-3 plan produces a replacement |
| [release-cli.md](release-cli.md) | Cutting finitechat / fsite / fbrain releases (component tags, rolling aliases, field-install verify) |
| [postgres-backup-restore.md](postgres-backup-restore.md) | **The restore drill** for lat1 native Postgres — highest-priority runbook in this tree |
| [hosted-web-chat-recovery.md](hosted-web-chat-recovery.md) | Coordinated Hosted Web Device + Finite Chat + SaaS Core snapshot and empty-target drill |
| [litestream-chat-replication.md](litestream-chat-replication.md) | Continuous chat + Brain SQLite replication to Latitude object storage + restore drill (DR-only) |
| [chats-appear-missing.md](chats-appear-missing.md) | Read-only-first continuity incident diagnosis; never creates replacement state |
| [platform-rollout.md](platform-rollout.md) | **The manual cross-component wave** — ordering (runners before Core), gates, layered verify ritual, and lifecycle-rollback sequencing across the deploy runbooks below |
| [production-cd.md](production-cd.md) | Protected GitHub Actions production deploy bootstrap, setup verification, enablement, and first deploy flow |
| [deploy-core.md](deploy-core.md) | finite-saas-core + dashboard on lat1 (NixOS: systemd core + podman dashboard, `nixos-rebuild`) |
| [deploy-sites.md](deploy-sites.md) | finitesitesd on lat1 (NixOS `nixos-rebuild`; flags the KATA / `--app-runner none` gap) |
| [deploy-finitechat-server.md](deploy-finitechat-server.md) | Chat server on lat1 (:8788) + the single-writer doctrine |
| [deploy-brain.md](deploy-brain.md) | finite-brain on lat1 at `brain.finite.computer`, with the dashboard-embedded WorkOS client; SQLite migration and rollback |
| [decommission-lat2.md](decommission-lat2.md) | Hard-cut offload, runner removal, credential revocation, and wipe/release procedure for finite-lat-2 |
| [stripe-billing.md](stripe-billing.md) | Live Stripe readiness, webhook/Core reconciliation, dunning, cancellation/refund, and secret rotation |
| [runtime-image.md](runtime-image.md) | Building and promoting the agent runtime image for the Kata runner on lat1 |
| [finite-private-deepseek-production-update.md](finite-private-deepseek-production-update.md) | Guarded promotion of the measured DeepSeek 128/2048 scheduler and canonical model label, with exact current DeepSeek rollback |
| [finite-private-routing-migration.md](finite-private-routing-migration.md) | Staged migration from the historical Kimi container/hostname to the stable `finite-private` identity without breaking issued Runtime readers |
| [runtime-cold-relocation.md](runtime-cold-relocation.md) | Operator-only stopped Kata Runtime move between exact hosts, with state-manifest and Agent Principal fencing |
| [legacy-hermes-box1-to-lat3.md](legacy-hermes-box1-to-lat3.md) | Versioned, identity-fenced migration of one box1 Hermes bot into a new lat3 Runtime; Austin is the first canary |
| [finite-private-limiter-mono-switch.md](finite-private-limiter-mono-switch.md) | Planned-downtime switch from the legacy limiter image to a mono-built limiter plus upstream GLM 5.2 v0.0.17 |
| [phala-confidential-runner.md](phala-confidential-runner.md) | Dark, separately fenced Phala worker and API-only preflight/lifecycle/recovery/inventory/cost procedures; no CLI or delete path |
| [break-glass.md](break-glass.md) | Getting on each box, logs, restarts (lat1 NixOS, lat2 decommission target, smoke rollback source, clawland legacy) |

## Release checklist discipline

Two rules apply to **every** release and promotion, no exceptions:

1. **Every release and promotion edits exactly one source of truth.** What
   is out in the field is read from where it is pinned, never from a copy:
   source tags and the rolling alias releases for the CLIs; Core's promoted
   runtime-artifact record plus
   `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `/etc/finite/runner.env` on each Kata
   host for the Agent Runtime image (existing Agents keep their launch-time
   image); the NixOS closure — `infra/nixos/modules/dashboard.nix` for the
   dashboard digest — for everything on lat1. Stranding a fielded artifact
   must be a deliberate, reviewed act in that source, never an accident.
   Anything the source cannot express (why a version shipped, when a fleet
   roll completed, a live compatibility promise) goes in
   [`../deployment-changelog.md`](../deployment-changelog.md). There is no
   hand-maintained ledger to keep in sync; the old one was retired on
   2026-08-21 and `just runbook-facts-contract` fails any runbook that
   reinstates it.

2. **Rung-ladder: local proof → Docker proof → Kata → Phala/Tinfoil.**
   Nothing is promoted to a confidential-compute lane without a recorded
   proof at the rung below it. `.github/workflows/runtime-image.yml` builds the
   canonical image once, smokes that immutable local image ID, then publishes
   those exact bytes. The separate `.github/workflows/hermes-runtime-smoke.yml`
   rebuild is optional source preflight, never publication or exact-image
   promotion evidence. Concretely:
   - local: devfinity / `cargo test` / local smoke scripts pass;
   - Docker: the relevant Docker smoke lane passes and its report artifact
     is kept;
   - only then: publish once and promote the digest to Kata/Phala or
     hand off to a Tinfoil satellite repo (`infra/tinfoil/README.md`).

## Standing rules

- **One status command:** run `scripts/finite-status` before and after every
  rollout. Use `scripts/finite-status --json` for retained evidence or
  automation. It is read-only and exits `0` when all four sections are green,
  `1` when any section is red, and `2` when a result could not be determined
  and nothing is already red. Fleet convergence is explicitly Core-recorded
  state plus heartbeat age, not proof of live provider compute. Inactive
  `project_runtime_links` are shown but excluded from drift.

  The command observes the last `finite-healthcheck` systemd invocation for
  service/HTTP health, verifies the sealed recovery manifest only by SHA-256,
  uses the Borg job result and its canonical success stamp, and summarizes the
  latest local `.local-state/runtime-rollouts` event stream. It never opens a
  snapshot SQLite database as a database and never runs a mutating provider,
  Core, systemd, or Borg operation. If an incident tempts you to type an ad-hoc
  probe, that probe becomes a PR to `finite-status` and its contract test.

  Until the reviewed revision is installed on lat1 through the normal NixOS
  deployment, run it from a read-only checkout on the host or collect its
  output after an operator installs the two `scripts/finite-status` and
  `scripts/finite_status.py` files together. Once installed, remote observation
  is simply `ssh -T root@64.34.82.77 finite-status --json`. Adding a systemd
  timer/page-on-red policy and applying the revision to production are explicit
  operator steps, outside the implementation PR.

  `finite-status` also reports per-Agent lifecycle-control health from the
  runner's read-only `lifecycle-probe` (app health and lifecycle health are
  separate fields; lifecycle verdicts gate upgrade eligibility only). The
  probe reads root-only Kata/containerd state — `/run/vc/sbs/*/persist.json`,
  `ctr --namespace finite tasks list`/`tasks ps`, `/var/run/netns`, and
  `/proc/<pid>/comm` — so the collector must run as root (as in the `ssh -T
  root@…` pattern above). Run as an unprivileged user, each Agent's lifecycle
  field degrades to a displayed `unknown`, never a silent green. It issues no
  mutating provider, Core, systemd, or Borg operation.

- Nothing is built on a prod box. Images are CI-built, digest-pinned, from
  `infra/images/` (`infra/README.md` deploy principles).
- Never open a snapshot SQLite file directly. Use
  `scripts/snapshot-sqlite`, which inspects a private scratch copy and leaves
  the sealed snapshot untouched.
- Rust service packages use content-scoped sources in
  `infra/nixos/packages.nix`: the root `Cargo.lock`, a generated root workspace
  manifest listing only the package's selected workspace members, and only the
  binary's transitive local crate directories plus explicitly embedded assets.
  Crane builds dummy-source dependency artifacts and feeds them into the
  real-source application builds. Related binaries share three product-family
  artifacts: `finite-saas` (`finite-saas-core`, `finite-saas-local`, and
  `finite-saas-runner`), `finitechat` (`finitechat-server`,
  `finitechat-hosted-device`, `finitechat`, and `finitechat-rmp`), and
  `finite-brain` (`finite-brain` and `fbrain`). Other packages retain their own
  artifact. A grouped artifact prebuilds its members in separate Cargo
  invocations so it retains each package's exact feature resolution instead of
  a feature-unified graph that the final builds cannot reuse. Every final
  package keeps its narrower source closure, so an ordinary Rust source edit
  must leave
  `.#packages.x86_64-linux.<package>.cargoArtifacts.drvPath` unchanged; manifest,
  lockfile, build-input, or dependency-group membership changes intentionally
  invalidate it. Inspect `<package>.cargoArtifactGroup` to identify the shared
  boundary; packages in one group must expose the same `cargoArtifacts.drvPath`.
  When a package gains a path dependency or an
  `include_str!`/`include_bytes!` input outside those crate directories, add
  that path to its `sourcePaths` in the same change. Do not add unrelated
  workspace members or fall back to the full flake source. Shared dependency
  groups must remain product-scoped: do not widen a group merely because two
  packages happen to use some of the same third-party crates. The `Nix service
  packages` CI lane builds every distinct dependency artifact before its scoped
  packages and explicitly includes both closures in trusted Cachix pushes; its
  job summary reports whether each phase required a build and how long it took.
  The lane must pass before rollout.
  For a supposedly component-only change, compare the clean base and candidate
  outputs with `nix path-info .#packages.x86_64-linux.<package>`; an unrelated
  package path change is a stop condition and a source-scoping bug.
- Backups are only real once restored. The coordinated Hosted Web Chat and
  Postgres empty-target drills have not yet passed. The accepted July 20 plan
  does not authorize additional paid or Launch Code Agent creation before its
  Phase 11; this docs edit does not claim the Core/UI gate is already deployed.
  The off-host repository and verified first archive exist; do not regress
  their health or mistake them for a completed drill.
- Any manual change made on a box during an incident must land back in
  `infra/` (or be reverted) **within a day** — see
  [break-glass.md](break-glass.md).
