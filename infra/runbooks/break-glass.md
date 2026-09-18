# Break-glass: getting on the boxes

For incidents. Host facts (services, ports, secrets locations) live in
`infra/hosts/<name>/README.md` — read the host README before touching a box.

> **The rule:** any manual change made on a host must land back in `infra/`
> (or be reverted) **within a day**. The whole value of this tree is that it
> matches reality; an undocumented hotfix is drift, and drift is how the
> pre-mono mess happened. Note the change in your PR even if it is embarrassing.

## lat1 — retired (`64.34.82.77`)

Preserve retained disks and state. Use SSH or provider rescue/IPMI only for
explicitly authorized diagnosis. Do not start old writers or point DNS here.
Recovery requires a verified Recovery Set, not reactivating a stale host.

## lat2 — finite-lat-2 (64.34.80.19) — app plane

lat2 is the live single app server (no Agent Runner) and
the `wg-finite` overlay hub at `10.254.3.1`.

- **Get on:** `ssh root@64.34.80.19`. Declarative NixOS:
  fix forward in `infra/nixos/hosts/finite-lat-2/` and redeploy via
  `just deploy-lat2-closure`; roll back with `nixos-rebuild switch
  --rollback`.
- **lat1 is the frozen point-in-time record.** Do not boot it back into
  service; a second writer on Postgres/chat/sites is split-brain.

## Legacy hosts

smoke is `15.204.56.61`; clawland is `15.204.108.57`. Consult their host README
and current read-only evidence. They are not app-plane rollback targets. Do not
run old deploy transcripts or build production artifacts on them.

## Incident discipline

Run `scripts/finite-status` before and after an authorized change. Preserve the
failed state and recovery evidence. Manual production changes require explicit
authorization and must be reflected in reviewed configuration or reverted.
