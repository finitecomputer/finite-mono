# Brain identity descriptions

Enables the named Brain access report to show who an exact key belongs to:
a hosted human's account email, or a hosted agent's name, lifecycle and
responsible account (FIN-122). Contracts:
[Core](../../finitecomputer-v2/docs/brain-identity-descriptions-v1.md) and
[Brain report](../../finite-brain/docs/brain-access-report-v1.md).

The initial v1 release introduced the access report route, `fbrain access list`
and `access summary`, hosted contact-sharing disclosures, and additive Core and
Brain schemas. Its optional connections stay off until configured: the
dashboard observation hook, Core's private listener and Brain's description
client. Existing configured deployments use the versioned upgrade procedures below;
do not repeat first-time configuration or clear their settings. This runbook
does not authorize a deploy, a backfill or any data repair; each needs its own
explicit approval.

## Where it runs

Core, the dashboard, Hosted Device and Brain all run on the lat2 app plane.
The authority is [`infra/nixos/hosts/finite-lat-2`](../nixos/hosts/finite-lat-2/);
older Core and Brain deploy notes that name lat1 are stale for this feature.
All three connections stay on lat2 loopback.

`fbrain` ships as a normal CLI component release. The server-side descriptions
feature alone does not require a Runtime upgrade. The dashboard roster and
Personal Brain setup do: they use new Runtime plugin routes and CLI commands.
Existing Agents also need the new CLI for the `open personal` selection fix.

Optional settings live in root-owned environment files so each part can be
switched on or off by editing one file and restarting one unit; there is no
separate Nix feature toggle. The private Core listener uses port 4202 on
loopback (see the [port map](../nixos/README.md#port-map-consolidated-box)).

## What each part does

| Part | Role | Off when |
| --- | --- | --- |
| Core | Private listener with two routes: trusted hosted observations (writer) and scoped exact-key descriptions (reader) | Any of its four variables is unset or invalid |
| Dashboard | After an Approve that Brain confirmed, tells Core which existing hosted key acted and that the account shares contact with that Brain's admins. The retained invitation-accept API can also record an explicit sharing request, but has no current Chat UI caller. | Either of its two variables is unset |
| Brain | Sends the report page and its participating subset to Core; only exact eligible keys can receive descriptions | Either of its two variables is unset or invalid |

Descriptions never grant or remove access. Optional Core failures preserve the
access rows. Description availability and legacy fallback follow the protocol
contract; nonparticipants are not silently added to a legacy lookup.

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

## Initial v1 configuration

This describes the original 0.7.0 introduction into an unconfigured deployment.
For an existing v1 deployment, use "Descriptions v2" below instead.

Core, Brain and the digest-pinned dashboard image ship together in one lat2
NixOS closure ([deploy-core.md](deploy-core.md#steps)). Deploy them with every
optional setting unset, then switch each part on separately.

1. Run `scripts/finite-status` and keep the output.
2. Build and download the reviewed `origin/main` revision's
   `lat2-nixos-closure-REV` artifact, then run
   `just deploy-lat2-closure "$ARTIFACT_DIR" --prepare`, `scripts/finite-status`,
   and `just deploy-lat2-closure "$ARTIFACT_DIR" --activate`. This applies
   Migration 0038 (Core) and SCHEMA_V30 (Brain), both additive, and runs the
   new dashboard image with hosted contact-sharing disclosure text and the hook
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
([release-cli.md](release-cli.md)) after the closure is live. That server-only
introduction did not require an Agent Runtime upgrade.

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
2. Deploy Core and Brain using the exact reviewed lat2 closure and verify
   health. Preserve the existing identity configuration. When shipping the
   dashboard roster and Personal Brain setup too, retain the current dashboard
   pin for this first closure activation.
3. For that combined release, qualify the candidate Runtime on a designated
   canary, then upgrade the selected existing Agents using the
   [Runtime rollout procedure](runtime-image.md). The participation filter
   takes effect with the Brain server; the `open personal` fix takes effect
   separately on each Agent's Runtime upgrade.
4. Deploy a second closure with the new dashboard digest after Runtime
   qualification, then run `finite skills sync` for the selected existing
   Agents. The updated skill points to the dashboard setup button, so sync it
   after that button is available. Old Runtimes keep their existing CLI and
   show an update-required or unavailable result for the new dashboard routes.
5. From an ordinary admin session on a designated Brain, run
   `fbrain access list --brain <exact-id> --json`. Expect `resolved` rows for
   participating keys without a sharing scope, `noParticipation` for keys that
   never acted, and owner `humanPublicKeysHex` limited to keys on that page.
6. Run both `scripts/finite-status` commands again.

To return to the v1 policy, roll back the closure (Core or Brain alone is
enough). To stop all descriptions, remove `/etc/finite/brain-identity.env` as
in Rollback.

## Scoped operator catch-up (v3)

The [v3 contract](../../finitecomputer-v2/docs/brain-scoped-roster-v3.md) adds
exact-Brain operator attribution, not global associations. It changes Core,
Brain, the Runtime plugin and existing dashboard cells. No user action is
required. An additive migration does not make activation interruption-free:
the app closure and selected Runtime upgrades still restart services. Use an
approved maintenance window, fresh recovery evidence and the existing deployment
gates whenever those restarts could interrupt users.

1. Freeze the reviewed source/artifact hashes and the private evidence/candidate.
   Rehearse real PostgreSQL apply, preview, races, exact retries, rollback and
   empty-target backup restore. Run actual old/new binaries and all required
   viewer combinations against a complete synthetic report. Resolve review
   findings before producing executable manifests.
2. Refresh platform health, actual backup timer results, recovery snapshot,
   schema, roster roles/grants and identity invariants. Qualify enough rollback
   time; do not overlap active backups or another operator. Build off-host and
   prepare the exact closure through the current deployment procedure.
3. Activate the reviewed Core/Brain/dashboard closure in the selected window.
   Preserve existing identity configuration. Check health and the direct v3
   protocol probe, then actual Brain/admin-viewer v3 coverage. No records are
   applied at this stage. Old Core ignores scoped records; a legacy fallback
   cannot pass the v3 gate.
4. Qualify the Runtime plugin artifact on the designated canary and upgrade
   only the approved admin-viewer cohort, one Agent at a time using the current
   Runtime upgrade procedure and watch periods. Confirm all required viewers
   can render dated annotations before applying any batch. A successful old
   viewer that drops annotations is insufficient.
5. Repeat the complete maintained roster inventory and hosted current-selector
   census. A hosted match requires only the public selector from `identity.json`
   plus its matching `hosted-web` store row under the same subject's data root.
   Use private copies and the maintained probe; do not read secrets or infer
   current ownership from historical store presence. Reconcile every candidate
   to the fresh exact Brain roster, account subject and source evidence. Leave
   unsupported identities Unknown; retain non-controller historical wording.
6. Create one private manifest per exact Brain, binding the configured server,
   linked operator, evidence digests and current application window. Under the
   existing global Core CLI coordination lock, and with Core stable, preview
   each using `finite-saas-core brain-scoped-records --manifest PATH`. Verify
   expected created/no-op rows and the natural keys, not random preview IDs.
   Protect files/receipts as private. No direct database edits substitute for
   the writer.
7. Apply a designated canary Brain with the identical manifest and preview's
   `--expected-manifest-sha256`, adding `--execute`. Keep the committed receipt.
   Verify the exact roster, unchanged access, v3 coverage, expected annotations,
   no new binding conflicts and Chat old-history/reload/new-message checks.
   Continue one Brain at a time only while every gate passes. A failure stops
   future batches; report completed and remaining scope separately.
8. For attribution rollback, preview a fresh `retire` manifest naming the
   creating `targetOperationId`, then execute with its reviewed digest. It must
   match every created afterimage; drift stops the whole retirement. It does
   not retire existing/no-op rows or undo normal observations. Do not restore
   databases, alter roles, replace identity stores or merge accounts to undo
   labels. Software rollback uses the qualified previous artifacts and retains
   additive tables and audit receipts.

`--brain-identity` must report scoped coverage as checked and the expected
conflict count. `--brain-roster ID` must report checked v3 for every page;
its direct-Core evidence is not a substitute for the actual admin viewer.
Record every Brain/Agent's final state and residual Unknown rows. Completion
requires the approved full scope and all required end-to-end checks.

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

- Hosted Chat bootstrap registration needs the new Core and dashboard. It uses
  the existing private listener and observation credential; no extra setting,
  migration or operator backfill is required. Deploy Core before the dashboard
  when possible; an older Core returns 404 and Chat continues. Existing Accounts
  register on their next successful bootstrap. Registration changes neither
  sharing nor participation. Binary rollback retains accepted associations and
  receipts; their existing Core backup coverage is unchanged.
- Per-row resolution messages need the new Runtime plugin and dashboard. Older
  plugins remain readable with a generic unavailable-details label. Preserve
  the public-name, scoped annotation, lifecycle and private-alias fallbacks.

- Hosted Agents keep the `fbrain` in their pinned Runtime until that Runtime
  is upgraded separately. Their `access list` is the older summary; the
  managed FiniteBrain skill tells them to check the report `version` and say
  a newer CLI is needed rather than claim a complete report.
- Applied Approve is the current Chat UI path that records a hosted human's
  association and sharing. The retained invitation-accept API also records an
  explicit sharing request; the old Join card has been removed. No current
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
- Under legacy v1/v2, keys that never acted are not sent to Core. V3 sends the
  exact page with a participating subset and permits only scoped operator
  records to describe those additional keys; adding an arbitrary key to a
  Brain still cannot reveal its controller.
