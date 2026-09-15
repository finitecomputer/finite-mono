# Execute the production storage layout on sparse virtual disks. Virtual media
# skip initial full-device synchronization only; physical SMART and resync are
# mandatory on the empty host. No production disk identity is used by QEMU.
{
  nixpkgs,
  disko,
  production,
}:
let
  pkgs = nixpkgs.legacyPackages.x86_64-linux;
  inherit (pkgs) lib;
  # Use the host's stable NixOS test driver as well as its packages. Disko's
  # flake follows the workspace's newer nixpkgs, whose QEMU API differs.
  diskoTestLib = import (disko + "/lib") {
    inherit lib;
    makeTest = import (nixpkgs + "/nixos/tests/make-test-python.nix");
    eval-config = import (nixpkgs + "/nixos/lib/eval-config.nix");
    qemu-common = import (nixpkgs + "/nixos/lib/qemu-common.nix");
  };
  ids = import ../hosts/finite-lat-5/storage-ids.nix;
  layout = (import ../hosts/finite-lat-5/disko.nix { }).disko.devices;
  testLayout = {
    disko.devices = layout // {
      mdadm = lib.mapAttrs (
        _: array:
        array
        // {
          extraArgs = array.extraArgs ++ [ "--assume-clean" ];
        }
      ) layout.mdadm;
    };
  };
  storageSystem = { lib, ... }: {
    imports = [
      disko.nixosModules.disko
      testLayout
      ../hosts/finite-lat-5/storage-health.nix
    ];
    networking.hostName = "lat5-storage-test";
    system.stateVersion = "26.05";
    boot.kernelPackages = production.config.boot.kernelPackages;
    boot.initrd.systemd.enable = true;
    boot.initrd.availableKernelModules = production.config.boot.initrd.availableKernelModules;
    boot.swraid = {
      enable = true;
      mdadmConf = production.config.boot.swraid.mdadmConf;
    };
    boot.loader = {
      efi.canTouchEfiVariables = false;
      grub = {
        enable = true;
        efiSupport = true;
        efiInstallAsRemovable = true;
        devices = lib.mkForce [ ];
        mirroredBoots = production.config.boot.loader.grub.mirroredBoots;
      };
    };
    fileSystems."/".neededForBoot = true;
    fileSystems."/data".neededForBoot = false;
    fileSystems."/boot-a".neededForBoot = false;
    fileSystems."/boot-b".neededForBoot = false;
    # The health script's physical swap/SMART checks are exercised on lat5.
    # Faulted-array checks below fail earlier, through the unchanged script.
    systemd.timers.finite-storage-health.enable = false;
    systemd.timers.finite-md-check.enable = false;
    environment.systemPackages = with pkgs; [
      mdadm
      util-linux
      e2fsprogs
    ];
  };
  unguarded = nixpkgs.lib.nixosSystem {
    system = "x86_64-linux";
    modules = [ storageSystem ];
  };
  guarded = unguarded.extendModules {
    specialArgs.unguardedInstallBootLoader = unguarded.config.system.build.installBootLoader;
    modules = [ ../hosts/finite-lat-5/esp-guard.nix ];
  };
in
(diskoTestLib.testLib.makeDiskoTest {
  name = "lat5-storage-boot";
  inherit pkgs;
  disko-config = testLayout;
  extendModules = guarded.extendModules;
  # MiB floor of the captured physical disks, lexical Disko disk order:
  # data-a, data-b, root-a, root-b. Every production partition still fits.
  extraInstallerConfig = {
    boot.kernelPackages = production.config.boot.kernelPackages;
    virtualisation.emptyDiskImages = lib.mkForce [
      7325650
      7325650
      457862
      457862
    ];
    virtualisation.memorySize = 4096;
  };
  postDisko = "disk_source_machine = machine";
  extraTestScript = ''
    machine.wait_for_unit("data.mount")
    machine.wait_for_unit("boot-a.mount")
    machine.wait_for_unit("boot-b.mount")
    machine.succeed("test $(blkid -s UUID -o value /dev/md/root) = ${ids.filesystemUuids.root}")
    machine.succeed("test $(blkid -s UUID -o value /dev/md/data) = ${ids.filesystemUuids.data}")
    machine.succeed("test -f /boot-a/EFI/BOOT/BOOTX64.EFI; test -f /boot-b/EFI/BOOT/BOOTX64.EFI")
    machine.succeed("findmnt -rn -o OPTIONS /data | grep -w prjquota")
    machine.succeed("test $(cat /sys/block/$(basename $(readlink -f /dev/md/root))/md/degraded) = 0")
    machine.succeed("test $(cat /sys/block/$(basename $(readlink -f /dev/md/data))/md/degraded) = 0")

    # Boot each ESP independently with the other's fallback loader absent.
    machine.succeed("mv /boot-b/EFI/BOOT/BOOTX64.EFI /boot-b/EFI/BOOT/BOOTX64.EFI.test-disabled; sync")
    machine.shutdown()
    machine = create_test_machine(oldmachine=disk_source_machine, name="booted_from_esp_a")
    machine.start()
    machine.wait_for_unit("local-fs.target")
    machine.wait_for_unit("boot-a.mount")
    machine.wait_for_unit("boot-b.mount")
    machine.succeed("mv /boot-b/EFI/BOOT/BOOTX64.EFI.test-disabled /boot-b/EFI/BOOT/BOOTX64.EFI")
    machine.succeed("mv /boot-a/EFI/BOOT/BOOTX64.EFI /boot-a/EFI/BOOT/BOOTX64.EFI.test-disabled; sync")
    machine.shutdown()
    machine = create_test_machine(oldmachine=disk_source_machine, name="booted_from_esp_b")
    machine.start()
    machine.wait_for_unit("local-fs.target")
    machine.wait_for_unit("boot-a.mount")
    machine.wait_for_unit("boot-b.mount")
    machine.succeed("mv /boot-a/EFI/BOOT/BOOTX64.EFI.test-disabled /boot-a/EFI/BOOT/BOOTX64.EFI")

    # The unchanged production guard must refuse before invoking GRUB.
    machine.succeed("umount /boot-b")
    machine.fail("${guarded.config.system.build.installBootLoader} /run/current-system > /tmp/guard.log 2>&1")
    machine.succeed("grep -F 'not an exact mountpoint' /tmp/guard.log")
    machine.succeed("mount --bind /boot-a /boot-b")
    machine.fail("${guarded.config.system.build.installBootLoader} /run/current-system > /tmp/guard.log 2>&1")
    machine.succeed("grep -F 'PARTUUID' /tmp/guard.log")
    machine.succeed("umount /boot-b; mount /boot-b")

    # Fault data first so the root check passes, then fault root. Do not
    # repair or resync multi-terabyte synthetic arrays merely to test refusal.
    machine.succeed("mdadm /dev/md/data --fail /dev/disk/by-partuuid/${ids.partuuids.dataB}")
    machine.fail("systemctl start finite-storage-health.service")
    machine.succeed("journalctl -u finite-storage-health.service --no-pager | grep -F '/dev/md/data is degraded'")
    machine.succeed("mdadm /dev/md/root --fail /dev/disk/by-partuuid/${ids.partuuids.rootB}")
    machine.succeed("systemctl reset-failed finite-storage-health.service")
    machine.fail("systemctl start finite-storage-health.service")
    machine.succeed("journalctl -u finite-storage-health.service --no-pager | grep -F '/dev/md/root is degraded'")
  '';
}).overrideTestDerivation
  (old: {
    meta = old.meta // {
      timeout = 1800;
    };
  })
