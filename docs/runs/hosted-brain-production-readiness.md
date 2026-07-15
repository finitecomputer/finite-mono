# Hosted Brain Production Readiness

Status: PROPOSED
Owner: Paul
Opened: 2026-07-15
Acceptance: a signed-in user can open the production Brain UI from an Agent,
pair that Agent by its canonical managed email, write inside its bounded Agent
Workspace Folder, and read the result as the Personal Vault owner after a
service restart.
Expiry: 2026-07-22; stop and rescope if the identity, release, or durable-data
boundary changes before activation.

This draft has no work authority. Brain remains disabled in dashboard
navigation until Paul activates this run and its acceptance passes.

## Problem statement

PR #70 merged the hosted Brain integration code, but the production product is
not ready to expose:

- GitHub Releases does not yet provide a current `fbrain` binary matching the
  merged implementation.
- The production dashboard Brain UI does not match the current UI Austin has
  validated locally.
- lat1 does not run the Finite Identity Authority required for canonical
  managed Agent Email registration and email-first Brain pairing.
- Core, Runner, Brain, dashboard, and the Identity Authority have not yet been
  deployed and accepted together as one production revision.

The direct Brain route and proxy code remain in place for development and
integration testing. The Agent sidebar entry is disabled so users do not enter
an incomplete production flow.

## Constraints

- Reuse the merged Finite Identity and Brain contracts; do not introduce a
  second identity store, pairing protocol, or product-specific email resolver.
- Choose and document one canonical Identity Authority origin before deploy;
  reconcile the current `identity.finite.chat` configuration examples with the
  `identity.finite.vip` operations example.
- Keep Identity Authority operator credentials on trusted services only. They
  never enter an Agent Runtime.
- Treat the Identity Authority SQLite directory and Brain SQLite database as
  durable production state. Name backups and rollback boundaries before any
  production mutation.
- Do not re-enable Brain navigation merely because services are healthy. The
  acceptance flow below must pass on the exact deployed revisions.

## Proposed queue

1. Land Austin's current Brain Product Client UI in finite-mono and prove the
   dashboard iframe/proxy flow locally.
2. Publish a component-scoped `fbrain` release from the exact reviewed mono
   revision and verify the rolling `fbrain-latest` assets and installer.
3. Add the existing `finite-identityd` to the lat1 NixOS deployment with a
   loopback listener, Caddy route, root-only environment, production mailer,
   operator token, durable state directory, health check, backup, and rollback
   procedure.
4. Configure Runner with the Identity Authority URL and operator token; configure
   Brain with the public authority URL. Deploy matching Core, Runner, Brain,
   dashboard, and Caddy configuration from one reviewed revision.
5. Launch a disposable Agent and prove its canonical managed email binds
   immutably to its retained Agent Principal. Stop on a mismatch or failed
   authority call; do not bypass the fail-closed launch contract.
6. Run the hosted Brain acceptance flow: initialize one Personal Vault, pair
   the disposable Agent by managed email, grant only its Agent Workspace
   Folder, write as the Agent, read as the owner, restart services, and repeat
   the readback.
7. Re-enable the Brain sidebar link in a separate small dashboard change, build
   and deploy its digest-pinned image, and repeat the production browser flow.

## Evaluation and rollback

- Local gates: Identity Authority tests, Brain CLI/server tests, dashboard
  tests/browser/build, and `just dev smoke`.
- Production evidence: exact Git revisions and image digests, healthy public
  authority and Brain endpoints, one immutable Agent Email binding, bounded
  Folder membership, owner readback, and restart persistence.
- Before deployment, take consistent backups of Identity Authority and Brain
  state and record their hashes outside database contents. A NixOS rollback is
  not a data rollback; preserve both sides if either service accepts writes.
- Fail closed on an unavailable authority, mismatched email/principal binding,
  ambiguous Vault ownership, broader-than-Folder access, or UI/release revision
  mismatch. Keep Brain navigation disabled and return to the last known-good
  system generation.

## Acceptance Request (to complete when ACTIVE)

- **Revision:** exact deployed mono revision, `fbrain` release, dashboard image,
  and NixOS system closure.
- **Where:** `https://finite.computer` using a disposable Agent owned by the
  designated acceptance account.
- **Time:** 10 minutes.
- **Steps and observations:** open Brain from the Agent sidebar; confirm the
  reviewed Product Client UI; pair by canonical Agent Email; create and read an
  Agent-folder document; restart the named services; read it again as owner.
- **Pass:** the same Agent Principal retains access only to its dedicated Folder
  and the owner retains the Personal Vault and post-restart data.
- **Fail/stop:** capture read-only service health, deployed revisions, binding
  inspection, and Brain authorization output; keep navigation disabled and do
  not rewrite identity or Vault state.
