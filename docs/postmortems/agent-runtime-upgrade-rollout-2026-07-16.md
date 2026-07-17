# Agent Runtime Upgrade and Rollout Post-mortem

Date: 2026-07-16

Status: complete analysis; recommended follow-up work is not implemented by
this report.

Scope note: this report covers Agent compute replacement and the
missing-canonical-compute stranding class. The immediate Boss/Skyler dashboard
incident, including the ineffective first fix, misleading tests, and stale
browser behavior, is documented separately in
[`boss-hosted-chat-recovery-2026-07-16.md`](boss-hosted-chat-recovery-2026-07-16.md).

## Executive summary

Finite did not have one upgrade bug. We crossed several lifecycle boundaries
at once while moving from disposable canaries to a real fleet:

- publishing a Runtime artifact changed new launches but not existing Agents;
- upgrading existing Agents required a newly built Core/Runner operation;
- Core inventory could outlive the canonical Kata container it referred to;
- older RuntimeSpecs carried a launch-time snapshot of secret references; and
- local Apple Container acceptance could launch and restart an Agent, but could
  not rehearse the production old-image-to-new-image replacement.

The most serious outcome was Waffle. Its Kata/containerd control path was
wedged, and the canonical container was manually removed during recovery. The
durable Agent files remained on the host, but the supported lifecycle could
only restart or upgrade existing compute. It could not create replacement
compute for an existing Core Runtime. Waffle was therefore stranded even
though its durable files were still present.

The fleet rollout itself became substantially safer during the incident: it
now uses immutable artifacts, exact plans, canonical-container preflight,
serial execution, stable Agent Principal checks, retained `/data`, and a stop
on the first failed postcondition. Upgrade also refreshes the configured
secret-reference snapshot, which fixed the gap where newly provisioned FAL and
other tool credentials reached new Agents but not old ones.

Those controls reduce the probability of stranding an Agent, but they cannot
recover one whose canonical compute is already missing. The only complete
answer is the separately proposed recovery run: off-host per-Agent backups, an
empty-target restore proof, missing-compute detection, and a Core-bound generic
relaunch that preserves the expected Agent Principal and enforces one `/data`
writer.

## User and fleet impact

- Waffle became unavailable. Its host-side durable directory remained, but no
  supported operation could reconstruct its missing canonical compute.
- Existing Agents accumulated different Runtime artifact versions until the
  explicit rollout path existed. New features could work on a canary or a new
  Agent while older Agents remained unchanged.
- Existing Agents did not automatically receive newly configured tool secret
  references. Chester's missing FAL credential exposed this distinction.
- Initial rollout automation needed two production-discovered corrections: the
  exact lat1 source-host binding and SSH input isolation for a multi-Agent
  cohort.
- A read-only fleet audit on 2026-07-16 found 29 active Core Runtime links but
  only 26 canonical Kata containers. The three missing-compute rows were not
  eligible for ordinary rollout.
- No broad rollout data loss was observed. This is not a disaster-recovery
  claim: the retained files have not yet passed the required empty-target
  restore proof.

## What happened

### 1. Promotion and rollout were initially easy to conflate

Hosted Agents pin their Runtime artifact at launch. Publishing or promoting a
new image changes the artifact available for future launches; an infrastructure
deploy changes Core, Runner, dashboard, and host services. Neither action
changes an existing Agent.

Before the explicit Runtime Upgrade operation and rollout wrapper landed, the
fleet therefore drifted naturally. UI version labels and a successful new
canary could make “latest” look deployed even though most Agents were still on
older immutable artifacts.

### 2. The first rollout path exposed operator-tooling defects

PRs #76 and #78 established the guarded existing-Runtime upgrade and corrected
the production source-host binding. PR #93 made every SSH call non-consuming
so one remote call could not drain the remaining multi-Agent plan from shell
standard input.

These bugs were not failures of the durable Agent state model, but they show
that shell orchestration around a sound lifecycle operation is still part of
the production safety boundary. It needs the same contract tests and fail-closed
behavior as Core and Runner.

### 3. Waffle crossed from wedged compute into missing compute

Waffle's container-runtime control channel was unhealthy: the Kata VM process
and containerd's recorded state no longer agreed well enough for ordinary
control calls to recover it confidently. Clearing the stale operation fence was
appropriate. Removing the canonical container was not.

The current Runner has two different creation boundaries:

- a creation lease may launch compute for a new Agent; and
- restart/upgrade may operate on the existing canonical compute for an existing
  Runtime.

There is no supported third path that takes an existing Runtime with missing
compute and recreates it from retained or restored state. Manual deletion
therefore removed the only object on which restart and upgrade could operate.
The correct incident response would have been to stop after bounded read-only
diagnosis and retain the container, even if it remained unhealthy.

### 4. Secret delivery was part of the RuntimeSpec, not the host alone

The host secret file could contain a valid FAL, X, Perplexity, or other tool
credential while an older RuntimeSpec did not name it. Restart preserved the
old spec, correctly, so it could not introduce the reference. This made a host
configuration fix appear ineffective on old Agents.

PR #105 narrowed the rule: a real image upgrade refreshes the Runtime's
host-wide secret-reference snapshot from current Core configuration, and the
Runner resolves those names through the existing root-only environment file.
Restart and recovery do not silently reconfigure an Agent, raw values never
enter Core, and a missing configured value fails before replacement.

### 5. Local acceptance did not exercise the production replacement

The Apple Container SaaS smoke proved launch, real Hermes chat, durable state,
Agent Principal stability, service interruption, and same-image restart. It did
not prove an old artifact could be replaced by a new artifact against the same
retained `/data` tree. The Apple adapter advertises restart, not Runtime
Upgrade, so the production Kata canary was the first complete rehearsal of the
replacement topology.

### 6. “Dashboard-only” deploys still had control-plane blast radius

During this review, a dashboard image-pin deploy restarted the dashboard as
expected, but also restarted Core, Hosted Web Device, Finite Chat, Sites, and
Brain. The Nix workspace packages all receive the complete flake source
(`src = self`), so a new monorepo revision changes every Rust package input and
therefore the systemd unit store paths even when the intended product change is
only a dashboard digest pin.

The deploy completed successfully and all services returned active, but the
behavior makes the phrase “dashboard-only” misleading. A control-plane restart
cannot directly replace or delete Agent Kata compute, but it creates avoidable
chat and creation-plane interruption and makes a small hotfix carry a broader
rollback surface.

## Root causes

### Primary causes

1. **Lifecycle asymmetry:** existing-runtime restart and upgrade assumed the
   canonical provider handle existed, while only new-Agent creation could
   create compute.
2. **No proven Recovery Set path for Agents:** durable `/data` existed, but
   per-Agent off-host backup, empty-target restore, and generic relaunch were
   intentionally deferred.
3. **Incomplete upgrade test rung:** local acceptance stopped at launch and
   restart, leaving full replacement to the production canary.
4. **Split inventory:** Core's active Runtime link and containerd's canonical
   container inventory could diverge without a standing alert.
5. **Configuration snapshot semantics were not visible enough:** secret
   presence on the host was mistaken for delivery into every existing
   RuntimeSpec.
6. **Over-broad Nix source invalidation:** a deploy revision, rather than the
   service's relevant source closure, determined whether unrelated Rust service
   paths changed.

### Contributing conditions

- The team was moving quickly from no external users to a training cohort.
- “Deploy,” “promote,” “restart,” and “roll the bots” had previously been used
  too loosely for four materially different operations.
- Fleet version visibility emphasized human-readable labels rather than Git
  revision, artifact id, and immutable digest together.
- The safest operator action during an ambiguous provider failure—stop and
  retain compute—was documented in the Runtime rollout audit but not prominent
  in the general break-glass entry point.
- Broad rollout could put the canary first, but one invocation could continue
  immediately into the rest of the fleet without a mandatory observation hold.

## What worked

- Agent data lived outside replaceable compute under a stable host bind.
- The Runner's upgrade state machine pulls before downtime, keeps one writer,
  retains the old container during candidate validation, and checks both
  `/healthz` and the pre-upgrade Agent Principal before accepting replacement.
- Core binds every operation to an exact Runtime, source host, source machine,
  artifact id, and state schema.
- The rollout wrapper plans first, rejects missing canonical compute before any
  mutation, executes serially, and stops on the first failure, timeout, wrong
  artifact, non-online result, or binding drift.
- Immutable OCI digests and the production baseline made “known good” concrete.
- Canaries found deployment and compatibility problems before another broad
  rollout.
- Secret-reference refresh was implemented as a narrow upgrade behavior rather
  than a general restart side effect.
- The old Waffle state was left intact after the failure; it was not manually
  reconstructed or rewritten.

## What must change

### P0 — before the next broad Agent Runtime rollout

#### 1. Make the canary a real hold point

Change broad rollout into two explicit phases:

1. plan the full exact cohort and upgrade only the named disposable canary;
2. emit the frozen remainder and stop; then require a second explicit command
   after the canary's product acceptance.

The hold must occur after verifying the digest, mount, Agent Principal,
contact, historical chat/workspace state, and the feature changed by the
release. “Canary first in one loop” is ordering, not a canary gate.

#### 2. Add a deploy impact preflight

Before `switch-to-configuration`, `scripts/deploy-lat1` should compare the
current and candidate closures and print the services whose unit definitions or
executables will change. The caller must declare the expected deploy class or
expected service set. A dashboard-only request must fail before activation if
Core, Runner, Chat, Sites, or Brain would restart.

This is a safety check, not the final build fix. It makes hidden blast radius
visible immediately.

#### 3. Turn fleet reconciliation into a required gate

Before and after rollout, generate one machine-readable report containing only
non-secret facts:

- active Core Runtime links;
- canonical provider containers and unexpected duplicates;
- current artifact id and immutable image digest;
- expected and observed Agent Principal;
- exactly one writable `/data` bind;
- Runtime status and contact health;
- active control operations, launch queue, sandbox count/cap, and host
  headroom; and
- configured, resolvable, leased, and post-upgrade secret-reference names as
  booleans, never values.

Missing or duplicate compute, an identity mismatch, an unresolved required
secret, an active conflicting operation, or insufficient headroom must block a
broad rollout. Explicit exclusions stay in the report with a reason.

#### 4. Retain a durable rollout record

Every rollout should retain the source revision, Nix closure, artifact id,
image digest, exact ordered cohort, canary result, per-Agent operation id and
postconditions, exclusions, and final fleet reconciliation. This can be a CI or
operator artifact; it does not need a new database subsystem.

#### 5. Put the no-delete rule in the break-glass path

The first screen of Runtime incident guidance should say: do not remove,
rename, or recreate canonical Agent compute manually. If a typed operation
cannot prove one container, one `/data` writer, and one expected Agent
Principal, stop. Provider deletion is not cleanup and must never be the first
leg of restart, upgrade, or recovery.

### P1 — eliminate the known stranding class

#### 1. Complete the separate Agent recovery run

This is the only work that makes missing compute recoverable rather than merely
less likely:

1. produce encrypted, periodic, off-host backups of each full Agent `/data`;
2. alert on failed or stale backups;
3. restore synthetic state onto an empty isolated target while the source and
   outbound side effects are fenced;
4. verify chat, attachments, workspace, agentd state, Sites state, and the
   expected Agent Principal; and
5. only then add a Core-bound generic relaunch from the exact persisted
   RuntimeSpec, artifact, secret references, source identity, and expected
   Principal.

The relaunch must fail closed on ambiguous state and atomically establish one
canonical provider handle and one writable `/data` owner. A provider durable
volume or a stopped container is not a backup.

#### 2. Add missing-compute detection before relaunch

A read-only reconciler should alert when an active or stale Core Runtime has no
canonical provider container, when more than one candidate owns the same
durable root, or when provider compute exists without a matching Core Runtime.
Detection can ship before mutation. Automated repair waits for the restore and
generic-relaunch proof.

#### 3. Build a retained-state local upgrade fixture

Extend the Apple Container acceptance harness to:

1. launch a pinned N-1 image and create representative chat, attachment,
   workspace, Sites, and agentd state;
2. stop old compute while retaining the exact `/data` bind;
3. replace only compute with the candidate image;
4. require the same Agent Principal and historical state;
5. exercise the changed feature; and
6. test failure before and after the handle swap, proving the old image resumes
   with one writer.

This may remain a test fixture rather than broadening ordinary Apple restart
semantics. Until it exists, a disposable production Kata canary remains
mandatory.

#### 4. Narrow Nix service source inputs

Use content-scoped source sets for each Nix-built service: root Cargo metadata
and lockfile, the selected crate, and its actual workspace dependencies. An
unrelated dashboard or documentation change should not change a Rust service's
derivation or systemd store path. Keep the closure impact preflight even after
this optimization; it protects future dependency mistakes.

#### 5. Exercise N-1 compatibility, not only candidate health

For every Runtime release, test N-1 state into N before promotion. When the
state schema is declared rollback-compatible, also test a return to the
previous known-good artifact against the post-N state. A matching schema label
is an admission check, not proof that an image did not mutate data
incompatibly.

### P2 — make fleet state obvious

- Show artifact id, immutable digest, last successful operation, and contact
  health in the admin fleet view.
- Show counts by artifact and surface missing/duplicate compute as a fleet
  fault, not as an ordinary “offline” Agent.
- Record the non-secret secret-reference set or its hash in the admin overview
  so operators can distinguish “host value exists” from “RuntimeSpec names it.”
- Track rollout duration and failures by phase so container pull, stop, boot,
  contact, identity, and Core completion failures are distinguishable.

## Recommended release and rollout flow

### Control-plane or dashboard deploy

1. Merge; deploy only a revision on `main`.
2. Build immutable service images from that exact revision.
3. Evaluate the candidate Nix closure and review the changed-service set.
4. Take the existing pre-deploy coordinated snapshot.
5. Switch only when the impact matches the declared deploy class.
6. Verify every restarted service plus public product health.
7. Do not infer an Agent Runtime rollout.

### Runtime release for new Agents

1. Pass local component and real-Hermes smoke on the exact merged revision.
2. Build the canonical Runtime image once and retain its report.
3. Register and promote the immutable digest and compatible state schema.
4. Launch a fresh disposable Agent and pass the product acceptance relevant to
   the release.
5. Promotion changes future launches only.

### Existing-Agent rollout

1. Pass the new-Agent release flow.
2. Reconcile Core, provider inventory, queues, capacity, mounts, identities,
   and secret-reference names.
3. Snapshot and upgrade one disposable existing-state canary.
4. Verify retained state and the changed product feature; then stop for the
   explicit cohort go-ahead.
5. Upgrade the frozen named cohort serially.
6. Stop on the first ambiguous result. Never delete canonical compute.
7. Reconcile again and retain the rollout report.

### Missing-compute recovery

Do not route this through normal rollout. Fence the Runtime, select a proven
backup, restore onto an empty isolated target, verify the expected identity and
state, then use the future Core-bound relaunch operation. Until that path is
implemented and proven, missing canonical compute remains an operator-visible
hard stop.

## Acceptance criteria for “safe upgrades”

We should call the upgrade system safe for valuable Agents only when all of the
following are true:

- a candidate passes N-1 retained-state replacement before promotion;
- a broad rollout cannot pass its canary without a separate continuation;
- deploy tooling rejects an unexpected service restart set;
- every Runtime operation proves one durable writer and a stable Agent
  Principal;
- fleet reconciliation detects missing, duplicate, stale, and unexpected
  compute before mutation;
- every Agent has a recent encrypted off-host backup;
- an empty-target restore has succeeded from that Recovery Set;
- a missing-compute Runtime has been relaunched from restored state through a
  supported, idempotent Core/Runner operation; and
- rollback and failure-injection tests cover crashes on both sides of the
  candidate/canonical handle swap.

Until the backup, restore, and relaunch criteria pass, the honest statement is:
ordinary upgrades preserve healthy existing Agents and fail closed around
known bad topology, but Finite cannot yet guarantee recovery of an Agent whose
canonical compute is lost.

## Proposed implementation order

1. Small, low-risk PR: deploy changed-service preflight, explicit canary hold,
   retained rollout report, and prominent no-delete break-glass warning.
2. Test-focused PR: retained-state Apple upgrade fixture and N-1 compatibility
   lane.
3. Build-focused PR: content-scope Nix service sources so component-only
   deploys do not restart unrelated services.
4. Separately reviewed recovery run: off-host per-Agent backup, synthetic
   empty-target restore, read-only missing-compute alert, and only then generic
   relaunch.

The first three reduce rollout risk without changing durable production state.
The fourth changes the Recovery Set and lifecycle boundary and therefore keeps
its own proof, rollback, and production-authorization requirements.

## Related evidence

- [`production-baseline-2026-07-15.md`](../runs/production-baseline-2026-07-15.md)
- [`runtime-rollout-gotchas-2026-07-16.md`](../audits/runtime-rollout-gotchas-2026-07-16.md)
- [`runtime-image.md`](../../infra/runbooks/runtime-image.md)
- [`finite-chat-reliability-remediation-2026-07-15.md`](../audits/finite-chat-reliability-remediation-2026-07-15.md)
- [ADR 0001: Recoverability precedes operator-blindness](../adr/0001-recoverability-precedes-operator-blindness.md)
- [`hosted-web-chat-disaster-recovery.md`](../runs/hosted-web-chat-disaster-recovery.md)
