{
  # Captured 2026-08-28 from the physical host in Latitude rescue mode
  # (infra/nixos/scripts/capture-lat2-host-evidence, Gate A of
  # infra/runbooks/lat2-replacement-cutover.md), reviewed by Paul.
  # Geometry carried over from the lat3 qualification and re-proven against
  # these disks: root member end 935331839s <= 937703088 sectors on the
  # 480G Micron pair; data member end 3747612671s <= 3750748848 sectors on
  # the 1.92T Samsung pair. Root pair = nvme0n1/nvme1n1 (Micron), data pair
  # = nvme2n1/nvme3n1 (Samsung), matching the Ubuntu arrays' pairing.
  captured = true;

  disks = {
    rootA = "/dev/disk/by-id/nvme-eui.000000000000000100a075244c213b3a";
    rootB = "/dev/disk/by-id/nvme-eui.000000000000000100a075244c213bdd";
    dataA = "/dev/disk/by-id/nvme-eui.3634473057c127620025385300000001";
    dataB = "/dev/disk/by-id/nvme-eui.3634473057c127510025385300000001";
  };

  partuuids = {
    espA = "915d85bd-5993-4121-83e4-4dc85659098d";
    rootA = "94df5349-e7f7-44dd-a717-f885a6d68989";
    espB = "c5473df8-bfda-4aeb-a390-d6f9bc0e95ac";
    rootB = "f1e862b1-9085-455d-b363-4f6786b5969e";
    dataA = "d9854627-8522-43d8-9c6d-da644a882812";
    dataB = "07698e7b-d6ee-4053-abf4-cec38e926fe3";
  };

  mdUuids = {
    root = "b3e145ef:c78b134c:6f6cc268:5f9371e0";
    data = "8ade1ea3:8e2ddeef:58704012:4b5e3adc";
  };

  filesystemUuids = {
    root = "820e2fa8-8285-4735-9259-1a5c53479799";
    data = "5a16eefe-79fe-4e42-a647-b509e6df0e58";
    espA = "BBCF-F4C0";
    espB = "659F-3667";
  };

  # mkfs.vfat takes the same volume IDs without the display hyphen.
  vfatVolumeIds = {
    espA = "BBCFF4C0";
    espB = "659F3667";
  };
}
