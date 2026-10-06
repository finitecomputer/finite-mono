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
separate Nix feature toggle. The private Core listener uses port 4202 on
loopback (see the [port map](../nixos/README.md#port-map-consolidated-box)).

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
  distinct from `FC_CORE_BIND` and `FC_CORE_RUNTIME_BIND`: `127.0.0.1:4202`
  on lat2. Never a wildcard or public address. Never add a Caddy
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

Brain, in `/etc/finite/brain-identity.env` (optional, root-owned `0600`;
`finite-brain-app.service` loads it only if present):

- `FINITE_BRAIN_CORE_IDENTITY_URL`: `http://<FC_CORE_BRAIN_IDENTITY_BIND>`.
- `FINITE_BRAIN_CORE_DESCRIPTION_TOKEN`: the Core description token.

Brain sends `FINITE_BRAIN_PUBLIC_BASE_URL` as its identity; it must equal the
Core Brain server value. Record each new secret in the secrets inventory by
name and location only.

## Rollout order

Core, Brain and the digest-pinned dashboard image ship together in one lat2
NixOS closure ([deploy-core.md](deploy-core.md#steps)). Deploy them with every
optional setting unset, then switch each part on separately.

1. Run `scripts/finite-status` and keep the output.
2. Build and download the reviewed `origin/main` revision's
   `lat2-nixos-closure-REV` artifact, then run
   `just deploy-lat2-closure "$ARTIFACT_DIR" --prepare`, `scripts/finite-status`,
   and `just deploy-lat2-closure "$ARTIFACT_DIR" --activate`. This applies
   Migration 0038 (Core) and SCHEMA_V30 (Brain), both additive, and runs the
   new dashboard image with the Join/Approve disclosure text and the hook
   still off. Tabs loaded before the new image do not send
   `shareAccountContact`; their Joins and Approvals still work and never
   record sharing.
3. Verify Core, Brain and dashboard health as in deploy-core.md and
   deploy-brain.md, then run `scripts/finite-status --brain-identity`. Expect
   `schema_present: true`. On first introduction associations, scopes and
   receipts are zero; after a rollback and re-enable, earlier rows are kept
   and counts start from them. Read the `agent_keys` counters (see below)
   before going further.
4. Add the four Core variables to `/etc/finite/core.env` and run
   `systemctl restart finite-saas-core.service`. Confirm the listener log line
   and that both routes return 404 on the main (4200) and runtime (4201)
   listeners.
5. Create `/etc/finite/brain-identity.env` and run
   `systemctl restart finite-brain-app.service`.
6. Add the two dashboard variables to `/etc/finite/dashboard.env` and run
   `systemctl restart podman-finite-saas-dashboard.service`.
7. From an ordinary admin session on a designated Brain, run
   `fbrain access list --brain <exact-id>` with `fbrain` 0.7.0 and keep the
   output as the read-only qualification. Before the public release, the
   operator may use a locally built 0.7.0 binary from the reviewed revision
   whose checksum they verified; never build on a production server. Then
   run `scripts/finite-status` again.

`fbrain` 0.7.0 is published through the normal CLI release
([release-cli.md](release-cli.md)) after the closure is live. Do not upgrade
Agent Runtimes for this feature.

## Descriptions v2 (FIN-166)

A closure with descriptions v2 changes no setting. New Core serves v1 and v2;
new Brain asks v2 and falls back to v1 only on Core's exact unsupported-version
answer. Either half alone keeps the v1 policy, so the order inside one closure
does not matter. After activation, participating keys whose Account never
shared with that Brain are described: an Agent's name and owner email, or a
hosted human's email. This disclosure cannot be recalled by a rollback; see the
accepted risk in the Core contract.

1. Run `scripts/finite-status` and `scripts/finite-status --brain-identity`
   and keep the output.
2. Deploy the closure as in "Rollout order" step 2 and verify health.
3. From an ordinary admin session on a designated Brain, run
   `fbrain access list --brain <exact-id> --json`. Expect `resolved` rows for
   participating keys without a sharing scope, `noParticipation` for keys that
   never acted, and owner `humanPublicKeysHex` limited to keys on that page.
4. Run both `scripts/finite-status` commands again.

To return to the v1 policy, roll back the closure (Core or Brain alone is
enough). To stop all descriptions, remove `/etc/finite/brain-identity.env` as
in Rollback.

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

Remove the dashboard variables, then `/etc/finite/brain-identity.env`, then
the Core variables, and restart each unit. Every part goes back to off, and
access reports keep every row. A closure rollback per deploy-core.md also
returns the previous dashboard image. The base Core
and Brain binaries reopen the additive schema; the new tables and indexes stay
in place and are ignored. They are part of the existing Core Postgres dump and
Brain SQLite Recovery Set; restoring them restores descriptions, not keys.

## Known limits

- Hosted Agents keep the `fbrain` in their pinned Runtime until that Runtime
  is upgraded separately. Their `access list` is the older summary; the
  managed FiniteBrain skill tells them to check the report `version` and say
  a newer CLI is needed rather than claim a complete report.
- Only hosted human actions (Join, applied Approve) record sharing. No current
  flow emits an owned-agent observation. Under v1 an agent whose owner never
  acted as a hosted human in that Brain stays `notShared` there; v2 needs no
  sharing.
- Scope revocation and project owner transfer have no product writer. The
  tests change those rows, and account emails, directly in synthetic state to
  prove the reader. No email-update or owner-transfer flow is added. Under v2
  an owner-transfer writer must not ship until it stops pre-transfer
  participation from releasing the new owner; a Core test enforces this.
- Core returns no NIP-05: a reserved agent name is not publication evidence.
  Brain shows its own stored aliases separately, dated and not rechecked.
- Keys that never acted in a Brain are never sent to Core, so admin-added keys
  stay undescribed until they act.
