# Captured 2026-10-14 from server sv_mJyo0vzLe0Dkw (64.34.89.107).
# All disks use 512-byte logical sectors. Root disks: 480103981056 bytes;
# data disks: 1920383410176 bytes. Fresh partition, RAID and filesystem IDs.
# Root A is the provider EFI boot disk (sda, serial 25285199C18C).
# The layout uses the qualified lat4 geometry for this disk capacity.
{
  captured = true;

  disks = {
    rootA = "/dev/disk/by-id/ata-Micron_5400_MTFDDAK480TGA_25285199C18C";
    rootB = "/dev/disk/by-id/ata-Micron_5400_MTFDDAK480TGA_25285199C2BD";
    dataA = "/dev/disk/by-id/nvme-eui.000000000000000100a0752349a063bc";
    dataB = "/dev/disk/by-id/nvme-eui.000000000000000100a0752349a06503";
  };

  partuuids = {
    espA = "07dfb0b3-50c0-4c49-aafc-e9578e2129fa";
    rootA = "89a4555c-fee4-4711-b2ab-1bbfcecd1f70";
    espB = "68b357a8-cd9a-4994-a7d8-a3a1c612b88c";
    rootB = "e2c1d316-d11e-47de-8c80-3b653067bfe1";
    dataA = "c2beef24-9d43-4a55-9779-8d8f4c4daba8";
    dataB = "dfdf20cc-1750-4e93-ae8d-f84e29e9045d";
  };

  mdUuids = {
    root = "52d350f1:4d62826f:5c7b8188:d233874e";
    data = "d35b8b5c:3700d77f:0cdca4c4:85a888e2";
  };

  filesystemUuids = {
    root = "3d464158-667d-4205-9527-8c199241c369";
    data = "54b8eac6-4ef1-4a7b-833f-adcbff9ca4e6";
    espA = "319B-7FBC";
    espB = "36EF-5B94";
  };

  # mkfs.vfat takes the same volume IDs without the display hyphen.
  vfatVolumeIds = {
    espA = "319b7fbc";
    espB = "36ef5b94";
  };
}
