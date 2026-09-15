# finite-lat-5 setup

Execution plan for [FIN-74](https://linear.app/finitecomputer/issue/FIN-74).
Status on 2026-09-15: provisioned in Chicago, SSH and initial hardware capture
complete. NixOS configuration and artifact tooling are drafted. All PR CI checks,
including Nix evaluation, passed at revision 5934f626. A synthetic storage/boot qualification gate and a fully packaged installer
are being added before installation; CI packaging, install, overlay admission
and launch qualification remain open.
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
configuration for other orders. Latitude replied on September 15 that the
standard configuration changed to 128 GB at the same price because hardware
costs increased; some older 192 GB units remain. This does not block setup.

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

### Reviewed OS installation procedure

Austin authorized steps 1–2 end to end on September 15. This operation replaces
only the disposable Ubuntu installation on `sv_6B9VaL7GE57vr`,
`ubuntu@64.34.93.213`. The separate `lat5-installer` Linux container consumes
CI closures; no source build runs on lat5 or an existing production server.

The `lat5-storage-boot` check executes the production Disko geometry and UUIDs
on sparse virtual media with the pinned 6.18 kernel, mirrored GRUB and the
production ESP guard. It boots independently from A and B and exercises
missing/wrong ESP and degraded-array refusal. Only virtual media use
`--assume-clean`; physical RAID synchronization and SMART are host gates.
The synthetic OS omits runtime services and the 64 GiB swap allocation;
those remain physical-host verification. CI must pass this check before merge
and before publishing the installation artifact.

After merging, record the exact revision and workflow run below, then use:

```sh
gh workflow run lat5-nixos-closure.yml --ref "$REV" -f rev="$REV"
gh run download "$RUN_ID" --name "lat5-nixos-closure-$REV" \
  --dir "target/lat5-nixos-closure-$REV"
scripts/install-lat5-from-artifact "target/lat5-nixos-closure-$REV" \
  ubuntu@64.34.93.213 --validate-only
scripts/install-lat5-from-artifact "target/lat5-nixos-closure-$REV" \
  ubuntu@64.34.93.213 --extra-files /run/lat5-bootstrap
```

The artifact records and contains the exact installer closure and successful
qualification output as well as SYSTEM, DISKO and KEXEC. The installer cannot
resolve a different executable from a local flake. The bootstrap set carries a fresh, locally generated SSH host keypair; record
its public fingerprint before installation and verify it after reboot.

Before install, transfer `metrics-remote-write.env` and `logs-write.env`
file-to-file from existing monitoring credential custody into
`/run/lat5-bootstrap/etc/finite/`, owned by root with mode 0600. They must be
present during activation. Add a fresh Ed25519 SSH host keypair under
`etc/ssh/`, with both files mode 0600. The staging root and its directories
must be root-owned 0700; the installer rejects every other file or symlink.
Do not stage `runner.env` during this OS-only step;
its absence keeps the Runner disabled. WireGuard/Core admission and the
remaining host credentials belong to step 3. Verify Latitude rescue/console
access and recheck the exact disk identities before starting the installer.

Rollback before agent admission is rescue access and reinstalling this empty
host from the same artifact. Ubuntu is disposable; no user state is being
migrated or erased. Record the resulting closure, healthy arrays, both ESPs,
SSH reachability, and canonical fleet status before calling steps 1–2 complete.

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

## Lat1 retirement

Austin confirmed on 2026-09-15 that lat1 will remain on its existing Latitude
retirement schedule because Dallas continues to have repeated heat problems.
This supersedes the reuse proposal. Lat1 is excluded from migration capacity;
lat5 setup proceeds independently.

[FIN-71](https://linear.app/finitecomputer/issue/FIN-71) tracks retirement.
The meeting recorded September 24; Alex's scheduling comment says end of month.
The exact provider date and eventual billing stop still need confirmation.
No cancellation was submitted to Latitude, and no lat1 disk changes occurred.

The September 15 Core snapshot showed zero active links and seven inactive
Runtime records. Lat1's old app-server disks remain a recovery source. Confirm
independent Recovery Sets and restore proof before provider deletion; inactive
compute does not authorize data loss or a separate manual purge.
