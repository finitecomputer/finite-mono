# Install a NixOS host from a CI artifact

This is a destructive operation, distinct from normal closure deployment.
It requires explicit authorization naming the host and disks. Never use a
retired host's transcript as authority to wipe a current machine.

## Preconditions

- Identify the exact machine, NICs, serial-stable disk paths and current data
  owners using the host-specific `infra/nixos/scripts/capture-lat*-host-evidence`
  tool (for example `capture-lat4-host-evidence`) and fresh provider-console evidence.
- Preserve its complete Recovery Set and independently prove restoration onto
  an empty target before erasing data. Record recovery credential custody and
  the previous system configuration outside git.
- Compare hardware against `infra/nixos/hosts/finite-lat-N/`, including
  `storage-ids.nix`. Validate geometry, RAID and EFI assumptions with the host's
  existing contract checks. Stop on mismatch; never adapt disks by sort order.
- Build the reviewed main revision using the matching `LatN NixOS Closure`
  workflow. Download its `latN-nixos-closure-REV` artifact and retain the manifest.

## Install and verify

The host-specific `scripts/install-latN-from-artifact` driver takes the artifact
directory and explicit rescue-mode SSH target. Run its `--validate-only` mode
first. The driver verifies the host, full source revision, manifest and cached
SYSTEM/DISKO/KEXEC store paths; it substitutes those paths without building.
Use a driver capable of realizing the artifact's Linux store paths. Do not
replace the pinned installer with an arbitrary local version.

After authorization, run the same driver without `--validate-only`. Installation
wipes the selected disks; a NixOS generation rollback cannot undo it. Keep the
provider console open through reboot.

Verify both EFI boot paths, synchronized mirrors, storage health and SSH host
identity. Provision secrets using [SOPS operations](../secret/OPERATIONS.md),
verify the WireGuard peer and Core authentication, and keep a Runner drained
until capacity, empty-target recovery and a disposable Agent canary pass.
Capture `scripts/finite-status` before and after bringing the host into service.
App-plane restoration uses [the coordinated recovery procedure](hosted-web-chat-recovery.md).

## Existing Runtime bindings

Installation does not authorize moving Core bindings. For an existing Runtime,
use [cold relocation](runtime-cold-relocation.md) and its exact
`runtime_relocation.v1` transaction (`runtime-cold-relocate-exact`).
`--source-compute-absent` requires the documented retained-state evidence;
never bulk-edit `source_host_id`. Retain the private `migrated-runtimes.manifest`
when moving an explicitly approved set, and verify every Runtime independently.

## Recovery boundary

On failure, preserve console output and disk evidence. Stop rather than reusing
another host's geometry or copying live databases. Recover from the independently
verified Recovery Set on an isolated target; moving traffic or enabling Runner
admission requires its own explicit authorization.
