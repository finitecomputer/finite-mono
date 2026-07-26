# Camp SaaS Stability

Status: ACTIVE

Owner: Paul

Opened: 2026-07-26

Expires: 2026-08-03, or earlier only when Paul explicitly lifts the camp
feature freeze.

Acceptance: A new customer can sign up through production, obtain an Agent
creation entitlement, create a normal Standard Agent on `finite-lat-3`, and
use chat without affecting an existing Agent. The same path then admits a
staged camp cohort within the recorded entitlement, host-capacity, recovery,
and stop boundaries.

## Operating rule

This is a feature freeze. Until this run closes:

- accept only bounded production bug fixes and reliability work that protects
  signup, Agent creation, chat, or recovery;
- do not merge or deploy PR #303, pairing/protocol changes, new iOS or Electron
  releases, cold-relocation changes, Runtime upgrades, storage migrations, or
  new product surfaces;
- require every allowed deploy to name the user-visible failure it fixes, the
  complete producer/consumer through-line, synthetic proof, backup boundary,
  rollback, and post-deploy production observation;
- keep `finite-lat-1` creation-drained and `finite-lat-3` as the only
  undrained Standard creator; and
- stop rather than improvise a durable-state repair or automatically undrain a
  second creator.

The causality review standard comes from
[`production-causality-2026-07-25.md`](../postmortems/production-causality-2026-07-25.md):
**Don't Break Chat.** Signup and Agent creation are the next protected
through-lines.

## Frozen production baseline

Read-only production inspection on 2026-07-26 established:

- production application code is merged revision
  `2c3261e14add3e4bee474daeae105be469c1079c`; current `main` differs only by
  the production-causality post-mortem;
- Chat reports healthy at contract version 6;
- Dashboard customer mode, live Stripe configuration, WorkOS authentication,
  Core, Identity Authority, and Hosted Chat are enabled;
- `/signup` redirects to the WorkOS hosted signup flow, and an unauthenticated
  `/dashboard` request fails closed with `401`;
- `finite-lat-1` has `FC_RUNNER_DRAIN=true`;
- `finite-lat-3` has `FC_RUNNER_DRAIN=false`,
  `FC_RUNNER_MAX_SANDBOXES=32`, and Runtime artifact
  `finite-agent-runtime-2026-07-24.2`;
- the lat3 Runner timer, private Core path, WireGuard link, RAID1 root/data,
  swap, and storage health are active and healthy;
- no Agent creation request is currently requested or launching; and
- application services have no warning-or-higher journal entries after the
  completed 2026-07-25 incident repair.

No deploy is required to make lat3 the default creator. Untargeted Standard
creation is claimed by the sole undrained compatible Runner, which is already
lat3. The drain pair is an operational invariant and must be checked before
every camp batch because setting both Runners to drain previously broke
signup-to-Agent creation.

## What changed and where it can fail

The ordinary paid signup and creation through-line is:

1. WorkOS signup and callback establish the dashboard session.
2. Stripe Checkout and its signed webhook establish the Core billing account.
3. Core grants one Standard creation entitlement.
4. Dashboard creates an untargeted Standard/Kata request.
5. The sole undrained Runner claims the request, launches the pinned Runtime,
   waits for `/healthz` and `/contact` to expose the Agent Principal, binds the
   managed Agent email through Identity Authority, and completes the Core
   Runtime record.
6. Hosted Device and Chat use that completed Runtime binding.

Recent changes increased the risk surface at steps 4–6:

- shared managed Agent Identity made Identity Authority a synchronous creation
  dependency. The initial rollout bound identity before `/contact` was ready
  and returned HTTP 503. The deployed fix waits for the Agent Principal before
  returning launch facts, but a normal fresh lat3 creation has not yet proved
  that exact production path;
- cold relocation added another request kind to the creation queue and Runtime
  launcher. Its failed attempts created both misleading dashboard recovery UI
  and one extra lat3 container not represented by a Core Runtime;
- confidential hosting added a second hosting branch. It remains admin-gated;
  normal customers must continue to receive Standard/Kata;
- the Runner poll interval changed from 20 seconds to 5 seconds. This reduces
  queue latency but does not make launches parallel: one Runner cycle claims
  at most one creation;
- a Core/Chat state contract change broke existing chat until the compatibility
  repair now deployed as the frozen baseline; and
- PR #303 changes pairing and protocol contracts but has no accepted physical
  iOS/Electron result. It stays open and undeployed during this freeze.

## Capacity and entitlement truth

The camp estimate is accepted only if “30 new bots” means approximately 30
total, including bots created by new users and the team. Fifteen customer bots
plus 30 additional team bots would exceed the lat3 hard maximum.

At the initial 2026-07-26 inspection:

- lat3 had one canonical canary and one non-canonical container left by a
  failed relocation;
- existing organizations collectively have 13 unused Agent creation
  entitlements;
- four unredeemed, unexpired launch codes remain; and
- a normal active Standard Stripe subscription grants one Agent entitlement;
  it does not grant an unbounded number of team test Agents.

The first normal canary then failed managed-email binding and left a second
non-canonical container. Lat3 therefore currently observes 3 of 32 slots
occupied: one canonical canary and two failed-launch remnants. Stopping both
non-canonical containers would leave the canary plus room for 30 new Agents
with only one spare slot.

Therefore cleanup, capacity, and entitlement are separate gates. Do not enqueue
30 requests and discover any limit in the product UI.

## Recovery boundary

The control/app state required for signup and routing is covered by the
successful daily encrypted off-host Hosted Web Chat Borg job. Core has a
current six-hourly Postgres dump and Identity Authority has a current validated
SQLite backup.

Live lat3 Agent `/data` is not periodically copied off host. RAID1 protects
availability from one disk failure but is not a backup. The proven
provider-independent Borg path snapshots a stopped Agent as part of Runtime
Retirement and can restore it onto an empty target; it is not continuous
protection for an active Agent.

Camp acceptance must state this residual risk plainly. Do not claim that a
new Agent's live chat/workspace state has off-host recovery until a separate
reviewed, writer-consistent periodic Agent backup has passed an empty-target
restore.

## Queue

Work top-down:

- [x] Freeze PR #303 and all non-bug-fix feature/release work.
- [x] Confirm the live drain pair, queue, host storage, private Core path,
  Runtime artifact, signup redirect, Core/Identity/Chat health, and current
  control-plane backup boundary.
- [ ] Fix the failed normal canary: lat3 reached Runtime readiness but sent
  the privileged managed-email binding to the public Identity vhost, where
  operator routes intentionally return `404`. Add a peer-scoped WireGuard
  proxy to the loopback Authority, point only the lat3 Runner at it, and keep
  the public operator-route `404` invariant.
- [ ] With explicit production authority, stop only the two exact
  non-canonical lat3 containers left by the failed relocation and failed
  normal canary. Do not remove their metadata or `/data`; stopping is the
  rollback-preserving capacity repair.
- [ ] Paul creates one normal Standard Agent through the production dashboard.
  Observe the request at every boundary and prove that lat3, not lat1, claims
  it.
- [ ] On the new Agent, complete two fresh chat turns, refresh the dashboard,
  and complete a third turn. In parallel, complete one fresh turn on an
  existing Agent.
- [ ] Decide how the team receives enough explicit creation entitlements for
  its planned test Agents. Preserve paid-customer billing semantics; do not
  repair this by editing Core rows.
- [ ] Admit the cohort in observable batches: 1 accepted canary, then 3, then
  5, then at most 10 per batch. Require the queue to return to zero and sample
  fresh chat after each batch.
- [ ] Before the final batch, recompute canonical container count, remaining
  entitlements, memory/swap, disk, storage health, and pending/failed requests.
- [ ] Paul explicitly accepts the live-Agent recovery limitation for camp or
  activates a separate reviewed periodic-backup run. This queue does not
  improvise a live-filesystem archive.

## Stop and rollback boundaries

Stop new admissions immediately if:

- both Runners are drained or lat1 becomes undrained;
- a normal request is claimed by any source host other than `finite-lat-3`;
- Identity Authority binding returns 5xx or the Agent Principal changes;
- a failed creation leaves compute or durable state not represented by Core;
- existing or new chat cannot load, decrypt history, send, or receive;
- the queue stops making progress, a batch produces any unexplained failure,
  or lat3 reaches its hard capacity;
- RAID/storage health degrades, swap grows unexpectedly, or a required
  control-plane backup becomes stale.

The admission stop is to set lat3 creation drain true and wait for the current
bounded Runner operation to finish. Do not automatically undrain lat1, delete
containers, rewrite Core state, or retry a destructive operation. Existing
Agents and chat stay in service while evidence is gathered.

## Acceptance Request

- **Revision:** production revision
  `2c3261e14add3e4bee474daeae105be469c1079c`, Runtime artifact
  `finite-agent-runtime-2026-07-24.2`.
- **Where:** `https://finite.computer/dashboard`, Paul's normal account, a new
  disposable Standard Agent name.
- **Time:** 15 minutes for the one-Agent gate.
- **Steps and observations:** create one normal Standard Agent; observe a
  lat3 claim and completion; open chat; complete two turns; refresh; complete
  one more turn; send one turn on an existing Agent.
- **Pass:** the creation has exactly one canonical lat3 Runtime, no lat1
  creation, no orphaned compute, stable Agent Principal and email binding,
  usable refreshed chat, and no regression on the existing Agent.
- **Fail/stop:** preserve the request, container, and `/data`; drain lat3
  creation; capture read-only Core/Runner/Identity/Chat evidence; do not start
  the staged cohort.
