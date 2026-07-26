# finite-lat-1 — 64.34.82.77, Latitude.sh — THE single app server.
# Reinstalled as NixOS via nixos-anywhere per finite-fable/single-server-plan.md.
# Public exposure is exactly 22/80/443; every service binds loopback behind Caddy.
{ config, pkgs, ... }:
let
  privateIdentityProxyPort = 18790;
in
{
  imports = [
    ./disko.nix
    ../../modules/finite-saas-core.nix
    ../../modules/finite-saas-runner.nix
    ../../modules/finite-saas-phala-runner.nix
    ../../modules/finite-identity.nix
    ../../modules/finitechat-server.nix
    ../../modules/finitechat-hosted-device.nix
    ../../modules/finitesitesd.nix
    ../../modules/finite-brain.nix
    ../../modules/dashboard.nix
    ../../modules/finite-search.nix
    ../../modules/caddy.nix
    ../../modules/postgres.nix
    ../../modules/backups.nix
    ../../modules/monitoring.nix
  ];

  networking.hostName = "finite-lat-1";

  # Reuse the existing finitecomputer rsync.net destination account, with a
  # repository dedicated to lat1 so encryption and retention are not coupled
  # to clawland's finitecomputer archives.
  finite.recoveryBackup.borgRepository = "fm2890@fm2890.rsync.net:finitecomputer/finite-lat-1";

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

  # Private Runner/control path to finite-lat-3. Core remains loopback-only;
  # the socket proxy below exposes only its authenticated API on this overlay.
  networking.wireguard.interfaces."wg-finite" = {
    ips = [ "10.254.3.1/30" ];
    listenPort = 51820;
    privateKeyFile = "/etc/finite/wireguard-private-key";
    peers = [
      {
        publicKey = "zykV8vPF1iaoN6Ycc2QQxEF+T8NHBYq9Qgk81U/V+mk=";
        allowedIPs = [ "10.254.3.2/32" ];
        endpoint = "207.188.7.157:51820";
        persistentKeepalive = 25;
      }
    ];
  };

  # ONLY the edge is public. Everything else is loopback (see port map in
  # ../../README.md).
  networking.firewall = {
    enable = true;
    allowedTCPPorts = [
      22
      80
      443
    ];
    # Preserve the bounded live rules: only finite-lat-3's public address can
    # establish WireGuard, and only that authenticated overlay address can
    # reach the private Core and Identity proxies. extraCommands run before
    # the final reject rule in the iptables firewall.
    extraCommands = ''
      iptables -w -A nixos-fw \
        -s 207.188.7.157/32 -d 64.34.82.77/32 \
        -p udp --dport 51820 \
        -m comment --comment finite-lat3-wg \
        -j nixos-fw-accept
      iptables -w -A nixos-fw \
        -s 10.254.3.2/32 -d 10.254.3.1/32 -i wg-finite \
        -p tcp --dport 14200 \
        -m comment --comment finite-lat3-core \
        -j nixos-fw-accept
      iptables -w -A nixos-fw \
        -s 10.254.3.2/32 -d 10.254.3.1/32 -i wg-finite \
        -p tcp --dport ${toString privateIdentityProxyPort} \
        -m comment --comment finite-lat3-identity \
        -j nixos-fw-accept
    '';
  };

  systemd.tmpfiles.rules = [
    "z /etc/finite/wireguard-private-key 0600 root root - -"
  ];

  systemd.sockets.finite-core-private-proxy = {
    description = "Private finite-lat Runner access to Core";
    wantedBy = [ "sockets.target" ];
    listenStreams = [ "10.254.3.1:14200" ];
    socketConfig = {
      Accept = false;
      FreeBind = true;
    };
  };

  systemd.services.finite-core-private-proxy = {
    description = "Proxy the private Runner socket to loopback Core";
    requires = [ "finite-saas-core.service" ];
    after = [ "finite-saas-core.service" ];
    serviceConfig = {
      ExecStart = "${pkgs.systemd}/lib/systemd/systemd-socket-proxyd 127.0.0.1:4200";
      DynamicUser = true;
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "strict";
      ProtectHome = true;
    };
  };

  # Identity's operator API is intentionally absent from its public Caddy
  # route. Give only the authenticated lat3 WireGuard peer a private path to
  # the same loopback Authority used by trusted same-host products.
  systemd.sockets.finite-identity-private-proxy = {
    description = "Private finite-lat Runner access to Identity Authority";
    wantedBy = [ "sockets.target" ];
    listenStreams = [ "10.254.3.1:${toString privateIdentityProxyPort}" ];
    socketConfig = {
      Accept = false;
      FreeBind = true;
    };
  };

  systemd.services.finite-identity-private-proxy = {
    description = "Proxy the private Runner socket to loopback Identity Authority";
    requires = [ "finite-identity.service" ];
    after = [ "finite-identity.service" ];
    serviceConfig = {
      ExecStart = "${pkgs.systemd}/lib/systemd/systemd-socket-proxyd 127.0.0.1:8790";
      DynamicUser = true;
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "strict";
      ProtectHome = true;
    };
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

  # Pre-RAID reliability step: add bounded swap and use the same zswap policy
  # qualified on finite-lat-3. This creates a file on the existing root
  # filesystem; it does not repartition either disk or enable mdraid.
  swapDevices = [
    {
      device = "/swapfile";
      size = 64 * 1024;
    }
  ];
  boot.zswap = {
    enable = true;
    compressor = "zstd";
    zpool = "zsmalloc";
    maxPoolPercent = 10;
    acceptThresholdPercent = 90;
    shrinkerEnabled = true;
  };
  boot.kernel.sysctl."vm.swappiness" = 20;
  zramSwap.enable = false;

  assertions = [
    {
      assertion = config.system.nixos.release == "26.05";
      message = "finite-lat-1 pre-RAID platform candidate must stay on the qualified NixOS 26.05 pin";
    }
    {
      assertion = config.system.stateVersion == "25.11";
      message = "finite-lat-1 stateVersion must not change during a platform upgrade";
    }
    {
      assertion = !config.boot.swraid.enable;
      message = "finite-lat-1 pre-RAID platform candidate must not enable mdraid";
    }
    {
      assertion =
        config.disko.devices.disk.root.device
        == "/dev/disk/by-id/nvme-Micron_7450_MTFDKBA480TFR_24474C59E53F"
        &&
          config.disko.devices.disk.data.device
          == "/dev/disk/by-id/nvme-SAMSUNG_MZQL21T9HCJR-00A07_S64GNC0Y510146";
      message = "finite-lat-1 pre-RAID candidate must retain the existing root and data disks";
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
      message = "finite-lat-1 pre-RAID swap contract drifted";
    }
    {
      assertion =
        config.boot.zswap.enable
        && config.boot.zswap.compressor == "zstd"
        && config.boot.zswap.maxPoolPercent == 10
        && config.boot.zswap.shrinkerEnabled;
      message = "finite-lat-1 pre-RAID zswap contract drifted";
    }
  ];

  # Container-shaped services (dashboard, finite-search) run under podman.
  virtualisation.podman.enable = true;
  virtualisation.oci-containers.backend = "podman";

  time.timeZone = "UTC";
  system.stateVersion = "25.11";
}
