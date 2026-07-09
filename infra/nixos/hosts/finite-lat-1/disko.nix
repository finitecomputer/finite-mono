# Single-disk layout (no mdadm). Decision 2026-07-09: disko's mdadm RAID1
# superblocks were unassemblable on the pinned nixpkgs (25.11) kernel — the
# recorded array size exceeded what fits after the 129 MiB data offset, so the
# kernel rejected every member on boot with "md_import_device returned -22"
# and stage-1 could not mount root. Rather than fight the mdadm-version bug
# mid-cutover, root and /data each live on a single NVMe. The other two NVMes
# (Micron ...E531, Samsung ...510141) are left untouched, free to add as
# mirrors (ZFS or a fixed mdadm) in a calm follow-up. All data is backed up
# and this config is in git, so a single root disk is an acceptable interim
# redundancy posture.
#
# Device paths are serial-stable /dev/disk/by-id (verified on the box
# 2026-07-09): root on the Micron that already carried the ESP; /data on a
# Samsung 1.7T. `nofail` on /data so a data-disk problem never blocks boot.
{ ... }:
{
  disko.devices = {
    disk = {
      root = {
        type = "disk";
        device = "/dev/disk/by-id/nvme-Micron_7450_MTFDKBA480TFR_24474C59E53F";
        content = {
          type = "gpt";
          partitions = {
            esp = {
              size = "512M";
              type = "EF00";
              content = {
                type = "filesystem";
                format = "vfat";
                mountpoint = "/boot";
                mountOptions = [ "umask=0077" ];
              };
            };
            root = {
              size = "100%";
              content = {
                type = "filesystem";
                format = "ext4";
                mountpoint = "/";
              };
            };
          };
        };
      };
      data = {
        type = "disk";
        device = "/dev/disk/by-id/nvme-SAMSUNG_MZQL21T9HCJR-00A07_S64GNC0Y510146";
        content = {
          type = "gpt";
          partitions = {
            data = {
              size = "100%";
              content = {
                type = "filesystem";
                format = "ext4";
                mountpoint = "/data";
                mountOptions = [ "nofail" ];
              };
            };
          };
        };
      };
    };
  };
}
