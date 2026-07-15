# Production Baseline — Sites and Agent Runtime Rollout

Status: KNOWN GOOD / IN PRODUCTION (2026-07-15)

This is the regression baseline for the first training cohort. It records what
was actually exercised in production; it does not expand recovery or privacy
claims.

## Deployed checkpoint

- finite-mono revision: `0a770799a4deb09bc31b51714797e0f07f1547d4`
- NixOS system:
  `/nix/store/dhmqlv426f43r7kxmp6i7vi26ncl4f1m-nixos-system-finite-lat-1-25.11.20260630.b6018f8`
- Agent Runtime artifact: `finite-agent-runtime-2026-07-15.2`
- Agent Runtime image:
  `ghcr.io/finitecomputer/agent-runtime:2026-07-15.2@sha256:a0121a343385c278c3fb87388f64f9e774d100cf26bd1a3aad55504b8050021b`
- Dashboard image digest:
  `sha256:8e81f701ef42d0cc6b07880cd2640dc486dc6d62dd7675d73576e00d89ec9234`

No secret values belong in this record.

## Production acceptance evidence

- A fresh Agent launched on the promoted artifact, opened on Agent Overview
  instead of the unready Chat route, reached Online, and completed real Hosted
  Web Device chat.
- The fresh Agent published a private Finite Site from chat. The authenticated
  requesting-user Principal reached Sites, and dashboard Preview rendered the
  private site without a magic-link detour.
- Telegram used `dm_policy: pairing`; an unknown DM received a pairing code and
  the dashboard completed approval after the connection mutation timeout was
  raised above the server-side transition window.
- Sites Canary, Chester, MyDearPaul, and Boss were rolled serially to the exact
  `2026-07-15.2` digest. Each retained its Agent Principal and durable `/data`
  mount and returned a healthy contact endpoint after the rollout.
- The deploy-integrated rollout command was exercised again against that
  cohort. It bound the plan to source host `finite-lat-1`, recognized every
  member as already current, performed zero mutations, and exited cleanly.
- Local and CI gates passed: Rust workspace, dashboard lint/test/browser/build,
  Hermes bridge, skills/search/runtime contracts, rollout contract tests, and
  the devfinity services-only smoke.

Waffle is not part of the healthy baseline. Its canonical compute is absent;
its retained durable state was not modified or manually reconstructed.

## Deploy vocabulary and rollout boundary

- **Deploy infrastructure only — no bot rollout:** switch the requested lat1
  revision and leave Agent Runtime artifacts unchanged.
- **Deploy and roll healthy bots:** switch lat1, plan an exact promoted Runtime
  artifact, preflight every canonical container, and roll the named cohort
  serially.
- **Plan the bot rollout only:** produce the exact cohort and skip reasons
  without executing it.
- A broad rollout requires an explicit canary. Named rollouts may explicitly
  exclude a stale or intentionally retained Agent.

Plain “deploy” means no Agent Runtime rollout. Runtime rollout is always an
additional explicit authorization.

## Baseline that future changes must preserve

Before admitting a runtime/image/dashboard change beyond local development:

1. Run the focused component tests and the repository integration gates.
2. Launch a disposable Agent on the proposed artifact and prove Overview,
   chat, Finite Sites private Preview, and any changed connection flow.
3. Roll one disposable canary before a named healthy cohort.
4. Verify exact image digest, one writable durable `/data` mount, stable Agent
   Principal, and contact health after every rollout.
5. Stop on the first failed Runtime operation. Do not remove provider compute
   manually or weaken the canonical-container preflight.

PR #70 may build on this baseline, but merging it must not silently change any
of these accepted behaviors.

## Proposed recovery run after cohort training

Backup/restore and generic relaunch are deliberately a separate run because
they change Recovery Set and lifecycle behavior. No part of this checkpoint
implements or deploys them.

Propose the follow-up with this narrow order:

1. Configure per-Agent encrypted, off-host, periodic backups of the full
   durable `/data` root using the image's existing Restic contract. Alert on
   failed or stale backups.
2. Prove an application-consistent restore onto an empty isolated target while
   outbound side effects and the retained source Runtime are fenced.
3. Add a read-only reconciliation alert for an active/stale Core Runtime whose
   canonical provider container is absent.
4. Only after the restore proof, design one generic Core-bound relaunch of
   missing compute from retained state. It must bind the exact RuntimeSpec,
   artifact, secret references, source identity, and expected Agent Principal;
   it must fail closed on ambiguous state.

Do not use Waffle as the implementation vehicle or reconstruct its container
by hand. Use synthetic/disposable state first, name the backup and rollback
boundary, and require separate production-mutation authorization.

## Known lifecycle limitation

The dashboard contains an operator removal flow, and successful Runtime
Retirement would remove compute, hide the hosted Project, revoke its runtime
credentials, and retain rows plus durable state. Core policy and every current
Runner intentionally advertise Runtime Retirement as false, so the flow is
unavailable. Sol 2, Waffle, AEON Canary, and Sites Canary were therefore not
removed during this checkpoint. Do not bypass that safety gate with direct
container or database changes.
