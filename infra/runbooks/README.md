# Runbooks

Operational procedures for everything Finite runs. Host facts live in
`infra/hosts/<name>/` (captured 2026-07-08, authoritative); these runbooks
reference them rather than duplicate them. This repo is public: **no secret
values, ever** — env var names and locations only (`infra/README.md`,
secrets policy).

Every runbook states PRECONDITIONS, STEPS, VERIFY, ROLLBACK. Steps that have
not been exercised yet are marked `TODO:` with what must be learned.

> **Topology changed 2026-07-09**: the lat1 consolidation cutover
> ([lat1-nixos-reinstall.md](lat1-nixos-reinstall.md)) moved Core, dashboard,
> Postgres, chat, and sites all onto finite-lat-1 (now NixOS); lat2 is the CI
> runner box. The deploy-core / deploy-sites / deploy-finitechat-server
> runbooks below still describe the PRE-cutover topology (k3s on lat1, sites on
> lat2, chat on clawland) — TODO: fold them into the NixOS/`nixos-rebuild`
> model. The reinstall runbook and the NixOS config (`infra/nixos/`) are the
> current source of truth for lat1.

## Index

| Runbook | Covers |
|---|---|
| [lat1-nixos-reinstall.md](lat1-nixos-reinstall.md) | **Rebuilding / recovering lat1** (NixOS) — the cutover procedure + the mdadm / NIC-by-MAC / ACME gotchas |
| [release-cli.md](release-cli.md) | Cutting finitechat / fsite / fbrain releases (component tags, rolling aliases, field-install verify) |
| [postgres-backup-restore.md](postgres-backup-restore.md) | **The restore drill** for lat1 Postgres — highest-priority runbook in this tree |
| [deploy-finitechat-server.md](deploy-finitechat-server.md) | Chat server + the single-writer doctrine (now on lat1) |
| [deploy-core.md](deploy-core.md) | finite-saas-core + dashboard (PRE-cutover k3s flow — see banner) |
| [deploy-sites.md](deploy-sites.md) | finitesitesd / fsite (PRE-cutover lat2 flow — see banner) |
| [deploy-brain.md](deploy-brain.md) | finite-brain on smoke (still there; deferred from the cutover) |
| [runtime-image.md](runtime-image.md) | Building and promoting the agent runtime image |
| [break-glass.md](break-glass.md) | Getting on each box, logs, restarts |

## Release checklist discipline

Two rules apply to **every** release and promotion, no exceptions:

1. **Every release updates `compat/matrix.toml` in the same PR/commit.**
   The matrix records what is already out in the field (installed CLIs,
   pinned runtime images, deployed servers). Stranding a fielded artifact
   must be a deliberate, reviewed act — an edit to that file — never an
   accident. Each runbook below has this as an explicit step.

2. **Rung-ladder: local proof → Docker proof → Phala/Tinfoil promote.**
   Nothing is promoted to a confidential-compute lane without a recorded
   proof at the rung below it. This is the champagne-test discipline encoded
   in `.github/workflows/hermes-runtime-smoke.yml` (publication of the
   runtime image is gated on the smoke report) — apply the same ladder
   manually anywhere a workflow does not enforce it yet. Concretely:
   - local: devfinity / `cargo test` / local smoke scripts pass;
   - Docker: the relevant Docker smoke lane passes and its report artifact
     is kept;
   - only then: promote the digest to Phala (runner env / Core artifact) or
     hand off to a Tinfoil satellite repo (`infra/tinfoil/README.md`).

## Standing rules

- Nothing is built on a prod box. Images are CI-built, digest-pinned, from
  `infra/images/` (`infra/README.md` deploy principles).
- Backups are only real once restored. The Postgres restore drill
  ([postgres-backup-restore.md](postgres-backup-restore.md)) has never been
  run — run it before trusting anything else here.
- Any manual change made on a box during an incident must land back in
  `infra/` (or be reverted) **within a day** — see
  [break-glass.md](break-glass.md).
