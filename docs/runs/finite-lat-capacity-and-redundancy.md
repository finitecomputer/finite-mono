# Finite Latitude Capacity and Redundancy

Status: EXECUTING

Owner: Paul

Opened: 2026-07-20

Acceptance: `finite-lat-3` runs the pinned NixOS configuration on mirrored
root and data storage with swap and is the only Runner accepting new Standard-
Agent creation, with a hard limit of 32. `finite-lat-1` remains available for
existing-Agent lifecycle work, and `finite-lat-2` remains CI/build-only.

## Outcome and scope

This run makes the next real reliability improvement:

- a disk failure does not immediately remove the new Agent host;
- host memory pressure can spill into a 64-GiB swapfile instead of forcing an
  immediate OOM;
- new Agent compute and mutable `/data` move off the lat1 control/app host;
- existing Agents stay exactly where they are; and
- failure during the handoff stops new creation without changing existing
  user state.

This run does not add Kubernetes, move existing Agents, make RAID a backup,
redesign recovery, add a scheduler, or qualify Phala. The Latitude Kubernetes
idea is parked in [`parking-lot.md`](parking-lot.md).

PR [#110](https://github.com/finitecomputer/finite-mono/pull/110) governs the
deployment posture: test the real producer/consumer path, make the canary a
real stop point, inspect the exact Nix service impact, and verify the affected
Agent rather than relying on generic health. PR
[#125](https://github.com/finitecomputer/finite-mono/pull/125) remains the
separate Runtime Retirement proposal; no removal or purge behavior is added
here. On 2026-07-20 Paul explicitly chose to skip the synthetic canary and
admission-fence work and open lat3 directly with a 32-Agent hard limit.

## Exact deployed state

| Host | Current role and state | Next change |
| --- | --- | --- |
| `finite-lat-1` (`64.34.82.77`) | NixOS control/app plane plus the Kata Runtimes for every existing Agent. Its Runner timer remains active for lifecycle work, but `FC_RUNNER_DRAIN=true` prevents new creation. The PR #134 WireGuard peer, peer-scoped firewall rules, private Core socket proxy, credential keyring Core, and root-only key are active declarative configuration; no `/run` bridge override remains. Both current system and system profile resolve to the exact merged closure. Root and `/data` remain single-disk. | Keep existing Agents in place. The next destructive storage step requires the accepted backup/empty-target restore gate and a separate maintenance window. |
| `finite-lat-2` (`64.34.80.19`) | Ubuntu/Nix finite-mono CI Runner and sole approved x86_64 production Nix builder. | No role or storage change. Build the reviewed closures here. |
| `finite-lat-3` (`207.188.7.157`) | NixOS `26.05.20260719.fd14620`, kernel 6.18.39. Healthy RAID1 root and `/data`, two ESPs, 64-GiB swapfile and zswap. The merged PR #134 closure is both active and the system profile. WireGuard has a current lat1 handshake and private Core health is 200. `FC_RUNNER_DRAIN=false`, `FC_RUNNER_MAX_SANDBOXES=32`, and the Runner timer is enabled declaratively. Repeated cycles return `idle`; containerd still has zero containers. | Accept up to 32 new Standard Agents. Keep existing Agents on lat1 and lat2 CI/build-only. |

The pinned lat3 nixpkgs revision is
`fd1462031fdee08f65fd0b4c6b64e22239a77870`.

### finite-lat-3 Runner deployment record

- PR [#131](https://github.com/finitecomputer/finite-mono/pull/131) merged as
  `36db4bada0b55bab4ca08b51678231fea4ae06cf` on 2026-07-20.
- `finite-lat-2` built that exact revision with remote builders disabled. The
  initial closure was
  `/nix/store/pran9m5218x8mbsznmp5v4hdd3a4myds-nixos-system-finite-lat-3-26.05.20260719.fd14620`.
- Activation completed through a named transient unit. The post-switch check
  found no failed units, both arrays `[UU]`, 64 GiB swap active, the storage
  health check successful, containerd active with zero namespaces or
  containers, and public SSH available.
- Before the private connection, the Runner remained fail closed: its timer
  was not attached to a target, its service was inactive, and its required
  root-only environment file was absent.
- Lat1's full candidate closure would restart more application services than
  this run accepts because Finite packages still hash the monorepo source as a
  whole. Do not switch that closure.

### Drained private connection record

- The bounded 2026-07-20 activation used runtime-only networkd files, two exact
  peer-scoped iptables rules, and a transient socket proxy. It did not restart
  systemd-networkd or change the public Core binding. A reboot intentionally
  removes this temporary network/proxy state and is therefore a stop, not a
  rollback step.
- The exact PR #131 Core package is
  `/nix/store/0sp1fcwpbrkgdj12165mq4qxhclq44y9-finite-saas-core-0.1.0`.
  A `/run` drop-in changed only Core's `ExecStart`. The combined legacy-lat1
  plus unique-lat3 credential environment passed an offline parse before one
  intentional Core restart. The byte-identical prior environment is retained
  root-only at `/var/lib/finite-lat3-canary/core.env.before-lat3`.
- Lat3 received the unique Runner token and unchanged Runtime secrets directly
  file-to-file. Both environment files are `root:root 0600`; drain is set in
  the Runner environment and forced again by a later runtime-only environment
  file. The timer is inactive, absent from `timers.target`, and refuses manual
  start.
- One manual Runner invocation authenticated through
  `http://10.254.3.1:14200`, reported `runner is draining`, advertised a
  six-sandbox maximum with zero active sandboxes, and exited successfully.
- Before and after that invocation, all 34 existing Runtime rows, all creation
  and control rows, and all 27 existing lat1 containers had identical ordered
  hashes. There were zero pending creation/control requests, zero lat3
  containers, and no Chat, Hosted Device, containerd, Postgres, or existing
  Runtime restart. Core was the only intentionally restarted service.
- This was the original stop before a synthetic Agent. Paul subsequently
  waived that canary and admission-fence gate for this opening.

### Reboot-persistence record

- PR [#134](https://github.com/finitecomputer/finite-mono/pull/134) merged as
  `f5feb0401f0264ea00f10e23ac8877d37680bbbd`. It retains lat3-only firewall
  admission while declaring the existing lat1 WireGuard peer and private Core
  socket proxy.
- Lat2 built the exact merged lat3 closure
  `/nix/store/7mz7h9s5w4c69rf8hlsmch316xa25hrc-nixos-system-finite-lat-3-26.05.20260719.fd14620`.
  Dry activation proposed no service stop, restart, reload, or start. Lat3 was
  switched to that closure, and both `/run/current-system` and the system
  profile resolve to it.
- The runtime-only lat3 timer symlinks were removed. The active timer is now
  enabled through `/etc/systemd/system/timers.target.wants`, its fragment is
  from the merged closure, repeated Runner cycles return `idle`, private Core
  health is good, and containerd still has zero containers.
- Lat1's live WireGuard key was copied byte-for-byte to
  `/etc/finite/wireguard-private-key` as `root:root 0600`. No unit was reloaded
  or restarted.
- Lat1 candidate
  `/nix/store/i81dpv94lx2ppnmgc0n7kpz22zrvsv5p-nixos-system-finite-lat-1-25.11.20260630.b6018f8`
  initially failed the bounded live-switch gate. Dry activation would stop and start
  Brain, Core, Sites, Hosted Device, and Chat, restart systemd-networkd, and
  reload the firewall. The five application unit diffs each select a different
  binary store path, so this is a real broad rollout rather than harmless unit
  relinking. The rollout stopped until Paul explicitly authorized the broader
  evening activation.
- Immediately before activation, the service-consistent Hosted Web Chat
  snapshot completed at
  `/data/recovery-snapshots/hosted-web-chat/20260721T013053Z`; the old system
  closure and all runtime bridge files were retained as the rollback boundary.
  Both Runner timers were stopped with their services idle.
- The first activation completed the application and network switch, but its
  newly enabled lat1 Runner timer fired before Core finished binding. That one
  drained Runner cycle received connection-refused and made the activation
  wrapper report failure. Core and every application were already healthy;
  the Runner was reset and then repeatedly returned authenticated
  `runner is draining`. No rollback or durable-state repair was required.
- Runtime network files, the Core `ExecStart` drop-in, transient proxy, and old
  firewall rules were removed. The active networkd units, private proxy socket,
  Core unit, and peer-scoped firewall rules now come only from the system
  closure. A second dry activation was empty.
- The PR-head closure embedded `49b94bb` in FiniteChat. Lat2 therefore built
  exact merged revision `f5feb0401f0264ea00f10e23ac8877d37680bbbd` as
  `/nix/store/yrpvrnp8a3h65hcmnk46jgxg563fmp28-nixos-system-finite-lat-1-25.11.20260630.b6018f8`.
  Its only live delta was FiniteChat's source revision. After a fresh snapshot
  at `/data/recovery-snapshots/hosted-web-chat/20260721T013700Z`, that closure
  was activated; Chat reports `source_commit=f5feb0401f02`.
- Final verification found zero failed units; all Core, private Core, Chat,
  Hosted Device, Brain, and Sites health checks pass. All 34 Runtime rows
  remain. Creation/control ordered hashes and the exact 27-container/task
  inventories match pre-activation; Postgres, containerd, and Caddy did not
  restart. Lat1 remains creation-drained, while lat3 is undrained at its hard
  limit of 32.
- Result: both halves of the private bridge and both Runner schedules are now
  reboot-persistent and declarative.
- PR [#153](https://github.com/finitecomputer/finite-mono/pull/153) was deployed
  from merged revision `8bb4e47b991f675eb84b47b7331d23809da5c241` after the
  pre-deploy recovery snapshot `20260721T190542Z`. Both the system profile and
  `/run/current-system` resolve to
  `/nix/store/2pr6p55l9wjwcv8as6kai91kg6vvlnkd-nixos-system-finite-lat-1-25.11.20260630.b6018f8`.
  Core and the public dashboard edge passed health checks after the switch.

### finite-lat-3 storage truth

- Root RAID1: 443 GiB component size, MD UUID
  `6ad37071:48614192:806ab364:80f0b8f0`, ext4 UUID
  `0e236c38-bbef-495a-b604-964a53ae6d22`.
- Data RAID1: 1,785 GiB component size, MD UUID
  `9c13aa78:0e674937:bd2e179c:59216802`, ext4 UUID
  `6661e69b-efbe-4c71-9e65-c0ed0f653e5b`.
- Root members:
  `/dev/disk/by-id/nvme-eui.000000000000000100a075255199d70f` and
  `/dev/disk/by-id/nvme-eui.000000000000000100a075255199d6cc`.
- Data members:
  `/dev/disk/by-id/nvme-eui.000000000000000100a075254fa09807` and
  `/dev/disk/by-id/nvme-eui.000000000000000100a075254fa098c7`.
- Both arrays report `[UU]`, idle, with mismatch count zero.
- Each array completed a full member removal, rebuild, degraded boot, and
  checksum comparison. Both ESP identities and fail-before-write guard pass.
- `/swapfile` is active at 64 GiB. zswap uses zstd with a 10% maximum pool;
  `vm.swappiness=20`.

These facts are enough to proceed. More soak time, temperature logging,
performance benchmarking, physical power cuts, and additional member drills
are not gates for the Runner slice.

## Placement decision

- lat1 remains the control/app plane and lifecycle Runner for existing Agents.
- lat3 becomes the only initial creator for new Standard Agents.
- lat2 remains CI/build-only.
- Existing Runtimes keep their persisted source host and source machine. There
  is no automatic failover or migration.
- Exactly one Standard Runner is undrained for creation during this rollout.
  The current queue accepts untargeted creation work, so active-active creation
  waits for a later, durable source-host reservation path.
- A lat3 outage closes new admission. It never automatically undrains lat1.
- The hard maximum is 32 Standard Agents at 4 vCPU and 8 GiB each.
  Swap is not counted as Agent capacity.

This keeps product concepts provider-neutral: Core still owns Hosting Tier,
Runner Class, Runtime Resource Class, and source-host binding. Latitude remains
an implementation detail, preserving future iOS, Electron, Brain, Sites, and
confidential Runner work.

## The only required cross-host change

Two current loopback assumptions prevent a remote Runner:

1. Core listens only on lat1 loopback.
2. Kata binds and advertises each Runtime on host loopback.

Use a WireGuard point-to-point overlay:

- lat1: `10.254.3.1/30`, public underlay `64.34.82.77:51820`;
- lat3: `10.254.3.2/30`, public underlay `207.188.7.157:51820`;
- peer routes are only the opposite `/32`; no default route or forwarding;
- private keys are root-only host files; public keys may be committed;
- lat1 exposes `10.254.3.1:14200` through
  `systemd-socket-proxyd` to `127.0.0.1:4200`;
- lat3 binds dynamic Kata Runtime ports only to `10.254.3.2`; and
- the lat3 firewall admits those ports only from its authenticated WireGuard
  peer.

Core receives a new credential bound to:

- Runner ID `finite-kata-runner-3`;
- source host `finite-lat-3`; and
- Runner class `kata`.

The existing lat1 credential remains valid during expansion. Core requires one
brief restart to load the new keyring entry. No database schema or Runtime
image changes.

## Runner candidate configuration

The lat3 Runner uses:

- the same promoted immutable Runtime artifact as lat1;
- `FC_RUNNER_WORK_ROOT=/data/finite-saas-runner`;
- `FC_RUNNER_KATA_HOST_ADDRESS=10.254.3.2`;
- the existing public Sites, Brain, Chat, and Identity endpoints;
- a direct copy of the current runtime secret file, never repository values;
- `FC_RUNNER_DRAIN=false`;
- `FC_RUNNER_MAX_SANDBOXES=32`; and
- an active recurring Runner timer.

The Nix Kata configuration uses the declared Standard shape of 4 vCPU and
8 GiB. Agent state is a host bind beneath `/data`; replacing compute must not
remove that directory.

## Build and deployment gates

Before any live switch:

1. merge and test the reviewed branch;
2. build exact lat1 and lat3 closures on lat2 with remote builders disabled;
3. compare lat1's current and candidate closures;
4. stop if Dashboard, Chat, Hosted Device, Sites, Brain, Search, Postgres,
   Caddy, or any existing Runtime would restart unexpectedly; and
5. record the exact source revision and closure paths.

The expected lat1 impact is network configuration, the private Core proxy,
Runner/Core executable or unit changes explicitly present in this run, and one
intentional Core restart for the credential. A broader service restart set is
an escalation, not an accepted surprise.

## Drained bring-up

1. Deploy the lat1 overlay and private Core proxy without changing Runner
   drain state.
2. Deploy the lat3 closure with its Runner timer disabled.
3. Install `/etc/finite/runner.env` and `/etc/finite/runtime-secrets.env` as
   `root:root 0600` without printing values.
4. Bring up WireGuard and require a current handshake in both directions.
5. From lat3, reach Core only through `http://10.254.3.1:14200`.
6. Invoke the lat3 Runner oneshot with `FC_RUNNER_DRAIN=true`.

Pass requires an authenticated `CapacityUnavailable` result with `draining`
true, no creation lease, no provider/container mutation, no new Core Runtime,
and no change to lat1 Agents.

Rollback is to stop the lat3 Runner, disable its timer, and remove only the
private overlay activation. Existing Agents are not involved.

## Superseded one-Agent handoff

This canary plan was not executed. Paul explicitly waived it on 2026-07-20 in
favor of opening lat3 directly with the hard 32-Agent limit. It remains below
only as historical rollback context.

1. Confirm zero pending/in-flight creation requests and zero conflicting
   runtime operations.
2. Set lat1 `FC_RUNNER_DRAIN=true`; wait for the next oneshot cycle and verify
   it no longer accepts creation while lifecycle work remains available.
3. Keep the lat3 timer disabled.
4. Create exactly one Finite-owned synthetic Agent request.
5. Set lat3 drain false for one manual oneshot invocation only.
6. Immediately return lat3 to drain after the request is claimed.
7. Verify Core records `source_host_id=finite-lat-3`, the contact URL uses
   `10.254.3.2`, the canonical container and writable `/data` exist only on
   lat3, and no new lat1 container appeared.
8. Complete multiple chat turns and one attachment.
9. Restart the Runtime, then reboot lat3 once. Require the same Agent
   Principal, ordered chat history, attachment, workspace, and writable data.

Pass means exactly one synthetic Agent exists on lat3 and existing lat1 Agents
continued operating unchanged.

On failure, drain lat3 and stop its timer. Preserve the synthetic Runtime and
`/data` for diagnosis; do not delete, move, or reconstruct it manually. Lat1
existing-Agent lifecycle remains active. Reopening lat1 creation is an
explicit operator decision after the queue is clear.

## Public opening

Executed on 2026-07-20:

- enable the lat3 Runner timer;
- set lat3 creation drain false;
- keep lat1 creation drain true while its lifecycle operations remain active;
- keep the 32-Agent hard maximum; and
- retain the source revision, closures, Runner result, Core binding, Runtime
  artifact/digest, and rollback state in this run record.

No synthetic-Agent chat/restart/reboot result is claimed.

### Existing-Agent empty-target recovery proof

On 2026-07-20 Paul authorized the Agents owned by `paul@finite.vip` as
disposable recovery-test candidates, specifically Sol 2, Waffle, AEON Canary
0714, and Sites Canary 0715. This authority does not activate Runtime
Retirement, change Core records or entitlements, or authorize deletion. The
separate proposal remains
[`runtime-retirement-readiness.md`](runtime-retirement-readiness.md).

AEON Canary 0714 was selected because it was online on the current Runtime
artifact; Waffle was stale, Sol 2 used an older artifact, and Sites Canary
would add publishing side effects. The bounded drill passed:

- the lat1 Runner timer was paused, the one canonical container was gracefully
  stopped, and its 35-MiB `/data` Recovery Set was archived while no writer was
  running;
- the authoritative container was immediately restarted and returned healthy
  before the Runner timer resumed;
- the checksummed artifact was copied off lat1 to a root-only directory on
  lat3 and extracted only into an empty target;
- the exact pinned image booted the restored state in a Kata VM with container
  network mode `none`; an explicit outbound connection probe failed;
- the restored and authoritative instances reported matching Agent Principal
  hashes and matching identity-file hashes, all four SQLite candidates passed
  `PRAGMA integrity_check`, and the restored tree matched the archive's 328
  files and 143 directories; and
- the restored VM, transient environment, and extracted tree were removed.
  Lat3 returned to zero containers and tasks. Root-only copies of the
  36,024,320-byte artifact remain at
  `/data/recovery-snapshots/agent-runtime/20260721T014619Z-aeon-canary-0714`
  on lat1 and
  `/data/recovery-drills/agent-runtime/20260721T014619Z-aeon-canary-0714`
  on lat3, with SHA-256
  `4265110a1aab16a4b7fac27df65e95237f5f68017be91609b4d29cc792400af3`.

The first isolated cleanup exposed a bounded Kata/nerdctl issue: VM cleanup
timed out and left one stopped container metadata record plus a stale local
name reservation. There was no task or second writer. The exact metadata
record was removed, a fresh drill name completed, and the stale name is not
reused. This does not invalidate the restored data or identity proof.

This proves the Agent Runtime portion for one representative Agent. It is not
yet authority to wipe lat1: the complete fleet artifact, Hosted Web Chat/Core
empty-target restore, Sites and Brain recovery sets, secrets bootstrap, and
the exact lat1 RAID closure remain separate go/no-go gates.

The follow-up in-flight-chat drill on 2026-07-21 established the narrower
recovery boundary needed for that gate:

- a deterministic local matrix used the canonical Runtime image, real Hermes
  0.18.2, real Finite Chat MLS state, and a local streaming provider held after
  response headers but before its first data frame;
- graceful stop, `SIGKILL`, and a stopped-writer empty-target restore each
  preserved the Agent Principal and Room and completed two new chats after
  restart;
- AEON Canary 0714 was stopped while a provider response was in flight, then
  restored into an empty canonical state path from the off-host-verified
  archive. Its four SQLite databases passed `PRAGMA integrity_check`, and the
  restored file and directory manifests exactly matched the stable source;
- the first archive attempt was rejected before launch because Kata task
  cleanup continued after `nerdctl stop` returned and changed WAL/SHM files.
  The required snapshot gate is therefore: the container is stopped, its
  containerd task is absent, and two source manifests are stable before the
  archive begins; and
- after restore, Paul's existing Hosted Web device decrypted one AEON response
  containing both the interrupted prompt's requested marker and the first
  fresh-chat marker, then decrypted a second independent reply in the same
  chat. AEON remained healthy and connected after normal Runner reconciliation
  resumed.

The stable root-only artifact is retained on lat1 at
`/data/recovery-snapshots/agent-runtime/20260721T141627Z-aeon-canary-0714-inflight-stable`
and off-host on lat3 at
`/data/recovery-drills/agent-runtime/20260721T141627Z-aeon-canary-0714-inflight-stable`.
Its archive SHA-256 is
`f6eb8fb1e34c68e9b52535276b2bc32310490a066d82c4460df710fa697e4a4c`.
The earlier artifact is marked `REJECTED` at both locations and must never be
used as a restore source.

The Docker matrix fences its source by removing the container and unmounting
the named volume before `tar`; it does not model Kata's delayed task cleanup.
The AEON task-absence and stable-manifest evidence is the production snapshot
gate.

This does not promise delivery of every in-flight token or message. It proves
the supported boundary that matters here: snapshot only stable, stopped-writer
state; preserve both sides' current Finite Chat state; after restore require
the same identity and Room plus two fresh decryptable chats. A live filesystem
copy, a snapshot taken while Kata cleanup is still changing the tree, or an
older Agent snapshot after the peer has advanced its MLS state remains outside
the guarantee.

## Next bounded slices

### Capacity admission

As a later bounded product slice, make checkout, Launch Code redemption, and
Agent creation fail closed when no creation slot is reserved. Signup, login,
existing-Agent access, and Contact Finite remain available. The UI should say
capacity is full and direct the user to contact Paul. This was not added as
part of opening lat3.

### finite-lat-1 improvement

After lat3 is stable capacity:

1. add swap and pin the next lat1 NixOS generation without changing storage;
2. extend the passed single-Agent empty-target proof to the complete lat1
   Recovery Set, including chat-safe SQLite handling;
3. schedule the destructive lat1 reprovision for an evening window; and
4. restore and verify before reopening normal operation.

For SQLite-backed chat state, either pause the owning writer for a bounded
snapshot or use SQLite's supported online backup mechanism while preserving
its WAL relationship. Copying a live database file alone is not accepted.
That restore work does not expand the lat3 Runner canary.

`finite-lat-2` remains untouched throughout.

## Execution record

Append only decisive checkpoints here:

| UTC | Checkpoint | Result |
| --- | --- | --- |
| 2026-07-20 | Installed pinned NixOS on RAID1 root/data with dual ESPs and 64-GiB swap | Pass |
| 2026-07-20 | Full member rebuilds, degraded boots, checksums, ESP guard, and swap activation | Pass |
| 2026-07-20 | PR #110 and PR #125 merged | Pass |
| 2026-07-20 | Exact lat3 closure build and bounded activation | Pass |
| 2026-07-20 | Drained lat3 Runner authentication | Pass |
| 2026-07-20 | Open lat3 for creation; drain lat1 creation; hard maximum 32 | Pass |
| 2026-07-20 | One synthetic Agent handoff and persistence check | Waived by owner |
| 2026-07-20 | PR #134 merged; exact lat3 closure activated with declarative Runner timer | Pass |
| 2026-07-20 | Exact lat1 persistence closure dry activation | No-go: broad application restart set |
| 2026-07-21 | Owner-authorized evening lat1 rollout; exact merged closure and declarative bridge | Pass |
| 2026-07-21 | AEON Canary 0714 write-fenced archive and network-isolated empty-target restore on lat3 | Pass |
| 2026-07-21 | Real-Hermes interruption matrix: graceful, `SIGKILL`, and empty-target restore | Pass |
| 2026-07-21 | AEON in-flight stop, stable empty-target restore, and two fresh decryptable chats | Pass |
