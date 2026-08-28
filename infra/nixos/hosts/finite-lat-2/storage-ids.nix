{
  # NOT YET CAPTURED. Every value below is a placeholder until
  # `infra/nixos/scripts/capture-lat2-host-evidence` has been run against the
  # physical host (Gate A of infra/runbooks/lat2-replacement-cutover.md) and
  # its output has been reviewed into this file. The disk paths are invalid by
  # construction, `captured` stays false, and
  # scripts/build-lat2-nixos-closure-artifact refuses to package a closure
  # until `captured = true`. The geometry constants are carried over from
  # finite-lat-3 (same chassis class, captured 439G root / 1.8T data arrays)
  # and must be re-proven against the real disk sizes before they are trusted.
  captured = false;

  disks = {
    rootA = "/dev/disk/by-id/REPLACE-ME-finite-lat-2-root-a";
    rootB = "/dev/disk/by-id/REPLACE-ME-finite-lat-2-root-b";
    dataA = "/dev/disk/by-id/REPLACE-ME-finite-lat-2-data-a";
    dataB = "/dev/disk/by-id/REPLACE-ME-finite-lat-2-data-b";
  };

  partuuids = {
    espA = "00000000-0000-0000-0000-00000000000a";
    rootA = "00000000-0000-0000-0000-00000000000b";
    espB = "00000000-0000-0000-0000-00000000000c";
    rootB = "00000000-0000-0000-0000-00000000000d";
    dataA = "00000000-0000-0000-0000-00000000000e";
    dataB = "00000000-0000-0000-0000-00000000000f";
  };

  mdUuids = {
    root = "00000000:00000000:00000000:00000000";
    data = "00000000:00000000:00000000:00000001";
  };

  filesystemUuids = {
    root = "00000000-0000-0000-0000-000000000001";
    data = "00000000-0000-0000-0000-000000000002";
    espA = "0000-000A";
    espB = "0000-000B";
  };

  # mkfs.vfat takes the same volume IDs without the display hyphen.
  vfatVolumeIds = {
    espA = "0000000A";
    espB = "0000000B";
  };
}
