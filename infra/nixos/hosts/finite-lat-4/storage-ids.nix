{
  # CAPTURED 2026-08-28 from the physical host pre-wipe (read-only SSH
  # evidence; see docs/runs/lat4-provisioning-prep.md §1-2). The disk paths
  # are the live nvme-eui by-id identities; identifiers are freshly generated
  # and unique. The geometry constants are carried over from finite-lat-3
  # (same chassis class) and were re-proven against the real disk sizes:
  # root last-usable sector 937703054 >= 935331839, data 3750748814 >=
  # 3747612671. `captured` must stay true for any closure build; the lat4
  # build script must refuse to package while it is false, mirroring the
  # finite-lat-2 guard.
  captured = true;

  disks = {
    rootA = "/dev/disk/by-id/nvme-eui.000000000000000100a075244c59f3f7";
    rootB = "/dev/disk/by-id/nvme-eui.000000000000000100a075244c5a0002";
    dataA = "/dev/disk/by-id/nvme-eui.36344730595101610025384300000001";
    dataB = "/dev/disk/by-id/nvme-eui.36344730595101600025384300000001";
  };

  partuuids = {
    espA = "fcdb443d-2b23-42da-be98-3b70176d7f07";
    rootA = "e4fedfa6-38b3-46fe-822b-676d5c3313d3";
    espB = "b92437e1-c3d0-4a31-94d6-49fa065e8c39";
    rootB = "900c02d7-cf57-485d-98a5-b88b5d882e24";
    dataA = "f10fd758-e165-4716-a137-adb74c33a0a5";
    dataB = "630d9618-8137-4780-a67f-c201433332b5";
  };

  mdUuids = {
    root = "300c4b83:68b24c62:a16273b2:a8c472d3";
    data = "cb627fb8:74b6435c:b70f15bc:9f3d4461";
  };

  filesystemUuids = {
    root = "08e73fc6-3c62-4e64-8393-330e3d3c3327";
    data = "6a6bd904-fd72-4f28-a53c-3e08afb3eb42";
    espA = "C8D7-7405";
    espB = "DC89-98B7";
  };

  # mkfs.vfat takes the same volume IDs without the display hyphen.
  vfatVolumeIds = {
    espA = "C8D77405";
    espB = "DC8998B7";
  };
}
