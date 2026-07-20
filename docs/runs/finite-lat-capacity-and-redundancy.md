# Finite Latitude Capacity and Redundancy

Status: EXECUTING

Owner: Paul

Opened: 2026-07-20

Acceptance: `finite-lat-3` runs the pinned NixOS configuration on mirrored
root and data storage with swap, then launches one synthetic Standard Agent
through a private, source-host-bound Runner path without moving, restarting,
or rewriting any existing Agent. After that canary passes, lat3 becomes the
only Runner accepting bounded new Standard-Agent creation while lat1 continues
lifecycle control for its existing Agents and lat2 remains CI/build-only.

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
here.

## Exact deployed state

| Host | Current role and state | Next change |
| --- | --- | --- |
| `finite-lat-1` (`64.34.82.77`) | NixOS control/app plane plus the Kata Runtimes for every existing Agent. Core binds `127.0.0.1:4200`. The Kata Runner timer is active. Root and `/data` are single-disk. | Add only the private lat3 peer, private Core proxy, and a distinct lat3 Runner credential. Drain new creation only after lat3 proves the drained path. No storage or existing-Agent change in this slice. |
| `finite-lat-2` (`64.34.80.19`) | Ubuntu/Nix finite-mono CI Runner and sole approved x86_64 production Nix builder. | No role or storage change. Build the reviewed closures here. |
| `finite-lat-3` (`207.188.7.157`) | NixOS `26.05.20260719.fd14620`, kernel 6.18.39. Healthy RAID1 root and `/data`, two ESPs, 64-GiB swapfile and zswap. The merged Runner/Kata closure is active; containerd is active with zero containers, while the Runner timer and service are inactive and `/etc/finite/runner.env` is absent. The lat3 WireGuard peer is configured but cannot handshake until lat1 is activated. No user Agent or Recovery Authority exists here. | Connect lat1 through the already-declared private peer and Core proxy without a broad application restart; prove the drained path and one synthetic Agent before accepting bounded new creation. |

The pinned lat3 nixpkgs revision is
`fd1462031fdee08f65fd0b4c6b64e22239a77870`.

### finite-lat-3 Runner deployment record

- PR [#131](https://github.com/finitecomputer/finite-mono/pull/131) merged as
  `36db4bada0b55bab4ca08b51678231fea4ae06cf` on 2026-07-20.
- `finite-lat-2` built that exact revision with remote builders disabled. The
  active and system-profile closure is
  `/nix/store/pran9m5218x8mbsznmp5v4hdd3a4myds-nixos-system-finite-lat-3-26.05.20260719.fd14620`.
- Activation completed through a named transient unit. The post-switch check
  found no failed units, both arrays `[UU]`, 64 GiB swap active, the storage
  health check successful, containerd active with zero namespaces or
  containers, and public SSH available.
- The Runner remains fail closed: its timer is not attached to a target, its
  service is inactive, and its required root-only environment file is absent.
- Lat1 has not been changed. Its full candidate closure would restart more
  application services than this run accepts because Finite packages still
  hash the monorepo source as a whole. Do not switch that closure. The next
  action requires an explicitly bounded lat1 activation or a separately
  reviewed package-source fix.

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
- The initial hard maximum is six Standard Agents at 4 vCPU and 8 GiB each.
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
- `FC_RUNNER_DRAIN=true` initially;
- `FC_RUNNER_MAX_SANDBOXES=6`; and
- a disabled timer until the canary passes.

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

## One-Agent handoff

Public paid and Launch Code admission remain closed during the canary.

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

## Promotion after the canary

After the canary passes:

- enable the lat3 Runner timer;
- set lat3 creation drain false;
- keep lat1 creation drain true while its lifecycle operations remain active;
- keep the six-Agent hard maximum; and
- retain the source revision, closures, Runner result, Core binding, Runtime
  artifact/digest, Agent Principal, chat/restart/reboot result, and rollback
  state in this run record.

This makes lat3 useful internal capacity. It does not by itself reopen public
self-service.

## Next bounded slices

### Capacity admission

Before accepting more public users, make checkout, Launch Code redemption, and
Agent creation fail closed when no creation slot is reserved. Signup, login,
existing-Agent access, and Contact Finite remain available. The UI should say
capacity is full and direct the user to contact Paul. This is a separate code
and product slice after lat3 works; it is not a prerequisite for the internal
canary while public admission remains closed.

### finite-lat-1 improvement

After lat3 is stable capacity:

1. add swap and pin the next lat1 NixOS generation without changing storage;
2. prove the existing backup and restore path, including chat-safe SQLite
   handling;
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
| pending | Exact lat1/lat3 closure build and impact review | Open |
| pending | Drained lat3 Runner authentication | Open |
| pending | One synthetic Agent handoff and persistence check | Open |
