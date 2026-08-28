{ ... }:
let
  ids = import ./storage-ids.nix;

  esp =
    {
      label,
      partuuid,
      mountpoint,
      volumeId,
    }:
    {
      inherit label;
      type = "EF00";
      uuid = partuuid;
      start = "2048s";
      end = "2099199s";
      alignment = 2048;
      content = {
        type = "filesystem";
        format = "vfat";
        inherit mountpoint;
        extraArgs = [
          "-F"
          "32"
          "-i"
          volumeId
          "-n"
          label
        ];
        mountOptions = [
          "umask=0077"
          "nofail"
          "x-systemd.device-timeout=10s"
        ];
      };
    };

  rootMember =
    {
      label,
      partuuid,
    }:
    {
      inherit label;
      type = "FD00";
      uuid = partuuid;
      start = "2099200s";
      end = "935331839s";
      alignment = 2048;
      content = {
        type = "mdraid";
        name = "root";
      };
    };

  dataMember =
    {
      label,
      partuuid,
    }:
    {
      inherit label;
      type = "FD00";
      uuid = partuuid;
      start = "2048s";
      end = "3747612671s";
      alignment = 2048;
      content = {
        type = "mdraid";
        name = "data";
      };
    };
in
{
  disko.devices = {
    disk = {
      root-a = {
        type = "disk";
        device = ids.disks.rootA;
        content = {
          type = "gpt";
          partitions = {
            esp-a = esp {
              label = "FIN-ESP-A";
              partuuid = ids.partuuids.espA;
              mountpoint = "/boot-a";
              volumeId = ids.vfatVolumeIds.espA;
            };
            root-a = rootMember {
              label = "finite-root-a";
              partuuid = ids.partuuids.rootA;
            };
          };
        };
      };

      root-b = {
        type = "disk";
        device = ids.disks.rootB;
        content = {
          type = "gpt";
          partitions = {
            esp-b = esp {
              label = "FIN-ESP-B";
              partuuid = ids.partuuids.espB;
              mountpoint = "/boot-b";
              volumeId = ids.vfatVolumeIds.espB;
            };
            root-b = rootMember {
              label = "finite-root-b";
              partuuid = ids.partuuids.rootB;
            };
          };
        };
      };

      data-a = {
        type = "disk";
        device = ids.disks.dataA;
        content = {
          type = "gpt";
          partitions.data-a = dataMember {
            label = "finite-data-a";
            partuuid = ids.partuuids.dataA;
          };
        };
      };

      data-b = {
        type = "disk";
        device = ids.disks.dataB;
        content = {
          type = "gpt";
          partitions.data-b = dataMember {
            label = "finite-data-b";
            partuuid = ids.partuuids.dataB;
          };
        };
      };
    };

    mdadm = {
      root = {
        type = "mdadm";
        level = 1;
        metadata = "1.2";
        extraArgs = [
          "--uuid=${ids.mdUuids.root}"
          "--data-offset=1024K"
          "--bitmap=internal"
          "--bitmap-chunk=64M"
          "--size=464519168K"
        ];
        content = {
          type = "filesystem";
          format = "ext4";
          mountpoint = "/";
          extraArgs = [
            "-b"
            "4096"
            "-L"
            "finite-root"
            "-U"
            ids.filesystemUuids.root
          ];
          mountOptions = [ "defaults" ];
        };
      };

      data = {
        type = "mdadm";
        level = 1;
        metadata = "1.2";
        extraArgs = [
          "--uuid=${ids.mdUuids.data}"
          "--data-offset=1024K"
          "--bitmap=internal"
          "--bitmap-chunk=64M"
          "--size=1871708160K"
        ];
        content = {
          type = "filesystem";
          format = "ext4";
          mountpoint = "/data";
          extraArgs = [
            "-b"
            "4096"
            "-L"
            "finite-data"
            "-U"
            ids.filesystemUuids.data
            "-O"
            "quota,project"
            "-E"
            "quotatype=prjquota"
          ];
          mountOptions = [
            "defaults"
            "prjquota"
            "nofail"
            "x-systemd.device-timeout=10s"
          ];
        };
      };
    };
  };
}
