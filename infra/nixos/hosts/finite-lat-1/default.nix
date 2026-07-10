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
    ../../modules/finitechat-hosted-device.nix
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

  # Static public addressing via systemd-networkd, matched by the WAN NIC's
  # MAC (90:5a:08:2e:63:1b, derived from the capture's eno1 link-local
  # fe80::925a:8ff:fe2e:631b). Matching by MAC instead of interface name
  # makes this immune to the NIC enumerating under a different predictable
  # name on the NixOS kernel — the suspected cause of the first single-disk
  # boot being up-but-unreachable (2026-07-09). eno2 (…:631a) stays down.
  networking.useDHCP = false;
  networking.useNetworkd = true;
  systemd.network.enable = true;
  systemd.network.networks."10-wan" = {
    matchConfig.MACAddress = "90:5a:08:2e:63:1b";
    address = [
      "64.34.82.77/31"
      "2605:6440:5002:18e::2/64"
    ];
    routes = [
      { Gateway = "64.34.82.76"; }
      { Gateway = "2605:6440:5002:18e::1"; }
    ];
    linkConfig.RequiredForOnline = "routable";
  };
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
  # Single-disk layout (see disko.nix) — no software RAID, so no swraid /
  # mdadm-in-initrd. Plain ext4 on NVMe boots without any assembly step.
  boot.initrd.availableKernelModules = [
    "nvme"
    "xhci_pci"
    "ahci"
    "usbhid"
    "sd_mod"
    "ext4"
  ];

  # Container-shaped services (dashboard, finite-search) run under podman.
  virtualisation.podman.enable = true;
  virtualisation.oci-containers.backend = "podman";

  time.timeZone = "UTC";
  system.stateVersion = "25.11";
}
