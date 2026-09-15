# finite-lat-5 setup

Execution plan for [FIN-74](https://linear.app/finitecomputer/issue/FIN-74).
Status on 2026-09-15: **steps 1–3 complete**. The reviewed configuration
merged in [PR #883](https://github.com/finitecomputer/finite-mono/pull/883)
and the exact CI artifact is installed. Both physical EFI boot paths,
fully synchronized mirrors, storage health, and SSH identity are verified.
The bootstrap credential fix merged in
[PR #889](https://github.com/finitecomputer/finite-mono/pull/889).
Lat5 is connected to the hub and Core and reports drained capacity.
Controlled launch and cohort capacity qualification remain in step 4.
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
gh workflow run lat5-nixos-closure.yml --ref main -f rev="$REV"
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
Generate a unique WireGuard private key with `wg genkey` under `umask 077`
and stage it at `etc/finite/wireguard-private-key` (root:root, 0600). Keep
a secure copy with the bootstrap escrow; never copy another host's key.
The generated `systemd-networkd` service requires this credential before it
can start **any networking**, including public SSH. Hub peer registration
can wait until step 3, but this file cannot.

Do not stage `runner.env` during this OS-only step; its absence keeps the
Runner disabled. Hub/Core admission and the remaining host credentials
belong to step 3. Verify Latitude rescue/console
access and recheck the exact disk identities before starting the installer.

Rollback before agent admission is rescue access and reinstalling this empty
host from the same artifact. Ubuntu is disposable; no user state is being
migrated or erased. Record the resulting closure, healthy arrays, both ESPs,
SSH reachability, and canonical fleet status before calling steps 1–2 complete.

### Installation receipt — 2026-09-15

| Check | Verified result |
| --- | --- |
| Installed revision | `8993d75b0d3e6ba5e9d6497b32bf43192ada3254` |
| CI artifact | [Run 34996898901](https://github.com/finitecomputer/finite-mono/actions/runs/34996898901), artifact `10409875610` |
| Artifact ZIP SHA-256 | `e794b85cd92795299adde2e6563da77d7b43f92c17402d56f05cd89bfa1aae21` |
| Running system | `/nix/store/a0wvpp0irzn622fc3llx24vfx61ijavz-nixos-system-finite-lat-5-26.05.20260719.fd14620` |
| Kernel | `6.18.39`; KVM device present and cgroups v2 active |
| SSH host identity | Ed25519 `SHA256:NzeN06mlYWSufL6DRLSzKKyI9XWyZBjabsQZQT+bGDk`, recorded before install and verified after both test boots |
| Physical ESP A boot | `BootCurrent: 0005`, PARTUUID `c176bf90-d8fd-445a-8588-daaa356b4676` |
| Physical ESP B boot | `BootCurrent: 0006`, PARTUUID `38edadcb-9ca4-4e87-9153-8aba824551e9` |
| RAID and storage | Both mirrors idle, two active members each, zero degraded members and mismatches; full storage health service succeeds |
| Swap and data | 64 GiB active swapfile, zswap 10%, swappiness 20, `/data` project quotas and expected filesystem identity |
| Admission | `runner.env` absent, Runner service inactive, agent and staging directories empty |

First boot exposed a bootstrap error: `systemd-networkd` failed with
`243/CREDENTIALS` because its required WireGuard key had been deferred to
step 3. Latitude rescue mode provided SSH access without reinstalling the
host. The exact root filesystem was mounted, a newly generated host-specific
key was staged with root-only permissions and private off-host escrow, and
normal boot restored networking. The installer now rejects that incomplete
bootstrap set and malformed or mismatched SSH keys before installation.
All 24 focused tests, CI checks, and both reviews passed for that correction.
The installed NixOS closure did not change.

The physical bootloader guard refused both a missing ESP and the wrong ESP
PARTUUID in private mount namespaces, leaving host mounts unchanged. The
real storage health service refused readiness during initial synchronization
and succeeded after both mirrors became idle. Temporary array-specific
resync limits were restored to the system defaults. The final boot uses
ESP A; the normal boot order prefers A, then B, then the provider's PXE
entries and EFI shell. No kernel errors or failed systemd units were present
in the final boot verification.

Canonical `scripts/finite-status` receipts at 19:20 UTC report lat5 storage
green. Its overall Runner status remains red/unknown until credentials,
artifact pin, and Core admission are configured in step 3. Lat2's chat,
recovery and rollout sections remain green, with lat3 at 31/31 and lat4 at
27/27 ready runtimes. The pre-existing fleet version-skew status remains red.
Do not treat OS qualification as permission to create or migrate agents.

## 3. Install and connect while drained

Run `scripts/finite-status` before the rollout. Record the existing lat2
closure and off-host recovery boundary before changing its configuration.
Add lat5's peer-scoped WireGuard access and unique Core Runner credential
through the infrastructure rollout. Verify existing Runner connectivity after
the hub change.

The OS installation and physical qualification above are complete; step 3
does not repeat the wipe. Keep the installed lat5 closure and its one-runtime
ceiling. Add the escrowed peer key at `10.254.3.5/32` on lat2, with the
captured public endpoint, and permit its overlay address to reach private
Core (`14200`) and Identity (`18790`). Preserve the existing public UDP/51820
listener: WireGuard authenticates configured peer keys and permits endpoint
roaming; it is not a public source-IP allowlist. Existing lat3 and lat4 peer
identities and access stay intact.

For this rollout, the pre-change lat2 closure is
`/nix/store/q7syl41zvi2bb4k31pdjs5fsmi8cc41y-nixos-system-finite-lat-2-26.05.20260719.fd14620`
(revision `1568d5a924c8c31a5a0f9e8d47a5379c0900f4c3`). Build the reviewed
hub change with `lat2-nixos-closure.yml` and use
`scripts/deploy-lat2-closure-cache` from a Linux driver. Review dry activation
and compare service executable paths before switching; this operation must
not introduce a product binary upgrade. Preserve an off-host root-only copy
of lat2's original `core.env` before adding the credential and restarting
Core. Use a compare-before-replace check against the backed-up file so a
concurrent operator change fails closed.

Append exactly this metadata record to `FC_CORE_RUNNER_CREDENTIALS_JSON`,
with a newly generated unique token in the named environment variable:

```json
{"credentialId":"finite-lat-5-current","tokenEnv":"FC_CORE_RUNNER_CREDENTIAL_TOKEN_FINITE_LAT_5_CURRENT","runnerId":"finite-kata-runner-5","runnerClasses":["kata"],"sourceHostId":"finite-lat-5"}
```

Reject duplicate identities or token-variable names, missing/empty referenced
tokens, reused bearer values, and any change to existing metadata or other
Core settings. Validate the candidate JSON and token references before
atomic replacement. Keep lat5's `runner.env` absent until Core restarts and
passes health checks with the new keyring. Then install lat5's root-only
environment with the matching token and `FC_RUNNER_DRAIN=true`.

If credential activation fails, stop lat5's timer/service first, restore the
original `core.env`, restart Core, and verify Core health and existing Runner
connectivity. If the hub configuration itself failed, also restore the
recorded lat2 closure. The closure deploy helper cannot roll back a separately
edited `core.env`; treat these as two explicit rollback boundaries.

The production path is lat5 Runner → authenticated WireGuard peer → lat2
Core/Identity socket proxies → existing Core and Identity services. Core
reads the credential keyring at startup; the new credential is bound to
`finite-kata-runner-5`, class `kata`, source host `finite-lat-5`. Preserve all
existing keyring entries. Core may record drained Runner contact/capacity,
but must offer no creation work; the lat5 Runner must create no Runtime or
Identity state. Existing chat services and Runtime bindings are unchanged.
Verify lat3/lat4 connectivity and ready counts after the hub switch and Core
restart, then authenticate lat5 while drained.

Stage credentials by name and secure file transfer, with root-only access:
`runner.env`, `identity-operator.env` and `runtime-secrets.env`
under `/etc/finite/`. Retain the WireGuard private key and monitoring
credentials staged during OS installation. Derive the public key from that
existing WireGuard key and register it on lat2; do not generate a replacement.
Use a unique Runner credential. Pin the current promoted runtime artifact and set `FC_RUNNER_DRAIN=true`.

Complete when lat5 boots the exact artifact, both arrays are healthy, both
ESPs and storage identities pass validation, storage/boot refusal checks
pass, and Core authenticates its drained Runner with zero agents created.
Verify metrics, logs and `scripts/finite-status` after the rollout.

Before first admission, recovery is a reinstall of this empty host plus its
escrowed configuration and credentials. Revert a failed lat2 change to its
recorded closure and credential configuration. Once lat5 holds user state,
reinstallation requires a verified Recovery Set and restore proof; the empty
host recovery boundary no longer applies.

### Drained connection receipt — 2026-09-15

[PR #893](https://github.com/finitecomputer/finite-mono/pull/893) added lat5's
existing WireGuard public key and private Core/Identity access. CI artifact
[35015095493](https://github.com/finitecomputer/finite-mono/actions/runs/35015095493)
built merged revision `6ca4e21bc6e51ceea440594b6b1494467f47974f`.
Its digest-verified system is now running on lat2:
`/nix/store/yfm0r7m75cciy9bdgckmsg6c9bj59sb2-nixos-system-finite-lat-2-26.05.20260719.fd14620`.
Lat5 retains the exact OS closure from step 2.

Dry activation required a networkd restart, firewall/D-Bus reloads, and a
tmpfiles refresh of the revision metric. These four units were explicitly
allowed. All nine product-service unit files were byte-identical; Core and
chat retained their PIDs through the hub switch. The helper initially
reported failure because it unconditionally required containerd on lat2,
but neither the previous nor the new app-plane closure installs it. Live
checks confirmed the successful switch and healthy product services. The
helper now preserves containerd health only when a daemon was running before
the switch, while retaining its PID-change fence. A synthetic test reproduced
the false failure and covers absent, healthy, stopped, and approved/unapproved
restart cases. All 22 lat2 helper tests pass; CI now runs that previously
omitted suite. The obsolete test requiring Kata/KVM on the app-plane host
was removed.

The new Core credential was added atomically after comparing the live file
with its off-host escrow; all existing metadata and settings were preserved.
Core restarted successfully and returned HTTP 200 before lat5 received
`runner.env`. The original root-only file remains in
`/var/backups/finite-lat5-admission-20260915/core.env` on lat2 and in off-host
escrow. Identity and Runtime secret files were transferred directly and
verified as root-owned mode 0600.

| Check | Verified result |
| --- | --- |
| Overlay | Fresh handshakes for lat3, lat4, and lat5; lat5 reaches private Core and Identity health endpoints with HTTP 200 |
| Runner authentication | Repeated successful cycles, `capacity_unavailable` because `runner is draining` |
| Capacity | `draining=true`, `maxSandboxCount=1`, `activeSandboxCount=0`; no Agent Runtime created |
| Runtime pin | `finite-agent-runtime-2026-09-14.1`, matching Core's current promoted, non-retired artifact |
| Storage and telemetry | Storage green; metrics `up=1`; Runner logs received with `host="finite-lat-5"` and `role="runner"` |
| Existing fleet | Chat, recovery, and rollout sections green; lat3 31/31 and lat4 27/27 ready |

Canonical status intentionally marks lat5's drain as red for new admission.
Its local-only artifact comparison is unknown because Core's catalog is on
lat2; the pin was checked against lat2's canonical status separately. Existing
fleet version skew remains red. These are not an all-green fleet claim or
permission to open cohort capacity. Step 4 remains outstanding.

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
