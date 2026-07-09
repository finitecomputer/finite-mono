# Disk layout for nixos-anywhere, reproducing the captured Ubuntu shape:
#   md0 (software RAID1, ~439G) -> /       (ext4; was 18% used)
#   md1 (software RAID1, ~1.8T) -> /data   (ext4; backup landing zone)
#   ESP: capture shows /dev/nvme2n1p1 mounted at /boot/efi -> we put the ESP
#   on the first root-pair member and a spare (unmounted) ESP on the second.
#
# Device names VERIFIED against the live box 2026-07-09 (lsblk + /proc/mdstat):
#   md0 = nvme2n1p2 + nvme0n1p2 (447.1G disks; ESP on nvme2n1p1)
#   md1 = nvme3n1p1 + nvme1n1p1 (1.7T disks)
# Still re-run `lsblk` from the rescue env before nixos-anywhere — device
# enumeration can change across reboots/kernel versions.
{ ... }:
{
  disko.devices = {
    disk = {
      root0 = {
        type = "disk";
        device = "/dev/nvme2n1"; # verified: carries the live ESP (nvme2n1p1)
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
        device = "/dev/nvme0n1"; # verified: second md0 member (447.1G)
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
        device = "/dev/nvme3n1"; # verified: first md1 member (1.7T)
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
        device = "/dev/nvme1n1"; # verified: second md1 member (1.7T)
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
