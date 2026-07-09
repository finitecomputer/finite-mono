# Disk layout for nixos-anywhere, reproducing the captured Ubuntu shape:
#   md0 (software RAID1, ~439G) -> /       (ext4; was 18% used)
#   md1 (software RAID1, ~1.8T) -> /data   (ext4; backup landing zone)
#   ESP: capture shows /dev/nvme2n1p1 mounted at /boot/efi -> we put the ESP
#   on the first root-pair member and a spare (unmounted) ESP on the second.
#
# Devices addressed by /dev/disk/by-id/ (serial-stable) so the installer
# kernel's enumeration order cannot mismatch them (verified 2026-07-09):
#   md0 (root, 447G Micron): ...E53F (ESP carrier) + ...E531
#   md1 (data, 1.7T Samsung): ...510146 + ...510141
{ ... }:
{
  disko.devices = {
    disk = {
      root0 = {
        type = "disk";
        device = "/dev/disk/by-id/nvme-Micron_7450_MTFDKBA480TFR_24474C59E53F"; # was nvme2n1 (ESP carrier)
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
            raid-root = {
              size = "100%";
              content = {
                type = "mdraid";
                name = "md0";
              };
            };
          };
        };
      };
      root1 = {
        type = "disk";
        device = "/dev/disk/by-id/nvme-Micron_7450_MTFDKBA480TFR_24474C59E531"; # was nvme0n1 (447G Micron)
        content = {
          type = "gpt";
          partitions = {
            # Spare ESP, kept in the partition table but not mounted; sync it
            # manually if root0 dies (systemd-boot only writes one ESP).
            esp-spare = {
              size = "512M";
              type = "EF00";
            };
            raid-root = {
              size = "100%";
              content = {
                type = "mdraid";
                name = "md0";
              };
            };
          };
        };
      };
      data0 = {
        type = "disk";
        device = "/dev/disk/by-id/nvme-SAMSUNG_MZQL21T9HCJR-00A07_S64GNC0Y510146"; # was nvme3n1 (1.7T Samsung)
        content = {
          type = "gpt";
          partitions = {
            raid-data = {
              size = "100%";
              content = {
                type = "mdraid";
                name = "md1";
              };
            };
          };
        };
      };
      data1 = {
        type = "disk";
        device = "/dev/disk/by-id/nvme-SAMSUNG_MZQL21T9HCJR-00A07_S64GNC0Y510141"; # was nvme1n1 (1.7T Samsung)
        content = {
          type = "gpt";
          partitions = {
            raid-data = {
              size = "100%";
              content = {
                type = "mdraid";
                name = "md1";
              };
            };
          };
        };
      };
    };
    mdadm = {
      md0 = {
        type = "mdadm";
        level = 1;
        content = {
          type = "filesystem";
          format = "ext4";
          mountpoint = "/";
        };
      };
      md1 = {
        type = "mdadm";
        level = 1;
        content = {
          type = "filesystem";
          format = "ext4";
          mountpoint = "/data";
        };
      };
    };
  };
}
