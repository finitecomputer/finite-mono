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
    ../../modules/finite-brain.nix
    ../../modules/finitesitesd.nix
    ../../modules/dashboard.nix
    ../../modules/finite-search.nix
    ../../modules/caddy.nix
    ../../modules/oauth2-proxy.nix
    ../../modules/postgres.nix
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
  # TODO: verify the IPv4 gateway before nixos-anywhere. 64.34.82.76 is the
  # /31 peer (the usual Latitude.sh point-to-point shape) but the capture has
  # no `ip route` output.
  networking.defaultGateway = {
    address = "64.34.82.76";
    interface = "eno1";
  };
  # TODO: IPv6 default gateway is not in the capture (likely a link-local
  # fe80:: next hop on eno1); set networking.defaultGateway6 once known.
  # TODO: confirm Latitude.sh resolvers; public resolvers as a safe default.
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
    # TODO: add operator public keys before nixos-anywhere (Paul's key + the
    # CI deploy key that runs nixos-rebuild --target-host).
  ];

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;
  boot.initrd.availableKernelModules = [
    "nvme"
    "xhci_pci"
    "ahci"
    "usbhid"
  ];
  boot.swraid.enable = true;
  boot.swraid.mdadmConf = ''
    MAILADDR root
  '';

  # Container-shaped services (dashboard, finite-search) run under podman.
  virtualisation.podman.enable = true;
  virtualisation.oci-containers.backend = "podman";

  time.timeZone = "UTC";
  system.stateVersion = "25.11";
}
