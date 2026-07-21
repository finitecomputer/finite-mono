{
  disks = {
    rootA = "/dev/disk/by-id/nvme-eui.000000000000000100a075255199d70f";
    rootB = "/dev/disk/by-id/nvme-eui.000000000000000100a075255199d6cc";
    dataA = "/dev/disk/by-id/nvme-eui.000000000000000100a075254fa09807";
    dataB = "/dev/disk/by-id/nvme-eui.000000000000000100a075254fa098c7";
  };

  partuuids = {
    espA = "50663fe2-131c-4a6b-b654-15f063e0ceb6";
    rootA = "1cf4d556-d757-4c95-9299-1f9ec51dd495";
    espB = "f5d070e0-80fb-4808-9ebf-eb05c7aee4ad";
    rootB = "8d674c57-bb09-4f77-baad-fec3e60df453";
    dataA = "926c6b96-276e-41d8-a7b4-a21951cca840";
    dataB = "74c7ae2a-a4c0-4575-be55-0714c8f5ada3";
  };

  mdUuids = {
    root = "6ad37071:48614192:806ab364:80f0b8f0";
    data = "9c13aa78:0e674937:bd2e179c:59216802";
  };

  filesystemUuids = {
    root = "0e236c38-bbef-495a-b604-964a53ae6d22";
    data = "6661e69b-efbe-4c71-9e65-c0ed0f653e5b";
    espA = "E0F0-10B5";
    espB = "0455-7775";
  };

  # mkfs.vfat takes the same volume IDs without the display hyphen.
  vfatVolumeIds = {
    espA = "E0F010B5";
    espB = "04557775";
  };
}
