# Current single-disk layout (no mdadm). During the 2026-07-09 cutover, the
# candidate RAID1 members were unassemblable: their recorded component size did
# not fit after the observed data offset, and the kernel rejected them with
# "md_import_device returned -22" before stage 1 could mount root. Root and
# /data therefore remain on one NVMe each. The matching disks retain stale MD
# metadata from that failed attempt; they are neither untouched nor authorized
# for an in-place mirror conversion. Complete Agent/Sites/Brain recovery and an
# empty-target full-host restore are also not yet proved, so do not infer that
# every data class is backed up. The finite-lat capacity/redundancy plan governs
# the replacement design and any future destructive work.
#
# Device paths are serial-stable /dev/disk/by-id (verified on the box
# 2026-07-09): root on the Micron that already carried the ESP; /data on a
# Samsung 1.92-TB (~1.75-TiB raw). The current `nofail` permits host boot
# without /data but does not itself stop a dependent service from writing into
# the root mountpoint;
# every next design must prove that failure mode closed before user state.
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
