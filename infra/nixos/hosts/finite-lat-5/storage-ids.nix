# Captured 2026-09-15 from server sv_6B9VaL7GE57vr (64.34.93.213).
# All disks use 512-byte logical sectors. Root disks: 480103981056 bytes;
# data disks: 7681501126656 bytes. Fresh partition, RAID and filesystem IDs.
# Root geometry follows lat4. Data end and MD size scale its data allocation
# by four and fit these larger drives. See the lat5 setup runbook.
{
  captured = true;

  disks = {
    rootA = "/dev/disk/by-id/ata-Micron_5400_MTFDDAK480TGA_252450DBF87F";
    rootB = "/dev/disk/by-id/ata-Micron_5400_MTFDDAK480TGA_252450DBF89D";
    dataA = "/dev/disk/by-id/nvme-eui.000000000000000100a0752555a59e79";
    dataB = "/dev/disk/by-id/nvme-eui.000000000000000100a0752555a5b623";
  };

  partuuids = {
    espA = "c176bf90-d8fd-445a-8588-daaa356b4676";
    rootA = "441a7b9b-ffe0-4928-9de3-6ac0881c68a6";
    espB = "38edadcb-9ca4-4e87-9153-8aba824551e9";
    rootB = "888ec622-dea5-4ba0-a224-b987995dcbeb";
    dataA = "c0ac6092-dd5f-437f-9acc-c667342406ac";
    dataB = "e838a90d-5aca-40b8-84a8-e6ac82577263";
  };

  mdUuids = {
    root = "15ecd4ee:7bf6ca48:4530626d:ff044c5b";
    data = "bc4cb7c8:3bda7546:a02267e8:9de6bef4";
  };

  filesystemUuids = {
    root = "f4c2f22a-ca26-4194-bfa9-cbb50d9a8cda";
    data = "845e75d6-2fb8-4f7f-a2e7-03897daf012b";
    espA = "8F77-7CA5";
    espB = "9F49-9EC0";
  };

  # mkfs.vfat takes the same volume IDs without the display hyphen.
  vfatVolumeIds = {
    espA = "8f777ca5";
    espB = "9f499ec0";
  };
}
