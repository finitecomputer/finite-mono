# Agent Runtime Rollout Audit — Gotchas and Guardrails

Status: documentation only, except for the separately approved upgrade-time
secret-reference refresh described below. No other remediation in this audit
is authorized or implemented by this run.

This audit extends the known-good production baseline in
`docs/runs/production-baseline-2026-07-15.md`. Its purpose is to keep routine
rollouts boring: exact artifact, explicit cohort, one canary, serial upgrades,
and a stop on the first ambiguous Runtime.

## Current fleet observations

The read-only fleet check on 2026-07-16 found 29 active Core Runtime links and
26 canonical Kata containers. All 26 present containers were running the then
promoted `finite-agent-runtime-2026-07-16.1` digest, returned healthy contact
documents, and preserved their expected Agent Principals. Host disk, memory,
load, and the 30-sandbox cap had headroom.

The three active Core rows without canonical compute were Smoke Studio, Sol 2,
and the old Waffle. They are not rollout candidates. AEON Canary and Sites
Canary still occupied capacity, and one exited noncanonical container remained
outside the canonical fleet. This audit did not remove or reconstruct any of
them.

## Gotchas the rollout plan must account for

### 1. RuntimeSpec secret references were launch-time snapshots

Before the approved fix in this run, an image upgrade copied the Runtime's old
`secretReferences` unchanged. Adding a secret to the host and Runner config
therefore fixed new Agents but not an older Agent whose persisted RuntimeSpec
did not name it. An ordinary restart also could not add the missing value.

The approved behavior is deliberately narrow:

- an explicit image upgrade replaces the Runtime's host-wide secret-reference
  snapshot with the currently configured set (plus the required
  `FINITE_PRIVATE_API_KEY` reference), so retired references such as OpenRouter
  do not block future upgrades;
- the Runner resolves those names from its existing operator-side secret file
  and writes values through the existing root-only environment-file path;
- secret values never enter Core state, command arguments, logs, or this repo;
- restart, recovery, stop, and destroy do not refresh references;
- a missing configured value fails the upgrade before replacement.

Operational consequence: changing only the host secret file is not a rollout.
An Agent needs a real target artifact upgrade before newly referenced secrets
can appear in its environment. Keep Core and Runner configuration stable while
an upgrade operation is in flight.

### 2. Infrastructure deploy, artifact promotion, and Runtime rollout differ

A lat1 NixOS switch updates Core/Runner services. Publishing or promoting an
Agent Runtime artifact changes what can be selected. Neither action upgrades an
existing Agent. The rollout command must bind and execute an explicit promoted
artifact against an explicit cohort.

A same-artifact plan correctly skips an Agent. It must not be used as a hidden
"reconfigure" mechanism; replacement and its downtime should remain visible as
an upgrade.

### 3. Core inventory and provider inventory can diverge

An active project/runtime link does not prove that canonical compute exists.
Fleet preflight must compare Core rows with provider-owned canonical container
names and fail closed for either a missing canonical container or an unexpected
duplicate. A missing-compute Runtime belongs to the separate backup/restore and
generic-relaunch run, not to a normal image rollout.

### 4. A failed stop or containerd/ttrpc call is not permission to delete

Transient provider-control failures have occurred during upgrades. A bounded
retry is reasonable only after proving there is no concurrent operation and
the same canonical container and `/data` bind still exist. Never remove the old
container as cleanup: the current launcher can upgrade existing compute but
cannot generically recreate missing compute for an existing Runtime.

### 5. Capacity headroom is part of rollout safety

Canaries, stale Core links, and abandoned/noncanonical containers can obscure
the effective sandbox count. Check the Runner's advertised active count and
cap, provider inventory, host memory, disk, and the creation/control queues
before a broad rollout. Preserve room for new-user launches while training or
onboarding is active.

### 6. "Latest" must resolve to four exact identities

Before mutation, record and compare:

1. merged finite-mono Git revision;
2. deployed NixOS closure/revision;
3. promoted Runtime artifact id and source revision;
4. immutable OCI image digest actually present on each container.

Tags and UI version labels are useful displays, not sufficient rollout proof.

### 7. Host secret presence is not Runtime secret delivery

Preflight should report booleans and names only:

- configured reference exists in Core configuration;
- same name resolves to a nonempty value in Runner's secret environment;
- leased upgrade RuntimeSpec names it;
- post-upgrade container environment contains the name;
- a bounded product-level probe authenticates successfully where safe.

Never print or compare raw values in rollout output.

### 8. Post-upgrade health needs both infrastructure and product checks

HTTP health alone does not prove preserved identity or useful chat. For every
canary and cohort member, verify the exact artifact digest, one writable durable
`/data` mount, stable Agent Principal, contact endpoint, and normal Core status.
For a release that changes user-facing Runtime behavior, also exercise the
changed path (for example chat/tool ordering, attachment delivery, Telegram, or
Sites) on the canary before continuing.

### 9. Local Apple Container is not yet a full upgrade simulator

The Apple Container adapter currently advertises durable restart, not the
production Kata image-upgrade operation. Local tests can prove Core refreshes
and persists the exact references, Runner resolves values without logging
them, replacement environment construction adds missing values, and an Apple
Container restart preserves real durable Agent state. They cannot yet exercise
the complete production upgrade state machine against a locally retained
Agent. Until a deliberately scoped simulator exists, the disposable production
canary remains the required end-to-end upgrade proof. Do not broaden ordinary
restart semantics to hide this testing gap.

## Recommended routine rollout gate

1. Merge and identify the exact revision; finish local/component tests.
2. Build from that merged revision and promote one immutable Runtime artifact.
3. Read-only preflight Core/provider parity, queues, capacity, host health,
   secret-reference resolution, and the exact named cohort.
4. Snapshot the disposable canary's durable state, upgrade it, and verify
   identity, state, environment names, health, and the changed feature.
5. Pause for explicit broad-rollout authorization when the request requires it.
6. Upgrade named healthy Agents serially. Stop on the first failure or identity,
   mount, digest, or contact mismatch.
7. Re-run the read-only fleet comparison and record skips and failures. Never
   coerce a missing-compute Runtime through the normal rollout path.

## Separate proposed run

Backup/restore, missing-compute reconciliation, and generic relaunch remain the
separate proposed recovery run in the production baseline. That work needs a
synthetic restore proof and an explicit rollback boundary before production
mutation. It should not be folded into routine artifact rollout work.
