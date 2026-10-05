# Brain identity descriptions

Enables the named Brain access report to show who an exact key belongs to:
a hosted human's account email, or a hosted agent's name, lifecycle and
responsible account (FIN-122). Contracts:
[Core](../../finitecomputer-v2/docs/brain-identity-descriptions-v1.md) and
[Brain report](../../finite-brain/docs/brain-access-report-v1.md).

Shipping the new binaries and image does change some things: the access
report route, the `fbrain access list` report and `access summary`, the
Join/Approve card wording, and the additive Core and Brain schemas all ship
with them. Only the optional connections are off until the variables below
are set: the dashboard observation hook, Core's private listener and Brain's
description client. This runbook does not authorize a deploy, a backfill or
any data repair; each needs its own explicit approval.

## Where it runs

Core, the dashboard, Hosted Device and Brain all run on the lat2 app plane.
The authority is [`infra/nixos/hosts/finite-lat-2`](../nixos/hosts/finite-lat-2/);
older Core and Brain deploy notes that name lat1 are stale for this feature.
All three connections stay on lat2 loopback.

`fbrain` ships as a normal CLI component release. Hosted Agent Runtimes do not
need a runtime-image upgrade for this feature: existing CLIs keep working
against the new Brain, and only admins who run `fbrain access list` need the
new CLI. Do not roll the agent fleet for it.

Optional settings live in root-owned environment files so each part can be
switched on or off by editing one file and restarting one unit; there is no
separate Nix feature toggle.

## What each part does

| Part | Role | Off when |
| --- | --- | --- |
| Core | Private listener with two routes: trusted hosted observations (writer) and scoped exact-key descriptions (reader) | Any of its four variables is unset or invalid |
| Dashboard | After a Join or Approve that the Brain server confirmed, tells Core which existing hosted key acted and that the account shares contact with that Brain's admins | Either of its two variables is unset |
| Brain | Asks Core to describe participating report keys | Either of its two variables is unset or invalid |

Descriptions never grant or remove access. If Core is off, old or down, every
access row still appears and descriptions read `unavailable`.

## Configuration (names and locations only)

Core, in `/etc/finite/core.env` (loaded by `finite-saas-core.service`):

- `FC_CORE_BRAIN_IDENTITY_BIND`: a loopback or private address with a port,
  distinct from `FC_CORE_BIND` and `FC_CORE_RUNTIME_BIND`, for example
  `127.0.0.1:4202`. Never a wildcard or public address. Never add a Caddy
  route for it.
- `FC_CORE_BRAIN_IDENTITY_BRAIN_SERVER`: the canonical Brain origin, exactly
  `https://brain.finite.computer` (no trailing slash).
- `FC_CORE_BRAIN_OBSERVATION_TOKEN`: new secret shared only with the
  dashboard.
- `FC_CORE_BRAIN_DESCRIPTION_TOKEN`: new secret shared only with Brain.

The two tokens must differ from each other and from every existing Core
credential. Partial or invalid settings, or a bind failure, disable only this
listener with a warning naming the variable; Core keeps serving.

Dashboard, in `/etc/finite/dashboard.env`:

- `FC_CORE_BRAIN_IDENTITY_URL`: `http://<FC_CORE_BRAIN_IDENTITY_BIND>`.
- `FC_CORE_BRAIN_OBSERVATION_TOKEN`: the Core observation token.
- `FC_BRAIN_PUBLIC_ORIGIN` must already equal the Core Brain server value.

Brain (`finite-brain-app.service`): the unit currently loads only the shared
mail environment file. Enabling needs a separate infra change adding an
optional root-owned environment file (proposed `/etc/finite/brain-identity.env`,
loaded only if present) with:

- `FINITE_BRAIN_CORE_IDENTITY_URL`: `http://<FC_CORE_BRAIN_IDENTITY_BIND>`.
- `FINITE_BRAIN_CORE_DESCRIPTION_TOKEN`: the Core description token.

Brain sends `FINITE_BRAIN_PUBLIC_BASE_URL` as its identity; it must equal the
Core Brain server value. Record each new secret in the secrets inventory by
name and location only.

## Rollout order

1. Run `scripts/finite-status` and keep the output.
2. Deploy Core with the feature unset. Core applies Migration 0038, which is
   additive and reapplied on every start.
3. Run `scripts/finite-status --brain-identity`. Expect `schema_present: true`.
   On first introduction associations, scopes and receipts are zero; after a
   rollback and re-enable, earlier rows are kept and counts start from them.
   Read the `agent_keys` counters (see below) before going further.
4. Deploy the updated dashboard image with its two variables unset. Core's
   deployment does not update the dashboard. The new image brings the Join and
   Approve disclosure text and the hook, which stays off. Tabs loaded before
   the new image do not send `shareAccountContact`; their Joins and Approvals
   still work and never record sharing.
5. Set the four Core variables and restart. Confirm the listener log line and
   that neither route answers on the main or runtime listener.
6. Deploy Brain and `fbrain` (SCHEMA_V30 adds indexes only). Old CLIs keep
   working; `fbrain access summary` is the older view.
7. Set the Brain variables, then the dashboard variables.
8. From an ordinary admin session on a designated Brain, run
   `fbrain access list --brain <exact-id>` and keep the output as the
   read-only qualification. Then run `scripts/finite-status` again.

Existing hosted humans are described only after their next Join or Approve.
Nothing is backfilled.

## Reading the agent counters

`scripts/finite-status --brain-identity` reports counts only.

- `ambiguous_projects` and `foreign_active_link`: Core reports these keys as
  `ambiguous` or `notShared`. Investigate read-only; do not repair.
- `unsupported_lifecycle`: no live link and not fully retired. Described as
  `unknown`, never guessed.
- `same_project_inactive_unoffboarded_sibling`: a project with another
  runtime row that has no active link and no offboarding phase. Current
  relocation keeps one runtime id, so this is legacy state. Such a key
  resolves while its live sibling exists and becomes `unknown` after retirement.

Record the counts before enabling. Any probe for a specific key belongs in
`scripts/finite-status`, not an ad hoc production query.

## Rollback

Unset the dashboard variables, then Brain's, then Core's, and restart each.
Every part goes back to off, and access reports keep every row. The base Core
and Brain binaries reopen the additive schema; the new tables and indexes stay
in place and are ignored. They are part of the existing Core Postgres dump and
Brain SQLite Recovery Set; restoring them restores descriptions, not keys.

## Known limits

- Only hosted human actions (Join, applied Approve) record sharing. No current
  flow emits an owned-agent observation, so an agent whose owner never acted
  as a hosted human in that Brain stays `notShared` there.
- Scope revocation and project owner transfer have no product writer. The
  tests change those rows, and account emails, directly in synthetic state to
  prove the reader. No email-update or owner-transfer flow is added.
- Core returns no NIP-05: a reserved agent name is not publication evidence.
  Brain shows its own stored aliases separately, dated and not rechecked.
- Keys that never acted in a Brain are never sent to Core, so admin-added keys
  stay undescribed until they act.
