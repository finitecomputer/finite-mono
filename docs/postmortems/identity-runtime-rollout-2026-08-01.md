# Identity and Agent Runtime Rollout Post-mortem

Date: 2026-08-01

Status: production train deployed; fleet convergence is incomplete (one lat1
Runtime exception and no broad lat3 cohort); product acceptance remains open
for Brain. Recommended follow-up work is not implemented by this report.

Source plan:
[`identity-rollout-reconciled-plan.md`](../identity-rollout-reconciled-plan.md)
and [`identity-rollout-test-log.md`](../identity-rollout-test-log.md).

All times below are UTC. Product observations made on 2026-08-02 are labeled
separately from gates that passed during the rollout.

## Executive summary

The August 1 train deployed the planned Brain identity work, Sites identity
keyset and sharing work, Chat clarification and presentation repairs, updated
CLIs, and Agent Runtime `finite-agent-runtime-2026-08-01.1`. The control plane
finished on finite-mono revision `85d08a486f7876e09fbd6e247e62c0c58a6130f3`.
Sites reconciliation migrated 27 rows, reported zero conflicts, left 36 rows
without sufficient proof unchanged, and preserved every measured legacy access
surface. Twenty-one of 22 active lat1 Agents reached the target Runtime. The
remaining Agent is healthy on the prior known-good Runtime because its old Kata
VM cannot complete the supported shutdown operation. Six inactive Runtime
links were intentionally untouched. On lat3, the new-Agent canary and existing
Brain canary reached the
target, but the broad existing-Agent cohort that had not been named before the
lat1 outage was never planned or executed. The retained evidence does not prove
the artifact version of every other running lat3 Agent. Phala Runtime upgrades
were not attempted.

The user-facing outcome is encouraging. On August 2, the product owner
confirmed that Sites publishing and sharing work well in real use. Chat has not reproduced the
incoming-update focus race; the sidebar still reorders as messages arrive,
which is visually jarring but not a rollback blocker. Brain navigation remains
disabled. Brain validation is still in progress, with promising results so far.

The operational outcome is more mixed. CI and release preparation consumed
hours because the Brain matrix exposed several independent races, the lat2
runner exhausted disk twice, an operator SSH logout disrupted CI PostgreSQL,
and every small monorepo source change rebuilt and restarted a much wider
service set than its product scope. Production reconciliation found one real
Sites adapter bug and correctly stopped before mutation. The fix was small, but
it required another full CI, exact-main gate, closure build, snapshot, offsite
archive, and broad app-plane restart.

The most serious event was finite-lat-1 becoming completely unreachable during
the eighth broad Agent upgrade. Latitude showed the server as `OFF`; the host
did not return until an operator powered it on about two hours later. The previous boot
journal ended abruptly, with no orderly shutdown, kernel panic, OOM, thermal,
machine-check, watchdog, filesystem, or storage error. A provider/BMC or power
event and a firmware/hardware protection event under transient load remain
plausible. The rollout is temporally correlated but not proven causal. Latitude
or BMC power-event telemetry is required to resolve this.

The cold boot exposed two difficult Agent lifecycle failures:

- The interrupted-upgrade Agent stopped between candidate creation and handle
  swap. It
  briefly had two VMs mounting the same durable data and advertising the same
  npub. The control plane failed closed, but recovery required removing the
  exact candidate, terminating an orphaned shim/VM, and isolating stale
  containerd Runtime state before the normal upgrade could succeed.
- The lifecycle-exception Agent repeatedly failed while stopping its old VM
  with `ttrpc: closed`.
  Recovery crossed stale CNI namespace, wrapped QEMU, restart-manager, and Kata
  sandbox-state boundaries. The Agent was restored on its original image and
  data, but a final normal upgrade still failed at old-container shutdown.

No Agent `/data` or identity material was hand-edited, and all 22 active lat1
Agents finished online with their expected Agent Principals. That is not enough
for Phala. The recovery depended on host visibility and manual manipulation of
ephemeral provider state. An opaque provider would not expose those internals.
Phala upgrades must therefore remain disabled until the lifecycle can recover
using provider-neutral, externally observable operations and a proven
empty-target Agent restore path. Shipping Phala cannot depend on an operator
opening a VM host and repairing CNI, QEMU, containerd, or Kata records by hand.

## Intended plan compared with the outcome

| Planned area | August 1 outcome | Remaining acceptance |
| --- | --- | --- |
| Brain identity overhaul from PR #172 | Deployed in servers, CLIs, and Runtime; migrations and product matrix had passed; Brain nav stayed disabled; an existing Brain canary recovered its conversation | Complete end-user Brain validation is still in progress |
| Sites CLI hygiene | Released as `fsite` 0.5.0 and included in the Runtime | The complete mixed old/new CLI checklist was not manually exercised during the production window |
| Sites keyset and reconciliation | Applied after preview and synthetic rehearsal: 27 migrated, zero conflicts, 36 `needs_proof`; second pass was a no-op | No destructive revoke flow was part of this rollout |
| Sites owner/viewer/request UX | Deployed; a new Agent had the expected owner binding | The product owner confirmed publishing and sharing on August 2; individual Request Access and first-publication-mail checklist items were not separately recorded as accepted |
| Chat navigation, tool disclosure, and clarification | PRs #355, #356, and #369 were deployed; a real canary Chat passed | Product acceptance found no focus race; sidebar reordering remains. Compaction stayed parked because pinned Hermes has no semantic event |
| One classified deploy train | Completed for control plane, CLIs, Sites reconciliation, Runtime promotion, and Kata rollout | The actual Nix blast radius was wider than product classifications implied |
| New-Agent and existing-Agent canaries | A new lat3 Agent, a legacy-identity lat1 Agent, and an existing Brain lat3 Agent passed before the broad cohort | The new canary proved Chat, ownership, key readiness, and disabled Brain nav, but not every Sites and Brain checklist item |
| Fleet Runtime/environment reconciliation | 21 of 22 active lat1 Agents reached the target; desired Brain environment repaired for those upgraded Agents. The new and existing lat3 canaries reached the target | One lat1 Agent remains on the prior artifact. Six inactive lat1 links remain intentionally untouched. The remaining lat3 cohort was not frozen or rolled after the outage, so fleet-wide target convergence is not proven. No Phala upgrades |
| Recovery boundary | Coordinated v3 snapshots and offsite Borg archives passed; PostgreSQL crash recovery and integrity checks passed | This was not an empty-target restore of every Agent `/data` Recovery Set and must not be described as one |

## User and fleet impact

- Finite-lat-1 was unavailable for roughly two hours, from approximately 19:05
  until it was powered on at 21:09. Dashboard and Sites public health, SSH, and
  ICMP all failed from multiple networks during the event.
- Seven broad lat1 upgrades had complete postflight records before the outage.
  The eighth, interrupted-upgrade Agent had preflight evidence but no completion
  evidence.
- After recovery, all 22 active lat1 Agents returned HTTP 200 from `/contact`.
  Twenty-one run the target image; one runs the prior known-good image.
- Real Finite Private inference passed using pre-rollout credentials on both
  difficult-upgrade Agents. Core retained 150 key records: 117 active and 33
  revoked.
- No database corruption, invalid PostgreSQL indexes, filesystem error, storage
  error, identity change, or Agent data loss was observed.
- Six inactive Runtime links were not activated merely because they appeared
  in inventory.
- The broad lat3 cohort did not run. The new-Agent and existing Brain canaries
  passed, but no retained plan establishes target convergence for every other
  running lat3 Agent.
- The interrupted eighth operation briefly created a two-writer risk. It was
  detected before normal progression and reduced back to one writer before the
  rollout continued. No resulting data damage was observed.

## Timeline

### Merge and CI recovery

- **14:59** — Work resumed on the approved stack. PRs #371, #368, #372, #369,
  #364, #365, and #367 were merged in dependency order after full checks.
- **15:05** — An operator SSH session to lat2 ended while CI PostgreSQL was
  running as the same `ubuntu` account. Systemd `RemoveIPC` removed PostgreSQL
  shared-memory segments and made the rerun fail readiness. The operator then
  stayed off lat2 while jobs were active.
- **15:14–15:17** — The self-hosted runner disappeared because lat2 reached
  zero free disk. Sixteen stale `brain-matrix-*` work directories were removed,
  reclaiming 131 GB of disposable CI state.
- **15:31–15:52** — The remaining Chat and Sites stack merged. Stale runner
  `busy` leases required bounded listener restarts only when no job was
  assigned.
- **16:14–16:28** — The exact-main Brain run became stuck while lat2 again
  approached capacity. Eight stopped CI containers were removed; unused image
  layers and 16 GB of build cache were pruned. All 12 running containers were
  preserved, and lat2 ended with 265 GB free.

### Release preparation

- **16:36–16:48** — Dashboard `2026-08-01.1` was published, and release PR
  #373 pinned the dashboard digest and prepared `fsite` 0.5.0 and `fbrain`
  0.2.0.
- **16:54–17:02** — Exact-release-SHA Brain CI first hit an HTTP 429 in a rapid
  synthetic collaboration sequence, then hit Chromium `ERR_NETWORK_CHANGED`
  after the deliberate Brain restart. The second failure identified a real
  harness readiness gap.
- **17:02–17:26** — PR #374 added a bounded pre-click readiness retry while
  keeping the mutating Create click single-shot. Its first CI attempt exposed a
  TypeScript nullability error that unit tests had missed; the production Next
  build caught it. The corrected PR and full matrix passed.
- **17:32–17:41** — Exact-main services smoke hit a transient direct-SQLite
  `database is locked (5)` observer race. One bounded rerun passed.
- **17:43–17:55** — Runtime source smoke was initially blocked by a root-owned
  matrix scratch directory. After ownership repair, the Runtime, CLI releases,
  lat1 closure, and lat3 closure built from the accepted SHA. Runtime
  `finite-agent-runtime-2026-08-01.1` was published at immutable digest
  `sha256:8b56ed2125eb03cdbe9c05f7686906ab2db6304a791c5321d6e9ca183c4fcf8f`.

### Control plane and Sites reconciliation

- **17:57–18:03** — Runners were drained, a coordinated v3 snapshot and offsite
  Borg archive passed, and the reviewed lat1/lat3 closures activated. Lat3's
  declarative activation briefly re-enabled its Runner timer; it was stopped
  again before work could be claimed.
- **18:03** — Sites reconciliation preview failed closed. Ordinary external
  mailbox addresses were sent to the finite NIP-05 resolver. Identity correctly
  returned HTTP 400; the Sites adapter incorrectly treated “not this resolver's
  domain” as a fatal service error.
- **18:06–18:28** — PR #375 mapped Identity 400/404 resolution results to “not
  resolved” while retaining all other failures. Full PR and exact-main CI
  passed after one unrelated deleted-Brain-text matrix flake and bounded rerun.
- **18:26 and 18:41** — Two nominally read-only SQLite comparisons opened an
  immutable snapshot in WAL mode and created `registry.db-shm` and empty
  `registry.db-wal` sidecars. The recorded database did not change, but the
  extra files invalidated the manifest. Only the generated sidecars were
  removed, the full manifest passed again, and later inspection used scratch
  copies.
- **18:29–18:38** — Because Nix packages each app from the complete monorepo
  source, the one-file Sites adapter fix changed the store paths for Brain,
  Identity, Core, Phala control, Sites, Hosted Device, and Chat. A new broad
  closure, fresh snapshot, offsite archive, and app-plane restart were required.
- **18:39–18:41** — Scratch preview and synthetic apply both reported 27
  migrations, zero conflicts, and 36 proof-required rows. Legacy share,
  collaborator, project/site access, git credential, and publish-grant diffs
  were zero. The live apply matched; a second pass changed nothing.

### Runtime canaries and host outage

- **18:42–18:55** — The target Runtime was registered and promoted. A new
  lat3-only Agent was created through the normal dashboard and launch-code path.
  It had the exact artifact, the expected owner binding, Hermes connectivity, an active
  Finite Private key, disabled Brain navigation, and a successful browser Chat.
- **18:55–18:59** — The legacy-identity canary upgraded on lat1 and recovered
  its Chat history. The existing Brain canary upgraded on lat3 and recovered
  both Chat history and its Brain conversation.
- **18:59–19:04** — The frozen lat1 plan contained 21 active upgrades, no
  provider exclusions, one already-upgraded canary, and six explicitly inactive
  skips. Seven serial upgrades completed with postflight evidence.
- **about 19:05** — During the eighth entry, lat1 stopped responding to SSH,
  ICMP, Dashboard, and Sites from local, lat2, and lat3. The local driver was
  stopped before a ninth upgrade could be submitted.
- **21:08–21:09** — Latitude showed the machine `OFF`. An operator powered it
  on.

### Recovery and fleet completion

- **21:14–21:17** — Services returned after normal PostgreSQL crash recovery.
  The interrupted Agent's old canonical VM and target candidate had both
  restarted against the same `/data` and npub. The candidate was removed and
  the canonical left serving.
- **21:18–21:21** — Database, filesystem, snapshot, hosted-chat backup, and
  real legacy Finite Private gates passed.
- **21:21–21:25** — The interrupted Agent's endpoint was live but its
  containerd task was orphaned. Normal stop timed out. The exact orphan shim/VM
  and stale Runtime bundle were isolated; the original
  image/data/principal returned normally; the standard one-Agent upgrade then
  passed.
- **21:25–21:28** — Four more Agents upgraded. The wrapper stopped when another
  Agent's old VM returned `ttrpc: closed` during shutdown.
- **21:28–21:35** — That lifecycle-exception Agent failed the same boundary on
  an isolated retry. It was kept on the old image while the eight untouched
  Agents completed serially.
- **21:35–21:47** — Its bounded cold recovery exposed an invalid CNI namespace,
  a wrapped QEMU process missed by the first process check, restart-manager
  races, and finally stale Kata sandbox `persist.json` state. The original
  Agent was restored healthy. A final supported upgrade still failed at
  old-container shutdown, so it stayed on the old artifact.
- **21:49** — Final fleet state: 22/22 active lat1 Agents online; 21 target and
  one prior-known-good; no helpers, active control requests, or failed units.
  The broad lat3 cohort remained unexecuted.

## What went right

### Product and identity design

- Sites now treats verified mailbox identity as authority for its own durable
  keyset without changing Chat's one-address-to-one-npub semantics or Brain's
  exact-npub encryption boundary.
- Reconciliation used only durable proof. Ambiguous rows failed closed as
  `needs_proof`; no key was guessed from a mailbox-like string.
- The reconciled account gained four active Sites keys from existing verified
  evidence while the legacy principal link and all measured access remained
  intact.
- The production Sites adapter failure appeared in preview, before live
  mutation. The failure mode was loud, bounded, and reproducible.
- Chat clarification remained a Hermes adapter concern. No clarification or
  compaction lifecycle type was added to the core Finite Chat protocol.
- Brain navigation remained disabled throughout.

### Release and recovery controls

- Artifacts were built from accepted revisions and recorded by immutable
  digest. Host activation used prebuilt, GC-rooted Nix closures.
- Dry activation exposed the actual restart set before mutation.
- Fresh snapshots and offsite archives existed before each broad app-plane
  switch. Snapshot manifest failures caused a stop and were repaired before the
  snapshot was trusted again.
- Sites reconciliation was previewed, rehearsed on a scratch copy, compared
  against legacy access, applied once, and proven idempotent.
- New-Agent, old-Agent, and existing-Brain canaries ran before broad rollout.
- The rollout plan was immutable, serial, and stopped on the first failed
  entry. Six inactive links did not get silently reactivated.
- Runtime replacement checks preserved the Agent Principal and durable data
  mount. The lifecycle-exception Agent's repeated failure never promoted an
  unverified candidate.
- The host outage did not lead to speculative database repair. PostgreSQL's
  own crash recovery completed and integrity evidence was gathered first.
- Phala Runtime upgrades remained disabled. The monolithic Nix switch did
  restart the Phala control service, but no Phala Agent was selected or moved.

### User-visible result

- Sites publishing and sharing received positive real-user acceptance on
  August 2.
- The Chat focus race has not reproduced in product acceptance testing.
- Existing canaries retained Chat history; the Brain canary retained its Brain
  conversation.
- Finite Private continued to work for legacy Runtime credentials.

## What went wrong

### CI was both fragile and operationally unsafe

Several unrelated failures accumulated in the same serialized Brain lane:
rate limiting, browser network change after a deliberate restart, a deleted
text assertion, a connection/readiness race, and a SQLite observer lock. The
matrix was valuable because it found real cross-product defects, but its
failure signatures were slow and often opaque enough that each required manual
classification before a safe rerun.

The runner host added two avoidable failure classes:

- SSH and CI shared the `ubuntu` login. Systemd `RemoveIPC` made the end of an
  operator session capable of deleting a live CI PostgreSQL process's shared
  memory.
- Scratch workspaces, image layers, and build cache had no effective capacity
  bound. Disk reached zero and the runner vanished mid-job. Cleanup reclaimed
  more than 100 GB twice during the train.

The runner also retained stale `busy` state after jobs, and root-owned scratch
directories blocked later source smokes. These are CI platform defects, not
reasons to weaken product gates.

### A narrow code fix had a broad production blast radius

PR #375 changed one Sites adapter file. Because all Rust applications consume
the complete monorepo source as their Nix input, that revision changed the
closure paths for unrelated services. The fix therefore required rebuilding
and restarting the broad app plane, including Phala control, plus another
snapshot and offsite archive.

This made the deployment classification less useful than intended. “Sites
server fix” described product scope, but not actual restart scope. Small fixes
will continue to carry broad ceremony and rollback surfaces until package
inputs are narrowed.

### Read-only tooling mutated snapshot directories

Twice, SQLite inspection created WAL/SHM sidecars beside an immutable snapshot.
The database contents did not change, but the snapshot manifest correctly
failed. The term “read-only query” was insufficient: opening a database with a
normal SQLite connection can write auxiliary files. Snapshot inspection must
operate on scratch copies or use a proven immutable connection mode.

### Operator probes depended on guessed implementation details

Several checks failed because the command guessed a Sites port, a systemd unit
name, a Borg success marker, the containerd namespace, or availability of
`sqlite3` in the operator profile. An initial Nix store copy also omitted the
documented SSH agent-forwarding trust path. These were false negatives, but
each interrupted the train and consumed attention. A checked-in typed status
command should own canonical service names, ports, namespaces, and asynchronous
unit completion.

### The broad rollout report could misstate interruption

Per-Agent rollout events correctly showed only seven completed broad entries
before the outage. The outer driver nevertheless left a generic final success
marker when it was interrupted. Durable entry events were sufficient to
reconstruct the truth, but a summary that can say “success” without all planned
postflights is unsafe. Interruption, lost SSH, or driver termination must emit
an incomplete/failure terminal state.

### Provider lifecycle state was not self-healing

The cold boot left real VMs serving while containerd, Kata, CNI, and the Runner
disagreed about their state. HTTP 200 was not evidence that the guest-control
path could stop or replace the VM. The two difficult-upgrade Agents exposed
different combinations of:

- a canonical and candidate mounting the same durable data;
- a live orphan VM without a healthy containerd task;
- `ttrpc: closed` during stop;
- stale Runtime bundles and Kata sandbox records;
- an invalid saved network namespace;
- stale veth/IP/CNI cache and port-forwarding state;
- wrapped QEMU process names missed by normal process matching; and
- a restart policy racing manual cleanup.

The existing state machine failed closed, which protected identity and data,
but it could not restore a consistent supported provider topology by itself.

## Latitude power-off analysis

### Facts

- Lat1 became unreachable during the eighth Agent's upgrade after seven
  completed broad entries.
- The failure was host-wide, not an application outage: SSH, ICMP, Dashboard,
  and Sites failed from multiple networks.
- Latitude displayed the server as `OFF`; a user Power On action was required.
- The old journal ends abruptly. `last -x` records the prior boot as ending in
  a crash and records the new boot, but no August 1 orderly shutdown event.
- No kernel panic, kernel OOM, thermal shutdown, watchdog, machine-check, NVMe,
  RAID, filesystem, or storage error was found in the retained OS logs.
- Firecrawl logged that it could not accept a connection due to RAM/CPU load at
  19:02:18, shortly before the host disappeared. The OS did not record an OOM.
- Kata cleanup repeatedly logged cgroup `misc.events: operation not permitted`
  and missing QEMU pid-file warnings during Runtime replacements.
- The host was running 22 Kata VMs while the serial rollout repeatedly stopped
  and started VMs. The rollout was not parallel.
- After power-on, both RAID arrays were healthy, PostgreSQL performed normal
  crash recovery, and no corruption indicators appeared.

### Plausible explanations

| Explanation | Supporting evidence | Contrary or missing evidence | Assessment |
| --- | --- | --- | --- |
| Provider, BMC, or external power action | Latitude recorded the machine as physically `OFF`; OS log ended abruptly | No Latitude organization/BMC event log was available in the session | Plausible; cannot distinguish manual/provider/power hardware without provider telemetry |
| Firmware or hardware protection under transient load | Firecrawl observed high RAM/CPU load; VM replacement creates a transient host workload; OS can disappear before logging a firmware cut | No thermal, MCE, watchdog, storage, or OOM precursor in retained logs | Plausible but unproven |
| Rollout software deliberately shut down the host | Temporal correlation with the eighth Agent upgrade and Kata warnings | Rollout path requests container replacement, not host power-off; no shutdown systemd target or orderly shutdown record | No supporting evidence found |
| Kernel crash too abrupt to persist | Abrupt journal end is compatible with a sudden kernel or hardware failure | No panic record, pstore evidence, or hardware event log was available | Possible, not demonstrated |
| Kata/containerd cleanup defect caused host power-off | Cleanup warnings occurred near Runtime upgrades | Warnings also occur as cleanup symptoms; no evidence connects them to a host power command or kernel fatal path | Weak hypothesis; investigate, do not state as cause |

The correct conclusion is **undetermined power loss during rollout**, not
“rollout caused the shutdown” and not “Latitude randomly powered it off.” The
rollout increased transient workload and was the activity in progress, so it
cannot be exonerated. The absence of a Linux shutdown path and the Latitude
`OFF` state make provider/BMC/power or firmware/hardware explanations at least
as plausible. The missing evidence is the Latitude/BMC event log and hardware
telemetry for approximately 19:00–21:10 UTC.

## Why the difficult Agent upgrades block confidence in Phala

The frightening part was not that one old Runtime remained behind. Mixed
versions were planned and that Agent stayed useful. The frightening part was that
getting back to a supported state required knowledge below the Agent and below
the Runner's provider contract.

For the interrupted-upgrade Agent, the operator had to identify an exact
candidate and canonical, prove they shared `/data` and npub, remove only the
candidate, terminate exact orphan shim/VM processes, and isolate a stale task
bundle. For the lifecycle-exception Agent, the operator had to reason across
the saved network namespace, CNI cache, bridge, veth, IP, QEMU wrapper name,
restart policy, sockets, and Kata `persist.json`. Those actions repaired
compute plumbing, not Agent files, but they were still manual host surgery.

This does not translate to Phala:

- the operator may not have shell access to the worker;
- the provider may not expose containerd, Kata, QEMU, CNI, or process state;
- a live application endpoint may coexist with a broken lifecycle control
  channel;
- “retry upgrade” cannot safely infer whether one or two writers exist; and
- retained provider-local `/data` is not a proven off-host Recovery Set.

Therefore the provider-neutral contract needs to make these outcomes possible
without looking inside the host:

1. identify canonical, candidate, and operation ownership from durable control
   state;
2. observe guest-service health separately from lifecycle-control health;
3. fence duplicate writers using an external lease, not process inspection;
4. abandon and replace irreconcilable compute while preserving a proven
   Recovery Set and exact Agent Principal;
5. prove replacement on an empty target before calling the provider safe for
   upgrades; and
6. report a bounded, resumable failure that leaves the old Agent serving when
   replacement cannot be proven.

Phala upgrades were already disabled pending complete-environment replacement
and rollback canaries. This incident strengthens that gate: even a successful
environment canary is insufficient without opaque-provider lifecycle recovery.

## What felt like too much ceremony

Some ceremony paid for itself and should remain:

- exact immutable artifacts and dry activation;
- preview/synthetic/live reconciliation with access diffs;
- serial canaries and stop-on-first-failure;
- named local and offsite recovery boundaries; and
- explicit separation of inactive links and Phala from the cohort.

Other ceremony was compensating for tooling and coupling rather than reducing
risk:

- Every PR ran seven lanes, then the merge SHA ran them again. Exact-merge
  validation is defensible for the release candidate, but repeated full-product
  reruns became the mechanism for classifying flaky infrastructure.
- A one-file Sites fix rebuilt every Rust application and forced a broad
  restart, fresh snapshot, and new offsite archive.
- Creating the disposable canary required minting and redeeming a launch code
  through the user flow even with an authenticated Admin Ops session.
- Runner drain, timer, artifact pin, and claim state had to be coordinated
  manually across lat1 and lat3. Lat3 had no wrapper equivalent to lat1's
  hash-bound rollout flow.
- Async oneshots such as Borg were checked through guessed immediate state
  before their canonical completion mechanism was understood.
- Canonical ports, unit names, container namespaces, and tool paths were
  rediscovered during the production window.

The answer is not to remove recovery gates. It is to make one reviewed command
produce the exact status, wait for canonical completion, and reuse service-
scoped build inputs so a small change has a small activation boundary.

## What took the most time

1. **CI and release-candidate qualification.** The serialized Brain matrix,
   disk exhaustion, `RemoveIPC`, stale runner leases, rate limit, browser
   network-change race, deletion assertion, SQLite lock, and TypeScript build
   correction consumed most of the pre-production window.
2. **Building broad closures.** Full monorepo source invalidation made both the
   original train and Sites hotfix compile and compare more services than the
   code changes warranted.
3. **Repeated recovery boundaries.** Borg itself took normal tens-of-seconds to
   minutes, but broad restarts forced the entire snapshot/archive gate more than
   once.
4. **The host power interval.** The rollout was paused for about two hours until
   someone with Latitude access could power the host on.
5. **Provider-state recovery.** The interrupted-upgrade Agent took several
   bounded recovery steps; the lifecycle-exception Agent consumed roughly 20
   minutes of increasingly low-level diagnosis and still could not be upgraded.

## Root causes and contributing conditions

### Primary causes

1. **Provider lifecycle state is split and insufficiently recoverable.** Core,
   Runner, containerd, Kata, CNI, and the live guest can disagree after abrupt
   interruption. The supported operation can fail closed but cannot always
   converge them.
2. **No proven opaque-provider Agent recovery path.** Agent data persisted on
   lat1, but off-host per-Agent backup, empty-target restore, and generic
   relaunch preserving the expected Agent Principal are not yet one proven
   operation.
3. **Over-broad monorepo build inputs.** A source revision invalidates unrelated
   service closures, expanding restart and recovery boundaries.
4. **CI runner lifecycle and capacity are unmanaged.** Shared login IPC,
   unbounded scratch/cache growth, ownership drift, and stale listener state
   made CI itself an incident source.
5. **Operational truth is spread across scripts and implementation details.**
   Missing canonical status/wait commands encouraged guessed probes and manual
   coordination.

### Contributing conditions

- The train combined Brain, Sites, Chat, dashboard, CLIs, control-plane
  services, Runtime environment repair, data reconciliation, and fleet
  replacement. This followed the approved plan, but made each late blocker
  expensive.
- The Brain matrix exercises valuable real behavior but has multiple long,
  environment-sensitive boundaries and limited phase-level diagnostics.
- Twenty-two Kata VMs plus other host workloads left less isolation for VM
  replacement transients than a dedicated compute host would provide.
- The rollout wrapper had good entry-level events but incorrect interrupted-run
  summary semantics.
- HTTP application health was treated as an important postcondition, but it did
  not imply that the provider control channel was healthy enough for upgrade.

## Follow-up actions

These are recommendations. This report does not authorize their implementation
or any further production mutation.

### P0 — before any Phala Runtime upgrade

1. Keep Phala upgrades disabled.
2. Define and test the provider-neutral lifecycle states needed for canonical,
   candidate, duplicate-writer fencing, lost-control recovery, and resumable
   failure without host shell access.
3. Prove one Agent's Recovery Set on an empty target: off-host backup, exact
   identity restoration, one writer, useful Chat/Brain/Sites state, and
   rollback. Provider durable volume alone is not the proof.
4. Add a synthetic abrupt-power-loss rehearsal at each replacement phase,
   including candidate started, old stop in progress, and handle swap pending.
5. Require separate application and provider-control health. A live `/contact`
   endpoint must not make an Agent upgrade-eligible when stop/exec/task control
   is broken.

### P0 — before the next broad Kata Runtime rollout

1. File the incomplete-summary defect: a rollout cannot emit terminal success
   unless every planned entry has a successful postflight or an explicit skip.
2. Add a preflight stop/control-channel probe that does not mutate the Agent and
   distinguishes a live guest from an operable lifecycle handle.
3. Turn both difficult-upgrade topologies into synthetic fixtures and prove a
   bounded supported recovery. Do not encode their manual filesystem surgery
   as the normal repair mechanism.
4. Obtain the Latitude/BMC event log for 19:00–21:10 UTC and preserve power,
   thermal, firmware, and hardware telemetry for future outages.
5. Add an explicit host capacity/load gate for Runtime replacement, including
   non-Agent workloads, while preserving serial execution.

### P1 — reduce ceremony without weakening safety

1. Give Nix packages service-scoped source inputs so a Sites-only change does
   not rebuild or restart unrelated applications.
2. Isolate CI jobs from operator logins, either with a dedicated runner account
   or safe `RemoveIPC` policy; never share the active PostgreSQL login session.
3. Bound and monitor lat2 scratch, image, and build-cache usage; clean by policy
   before zero disk takes the runner offline.
4. Make Brain matrix phases independently timed and reported. Keep the real
   browser/Runtime coverage while making 429, readiness, deletion, and teardown
   failures immediately distinguishable.
5. Provide one checked-in rollout status command for canonical services, ports,
   Nix revisions, Runner drain/timer/pin, container namespace/count, RAID,
   queues, snapshot, and offsite archive completion.
6. Inspect snapshots only through scratch copies or a tested immutable SQLite
   helper.
7. Bring lat3 to parity with lat1's hash-bound prepare/execute wrapper.

### Product follow-up, not rollback blockers

- Keep the Chat focus regression coverage. Fix sidebar reordering as a bounded
  presentation issue; do not reopen active-chat ownership.
- Finish Brain acceptance with navigation still disabled. Do not infer
  readiness to enable Brain navigation from CI alone.
- Record the remaining Sites checklist outcomes separately: unshared Request
  Access, persistent viewer session, and exactly-once first-publication email.
- Keep compaction UI parked until Hermes exposes a semantic adapter event; do
  not add prose matching or a core Chat protocol concept.

## Final state

- Live control plane: finite-mono `85d08a486f7876e09fbd6e247e62c0c58a6130f3`.
- Agent Runtime target: `finite-agent-runtime-2026-08-01.1`, immutable digest
  `sha256:8b56ed2125eb03cdbe9c05f7686906ab2db6304a791c5321d6e9ca183c4fcf8f`.
- Lat1 active fleet: 22 online; 21 target, one prior known-good.
- Lat3: new-Agent and existing Brain canaries on target; broad existing-Agent
  rollout not executed and other artifact versions not reconciled by this run.
- Inactive lat1 links: six untouched.
- Phala: no Agent Runtime upgrades; upgrade remains disabled.
- Sites: reconciliation complete and idempotent; product acceptance reports
  publishing and sharing working well.
- Chat: focus race not reproduced; sidebar reorder remains.
- Brain: navigation disabled; validation promising but not complete.
- Data: no observed corruption or identity drift. Existing snapshots and
  offsite archives passed their checks, but a complete per-Agent empty-target
  Recovery Set proof remains outstanding.

## Related documents

- [`identity-rollout-reconciled-plan.md`](../identity-rollout-reconciled-plan.md)
- [`identity-rollout-test-log.md`](../identity-rollout-test-log.md)
- [`agent-runtime-upgrade-rollout-2026-07-16.md`](agent-runtime-upgrade-rollout-2026-07-16.md)
- [`runtime-rollout-gotchas-2026-07-16.md`](../audits/runtime-rollout-gotchas-2026-07-16.md)
- [`phala-confidential-runner-readiness.md`](../runs/phala-confidential-runner-readiness.md)
- [`0001-recoverability-precedes-operator-blindness.md`](../adr/0001-recoverability-precedes-operator-blindness.md)
- [`0004-products-own-bounded-identity-adapters.md`](../adr/0004-products-own-bounded-identity-adapters.md)
- [`runtime-image.md`](../../infra/runbooks/runtime-image.md)
