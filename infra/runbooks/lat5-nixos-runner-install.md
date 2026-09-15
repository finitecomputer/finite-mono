# finite-lat-5 setup

Execution plan for [FIN-74](https://linear.app/finitecomputer/issue/FIN-74).
Status on 2026-09-15: provisioned in Chicago, SSH and initial hardware capture
complete. NixOS configuration and artifact tooling are drafted. Nix evaluation,
CI packaging, install, overlay admission and launch qualification remain open.
No Agent Runtime has been created on lat5.

Use lat4's dedicated NixOS/Kata Runner architecture with an empty `/data`.
Lat2 remains the app plane and WireGuard hub. Existing Agent Runtimes remain
on their current hosts during setup.

## 1. Provisioned hardware and evidence

Austin authorized continuing the prepared 128 GB order on 2026-09-15.
Latitude delivered **192 GB**, confirmed over SSH as four 48 GB DIMMs and
201,653,641,216 bytes of usable memory. The delivered server detail page still
shows **$555/month**. The order listing is not a reliable hardware inventory.

| Field | Captured value |
| --- | --- |
| Server | `finite-lat-5`, `sv_6B9VaL7GE57vr`, Chicago, Default Project |
| CPU | AMD EPYC 4564P, 16 cores / 32 threads; AMD-V and `/dev/kvm` available |
| Memory | 192 GB installed; replaces the earlier 128 GB capacity assumption |
| Boot disks | 2 × Micron 5400 SATA SSD, 480,103,981,056 bytes each |
| Data disks | 2 × Micron NVMe, 7,681,501,126,656 bytes each |
| Logical sectors | 512 bytes on all four disks |
| IPv4 | `64.34.93.213/31`, gateway `64.34.93.212` |
| IPv6 | `2605:6440:2004:1c6::2/64`, gateway `2605:6440:2004:1c6::1` |
| WAN / other NIC MAC | `90:5a:08:30:f9:ff` / `90:5a:08:30:f9:fe` |
| Interim OS / access | Ubuntu 24.04, `ubuntu@64.34.93.213`, Austin SSH key verified |

Capture: `infra/nixos/scripts/capture-lat5-host-evidence
ubuntu@64.34.93.213`; local output:
`target/lat5-host-evidence-20260915T143012Z/evidence.txt`.
The helper writes an incomplete storage skeleton; it never approves a wipe.
Disk paths in `infra/nixos/hosts/finite-lat-5/storage-ids.nix` were checked
against this capture and use ATA identities for boot disks and NVMe EUI
identities for data disks. Filesystem, partition and RAID IDs are fresh.

All four initial SMART health checks passed. NVMe media errors and critical
warnings are zero. Retain the boot SSD baseline: historical command timeouts
94/92 and CRC errors 0/2. A passed SMART summary does not replace burn-in;
check for new errors during storage and workload qualification. Hardware
inspection packages were installed only on the disposable Ubuntu system.

Root geometry follows lat4 and fits the captured SATA devices. Data partition
end is 14,990,450,687 (512-byte sectors), with a 7,486,832,640 KiB mirrored MD
allocation, fitting both larger NVMe devices. Prove array creation, storage
health and both ESP boot paths on synthetic disks and then this empty host.
Do not silently fall back to lat4's smaller data allocation.

Support ticket **832519** was opened from `austin@finite.vip` to
`support@latitude.sh` and routed to Sales. The original request asks about the
128/192 GB listing and unchanged price. The actual lat5 delivery now proves
that this order received 192 GB; it does not establish a guaranteed standard
configuration for other orders. Their response does not block setup.

Pre-provision `scripts/finite-status` evidence: lat2 app HTTP/Chat, storage and
recovery checks passed; lat3 had 31 and lat4 27 active, ready Agent Runtimes.
Fleet convergence was red because of mixed Runtime artifact versions. Some
host inventory probes were unknown due to missing container tools, so this
was not an all-green fleet baseline. Preserve that distinction after rollout.

## 2. Implement and verify the host configuration

Use `infra/nixos/hosts/finite-lat-4/` as the reference and reuse the shared
`kata-runner-host.nix` module. Preserve mirrored root and `/data`, dual ESPs,
storage health checks, boot guards and the pinned kernel/installer pairing.

Give lat5 fresh filesystem, partition and RAID identities; its own hostname,
Runner ID, source-host ID, WireGuard key and verified unused overlay address;
and the public networking captured in step 1. Validate disk geometry against
the physical sector counts. Keep storage capture marked incomplete until
the evidence is checked. Choose capacity from lat5 qualification; lat4's
42-sandbox setting is an overcommit policy, not a measured lat5 capacity.

Add the flake system, disko and kexec outputs, CI closure packaging,
artifact-only install/deploy drivers and root just recipes following lat4's
pattern. Artifact validation must reject incomplete capture, a different
host, an invalid manifest or an unmerged revision before contacting a target.
Keep the no-build-fallback contract. Extend the relevant artifact, Runner,
monitoring and CI harness checks. Add lat5 to the fleet status inventory
through `scripts/finite-status`'s existing contract.

Complete when those checks pass and CI packages an exact merged revision
with matching system, disko and kexec paths. Record the reviewed install
command here before executing it.

## 3. Install and connect while drained

Run `scripts/finite-status` before the rollout. Record the existing lat2
closure and off-host recovery boundary before changing its configuration.
Add lat5's peer-scoped WireGuard access and unique Core Runner credential
through the infrastructure rollout. Verify existing Runner connectivity after
the hub change.

Verify the empty target again before the authorized wipe/install. Consume
only the CI artifact from a Linux driver. Follow lat4's storage and boot
qualification in [the lat4 install runbook](lat4-nixos-runner-install.md),
substituting the reviewed lat5 identities and artifact tooling. Treat its
historical migration section as a separate operation.

Stage credentials by name and secure file transfer, with root-only access:
`runner.env`, `identity-operator.env`, `runtime-secrets.env`,
`wireguard-private-key`, `metrics-remote-write.env` and `logs-write.env`
under `/etc/finite/`. Use a unique Runner credential and WireGuard private
key. Pin the current promoted runtime artifact and set `FC_RUNNER_DRAIN=true`.

Complete when lat5 boots the exact artifact, both arrays are healthy, both
ESPs and storage identities pass validation, storage/boot refusal checks
pass, and Core authenticates its drained Runner with zero agents created.
Verify metrics, logs and `scripts/finite-status` after the rollout.

Before first admission, recovery is a reinstall of this empty host plus its
escrowed configuration and credentials. Revert a failed lat2 change to its
recorded closure and credential configuration. Once lat5 holds user state,
reinstallation requires a verified Recovery Set and restore proof; the empty
host recovery boundary no longer applies.

## 4. Qualify launch and release capacity

Use an explicit test account to prove enrollment, Agent admission, launch,
identity readiness and a Chat reply. Verify the actual destination through
the supported placement path: a Launch Code grants entitlement and does not
itself select lat5. Establish this path before inviting TRF IT to create the
cohort. Begin with one canary, then qualify the intended workload in stages.
The earlier 24–28-runtime estimate was for the advertised 128 GB hardware
and is superseded. The captured host has 192 GB, like lat4. Start with the
Nix-owned one-runtime ceiling and drain enabled; lat4's 42-runtime setting is
only a qualification target, not proof of capacity on this host.
Measure host and guest memory, CPU pressure, launch failures and Chat behavior
under representative concurrent load. Choose the admission ceiling from that
evidence before opening the host to the cohort. Record whether lat6 is needed.

Complete FIN-74 when new agents reliably land on lat5 and pass those checks
at the accepted capacity. Then TRF IT can create empty Agent Runtimes and
hand them back to Austin for migration. Retain suitable prepared destinations
on other hosts, including Rene's existing slot.

Source Telegram bots stay running during host setup and migration rehearsal.
Each later migration owns a brief, single-consumer Telegram switch and Google
reconnection under the [shared fleet plan](https://linear.app/finitecomputer/document/trf-fleet-migration-plan-and-evidence-21410d33bc9b).

## Lat1 reuse assessment

Austin requested that lat1 conversion be considered alongside lat5. Core's
2026-09-15 status snapshot shows zero active links and seven intentionally
inactive Runtime records on `finite-lat-1`. Latitude still reports rescue mode.
Neither observation proves that its disks contain no retained recovery data.

[FIN-71](https://linear.app/finitecomputer/issue/FIN-71) records retirement on
September 24; leave that scheduled while assessing reuse. The earlier failure
included repeated thermal shutdowns, including after provider maintenance.
A software reinstall is not evidence of repaired hardware.

Before proposing conversion: verify the rescue SSH host key through a trusted
provider console (the current SSH key differs from the saved production key),
inventory retained disks without mounting them writable, reconcile all retained
state with independently verified Recovery Sets and restore proof, and obtain
provider repair evidence plus sustained thermal, memory and storage qualification.
If those checks pass, compare retaining this Dallas host with its scheduled
retirement. Retaining it, cancelling retirement, wiping it and admitting agents
are separate decisions; none occurred during this assessment.
