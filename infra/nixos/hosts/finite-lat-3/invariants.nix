{ config, lib, ... }:
let
  ids = import ./storage-ids.nix;
  disks = config.disko.devices.disk;
  arrays = config.disko.devices.mdadm;
  rootA = disks."root-a".content.partitions;
  rootB = disks."root-b".content.partitions;
  dataA = disks."data-a".content.partitions;
  dataB = disks."data-b".content.partitions;
in
{
  assertions = [
    {
      assertion = disks."root-a".device == ids.disks.rootA;
      message = "finite-lat-3 root A disk identity drifted";
    }
    {
      assertion = disks."root-b".device == ids.disks.rootB;
      message = "finite-lat-3 root B disk identity drifted";
    }
    {
      assertion = disks."data-a".device == ids.disks.dataA;
      message = "finite-lat-3 data A disk identity drifted";
    }
    {
      assertion = disks."data-b".device == ids.disks.dataB;
      message = "finite-lat-3 data B disk identity drifted";
    }
    {
      assertion =
        rootA."esp-a".start == "2048s"
        && rootA."esp-a".end == "2099199s"
        && rootB."esp-b".start == "2048s"
        && rootB."esp-b".end == "2099199s";
      message = "finite-lat-3 ESP geometry drifted";
    }
    {
      assertion =
        rootA."root-a".start == "2099200s"
        && rootA."root-a".end == "935331839s"
        && rootB."root-b".start == "2099200s"
        && rootB."root-b".end == "935331839s";
      message = "finite-lat-3 root member geometry drifted";
    }
    {
      assertion =
        dataA."data-a".start == "2048s"
        && dataA."data-a".end == "3747612671s"
        && dataB."data-b".start == "2048s"
        && dataB."data-b".end == "3747612671s";
      message = "finite-lat-3 data member geometry drifted";
    }
    {
      assertion =
        arrays.root.extraArgs == [
          "--uuid=${ids.mdUuids.root}"
          "--data-offset=1024K"
          "--bitmap=internal"
          "--bitmap-chunk=64M"
          "--size=464519168K"
        ];
      message = "finite-lat-3 root MD creation contract drifted";
    }
    {
      assertion =
        arrays.data.extraArgs == [
          "--uuid=${ids.mdUuids.data}"
          "--data-offset=1024K"
          "--bitmap=internal"
          "--bitmap-chunk=64M"
          "--size=1871708160K"
        ];
      message = "finite-lat-3 data MD creation contract drifted";
    }
    {
      assertion = arrays.root.metadata == "1.2" && arrays.data.metadata == "1.2";
      message = "finite-lat-3 MD metadata version drifted";
    }
    {
      assertion =
        lib.length (lib.unique (lib.attrValues ids.partuuids)) == 6
        && lib.length (lib.unique (lib.attrValues ids.filesystemUuids)) == 4;
      message = "finite-lat-3 partition or filesystem identifiers are not unique";
    }
    {
      assertion =
        map (swap: {
          inherit (swap) device size;
        }) config.swapDevices == [
          {
            device = "/swapfile";
            size = 64 * 1024;
          }
        ];
      message = "finite-lat-3 swap contract drifted";
    }
    {
      assertion =
        config.boot.zswap.enable
        && config.boot.zswap.compressor == "zstd"
        && config.boot.zswap.maxPoolPercent == 10
        && config.boot.zswap.shrinkerEnabled;
      message = "finite-lat-3 zswap contract drifted";
    }
    {
      assertion =
        map (entry: entry.path) config.boot.loader.grub.mirroredBoots == [
          "/boot-a"
          "/boot-b"
        ];
      message = "finite-lat-3 mirrored ESP paths drifted";
    }
  ];
}
