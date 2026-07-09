# Disk layout for nixos-anywhere, reproducing the captured Ubuntu shape:
#   md0 (software RAID1, ~439G) -> /       (ext4; was 18% used)
#   md1 (software RAID1, ~1.8T) -> /data   (ext4; backup landing zone)
#   ESP: capture shows /dev/nvme2n1p1 mounted at /boot/efi -> we put the ESP
#   on the first root-pair member and a spare (unmounted) ESP on the second.
#
# ############################################################################
# ##                                                                        ##
# ##  TODO: verify device names against the box before nixos-anywhere.      ##
# ##                                                                        ##
# ##  The capture (host-capture/lat1/os-and-host.txt) proves md0/md1 exist  ##
# ##  and that the ESP lives on nvme2n1p1, but contains NO lsblk or         ##
# ##  /proc/mdstat, so WHICH nvme devices back each array is a GUESS:       ##
# ##    - nvme2n1 + nvme3n1 as the ~480G root pair (md0) — nvme2n1 is       ##
# ##      plausible because it carries the ESP; nvme3n1 is a placeholder.   ##
# ##    - nvme0n1 + nvme1n1 as the ~1.9T data pair (md1) — placeholders.    ##
# ##  Boot the Latitude rescue env and run `lsblk -o NAME,SIZE,MODEL` and   ##
# ##  `cat /proc/mdstat` FIRST, then fix the four `device =` lines below.   ##
# ##                                                                        ##
# ############################################################################
{ ... }:
{
  disko.devices = {
    disk = {
      root0 = {
        type = "disk";
        device = "/dev/nvme2n1"; # TODO: verify (carries the ESP in the capture)
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
        device = "/dev/nvme3n1"; # TODO: verify device name (placeholder)
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
        device = "/dev/nvme0n1"; # TODO: verify device name (placeholder)
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
        device = "/dev/nvme1n1"; # TODO: verify device name (placeholder)
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
