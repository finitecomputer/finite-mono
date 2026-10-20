{
  lib,
  pkgs,
  unguardedInstallBootLoader,
  ...
}:
let
  ids = import ./storage-ids.nix;

  guardedInstallBootLoader = pkgs.writeShellScript "install-grub-with-finite-esp-guard" ''
    set -euo pipefail

    check_esp() {
      mountpoint="$1"
      expected_partuuid="$2"

      actual_target="$(${pkgs.util-linux}/bin/findmnt -rn -o TARGET --target "$mountpoint" || true)"
      if [[ "$actual_target" != "$mountpoint" ]]; then
        echo "refusing bootloader write: $mountpoint is not an exact mountpoint" >&2
        exit 74
      fi

      fstype="$(${pkgs.util-linux}/bin/findmnt -rn -o FSTYPE --target "$mountpoint")"
      if [[ "$fstype" != "vfat" ]]; then
        echo "refusing bootloader write: $mountpoint is $fstype, expected vfat" >&2
        exit 74
      fi

      source="$(${pkgs.util-linux}/bin/findmnt -rn -o SOURCE --target "$mountpoint")"
      actual_partuuid="$(${pkgs.util-linux}/bin/blkid -s PARTUUID -o value "$source" | ${pkgs.coreutils}/bin/tr '[:upper:]' '[:lower:]')"
      if [[ "$actual_partuuid" != "$expected_partuuid" ]]; then
        echo "refusing bootloader write: $mountpoint PARTUUID $actual_partuuid, expected $expected_partuuid" >&2
        exit 74
      fi

      options="$(${pkgs.util-linux}/bin/findmnt -rn -o OPTIONS --target "$mountpoint")"
      if [[ ",$options," != *,rw,* ]]; then
        echo "refusing bootloader write: $mountpoint is not mounted read-write" >&2
        exit 74
      fi
    }

    check_esp /boot-a ${ids.partuuids.espA}
    check_esp /boot-b ${ids.partuuids.espB}
    exec ${unguardedInstallBootLoader} "$@"
  '';
in
{
  # The stock mirroredBoots installer writes before its extra hooks run. Wrap
  # the independently evaluated stock installer so every ordinary install,
  # switch, and boot action checks both exact mounted ESP identities first.
  system.build.installBootLoader = lib.mkForce guardedInstallBootLoader;
}
