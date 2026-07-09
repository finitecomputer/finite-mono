# finite-lat-1 — 64.34.82.77, Latitude.sh — THE single app server.
# Reinstalled as NixOS via nixos-anywhere per finite-fable/single-server-plan.md.
# Public exposure is exactly 22/80/443; every service binds loopback behind Caddy.
{ ... }:
{
  imports = [
    ./disko.nix
    ../../modules/finite-saas-core.nix
    ../../modules/finite-saas-runner.nix
    ../../modules/finitechat-server.nix
    ../../modules/finitesitesd.nix
    ../../modules/dashboard.nix
    ../../modules/finite-search.nix
    ../../modules/caddy.nix
    ../../modules/postgres.nix
    # DEFERRED to the brain/auth-integration follow-up (Paul, 2026-07-09):
    # brain is fully independent (own box/data/DNS) and is tangled with
    # oauth2-proxy, which is slated for replacement by WorkOS/Core-integrated
    # auth. Brain stays on smoke, zero downtime, through this cutover.
    # ../../modules/finite-brain.nix
    # ../../modules/oauth2-proxy.nix
    ../../modules/backups.nix
    ../../modules/monitoring.nix
  ];

  networking.hostName = "finite-lat-1";

  # Static public addressing, from the 2026-07-08 capture (eno1 carries
  # 64.34.82.77/31 + 2605:6440:5002:18e::2/64; eno2 is up but unaddressed).
  networking.useDHCP = false;
  networking.interfaces.eno1 = {
    ipv4.addresses = [
      {
        address = "64.34.82.77";
        prefixLength = 31;
      }
    ];
    ipv6.addresses = [
      {
        address = "2605:6440:5002:18e::2";
        prefixLength = 64;
      }
    ];
  };
  # Gateways VERIFIED against the live box 2026-07-09 (`ip route` / `ip -6 route`).
  networking.defaultGateway = {
    address = "64.34.82.76";
    interface = "eno1";
  };
  networking.defaultGateway6 = {
    address = "2605:6440:5002:18e::1";
    interface = "eno1";
  };
  # Matches the live resolver set (resolvectl on the Ubuntu install).
  networking.nameservers = [
    "1.1.1.1"
    "8.8.8.8"
  ];

  # ONLY the edge is public. Everything else is loopback (see port map in
  # ../../README.md).
  networking.firewall = {
    enable = true;
    allowedTCPPorts = [
      22
      80
      443
    ];
    allowedUDPPorts = [ ];
  };

  services.openssh = {
    enable = true;
    settings = {
      PermitRootLogin = "prohibit-password";
      PasswordAuthentication = false;
      KbdInteractiveAuthentication = false;
    };
  };
  users.users.root.openssh.authorizedKeys.keys = [
    # Paul (same key that already administers the fleet).
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHqbHvWlrXRkTc0403ubkqNE/Ge4YbPvKwWuRBoLPVAW paul@paul.lol"
    # TODO: add a CI deploy key here if/when deploys move off Paul's machine.
  ];

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;
  boot.initrd.availableKernelModules = [
    "nvme"
    "xhci_pci"
    "ahci"
    "usbhid"
    "sd_mod"
    "ext4"
  ];
  # Force-load the RAID1 personality in the initrd. raid1.ko ships in the
  # initrd via swraid.enable, but on the 6.12 kernel the on-demand load
  # wasn't firing during incremental assembly, so the kernel rejected the
  # members with EINVAL ("failed to add member: Invalid argument",
  # "assembled from 0 drives") and /dev/md/md0 never appeared → stage-1
  # could not mount root (observed twice, 2026-07-09). Loading it explicitly
  # guarantees the personality is ready before mdadm assembles the arrays.
  boot.initrd.kernelModules = [ "raid1" ];
  boot.swraid.enable = true;
  # The initrd runs `mdadm --assemble --scan` against THIS conf. Without
  # explicit ARRAY lines it assembled nothing, so /dev/md/md0 never appeared
  # and stage-1 failed to mount root (observed 2026-07-09 first boot). Match
  # by the metadata array name, which disko reproduces on every run
  # (homehost is unset in the installer, so mdadm stores "any:md0"/"any:md1")
  # — stable across re-partitioning, unlike the per-format UUID.
  boot.swraid.mdadmConf = ''
    DEVICE partitions
    HOMEHOST <ignore>
    AUTO +all
    ARRAY /dev/md0 metadata=1.2 name=any:md0
    ARRAY /dev/md1 metadata=1.2 name=any:md1
    MAILADDR root
  '';

  # Container-shaped services (dashboard, finite-search) run under podman.
  virtualisation.podman.enable = true;
  virtualisation.oci-containers.backend = "podman";

  time.timeZone = "UTC";
  system.stateVersion = "25.11";
}
