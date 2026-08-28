# finite-lat-2 — 64.34.80.19 — THE replacement single app server.
#
# Emergency cutover target for finite-lat-1 (thermal failure, 2026-08-27):
# lat1's full service stack (Core, Postgres, chat, hosted-device, sites,
# Brain, Identity, dashboard, search, Caddy edge, backups, litestream)
# cloned onto the lat3-qualified storage chassis (mirrored root + data,
# dual ESPs, fail-closed storage health), MINUS every Agent Runner piece —
# the runner lane moves to a separate host. Authority:
# ADR 0007.
#
# The host boots in import mode (finite.importMode.enable): product units do
# not start until lat1's state is imported and verified; the go-live closure
# flips that option. Storage identities are placeholders until the Gate A
# capture (scripts/capture-lat2-host-evidence) is reviewed into
# ./storage-ids.nix.
{
  config,
  finitePackages,
  lib,
  pkgs,
  ...
}:
let
  ids = import ./storage-ids.nix;
  paulKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHqbHvWlrXRkTc0403ubkqNE/Ge4YbPvKwWuRBoLPVAW paul@paul.lol";
  austinKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICYF8JxzaET1DBnD1WVVpGBj4Sw76950OYip0TrPk+bV austinkelsay@protonmail.com";
  alexKey = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQC67WRctF9vtJU/3z3q0MCNnAJeQUPHOCY8QIhg4ji6L99gH/IRmyiAE3qA9jivIBCVc6PrbBElcC/w36C9+CymLTWT3K9NLSuC91BZWZpp/HWvyQ6+36P9Qru5UsmzKecHEq2UerJhCJR+Y/qU+r1oLxYqIRx5RkfxurI5PbkvM+Z88/7bBYuxQU3e7PzI11M/bqZsxgCIdkGwPalUVv0iKmxhh8U63How4DqpIJf29bYkti7qKdj+mWaixF+chXu4Xo+JTdarLnGYpxnBjDiYbBhsUF0E+zoHiCSomRne9YVNfS4FIfAn7wSVdrgF2h+opBdwNZ2lRr1StK/x/n7qQJ8wlDLl61pWO3dp3ljTCbVRlIu6y2QNHyFgh8298MiUyuKPFSvELsC9EaPxEXNwnE+rYpdP+fve6sbzjt9lNw2SGUHJONEQgM/0QTiPdxetS83aoL+/kKh1AqaLiz9bh5KOi09EfxgYdRWeHdITCufT6j20JNoBcIBJmrEvlSs= alex@AlexLs-MacBook-Pro.local";
  operatorKeys = [
    paulKey
    austinKey
    alexKey
  ];

  # Product units held down while import mode is on: everything that writes
  # user state or serves the edge. Postgres, sshd, and node-exporter stay up
  # (postgres is the pg_restore target); litestream/borg/healthcheck wait
  # with the rest so they never see a half-imported state.
  importModeUnits = [
    "caddy.service"
    "finite-saas-core.service"
    "finitechat-server.service"
    "finitechat-hosted-device.service"
    "finite-saas-sites.service"
    "finite-brain-app.service"
    "finite-identity.service"
    "podman-finite-saas-dashboard.service"
    "podman-searxng.service"
    "podman-firecrawl-api.service"
    "init-firecrawl-network.service"
    "finite-healthcheck.service"
    "finite-healthcheck.timer"
    "finite-postgres-backup.service"
    "finite-postgres-backup.timer"
    "finite-litestream-finite-chat-server.service"
    "finite-litestream-finite-brain.service"
    "finite-litestream-health.service"
    "finite-litestream-health.timer"
    "finite-hosted-web-chat-snapshot.service"
    "finite-hosted-web-chat-snapshot-health.service"
    "finite-hosted-web-chat-snapshot-health.timer"
    "finite-identity-backup.service"
    "finite-identity-backup-health.service"
    "finite-identity-backup-health.timer"
    "borgbackup-job-finite-hosted-web-chat-offsite.service"
    "finite-hosted-web-chat-offsite-health.service"
    "finite-hosted-web-chat-offsite-health.timer"
    "finite-runtime-metrics.service"
    "finite-runtime-metrics.timer"
  ];
in
{
  imports = [
    ./disko.nix
    ./invariants.nix
    ./storage-health.nix
    ../../modules/import-mode.nix
    # Sites' tier-2 Kata apps need the shared containerd host runtime; this
    # host runs no Agent Runner, so it imports the runtime without the role.
    ../../modules/kata-host-runtime.nix
    ../../modules/finite-saas-core.nix
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
    ../../modules/finite-litestream.nix
  ];

  networking.hostName = "finite-lat-2";

  # Start-down: flip to false only in the go-live closure, after the state
  # import has passed every verification gate (see the cutover runbook).
  finite.importMode = {
    enable = true;
    units = importModeUnits;
  };

  # Reuse the existing finitecomputer rsync.net destination account, with a
  # repository dedicated to lat-2 so lat-1's frozen archives are never
  # appended to by a different machine.
  finite.recoveryBackup.borgRepository = "fm2890@fm2890.rsync.net:finitecomputer/finite-lat-2";

  # Continuous chat + Brain SQLite replication. New bucket for the new
  # authority host; lat-1's bucket stays frozen as the outage point-in-time
  # record (the import itself restores FROM finite-lat-1-litestream).
  finite.litestream = {
    enable = true;
    replica = {
      endpoint = "https://objects.chi.storage.sh";
      bucket = "finite-lat-2-litestream";
    };
    dbs = [
      {
        # 9351 predates the per-db split and is referenced by runbooks —
        # keep it stable for the chat instance.
        name = "finite-chat-server";
        path = "/var/lib/private/finite-chat/data/server.sqlite3";
        owningService = "finitechat-server.service";
        metricsAddress = "127.0.0.1:9351";
      }
      {
        name = "finite-brain";
        path = "/var/lib/private/finitebrain/finite-brain.sqlite3";
        owningService = "finite-brain-app.service";
        metricsAddress = "127.0.0.1:9352";
      }
    ];
  };

  assertions = [
    {
      assertion = config.system.nixos.release == "26.05";
      message = "finite-lat-2 must stay on the qualified NixOS 26.05 release pin";
    }
    {
      # lat1 parity: identical module defaults for postgres, caddy, chat.
      # Do not bump while this host is lat1's replacement.
      assertion = config.system.stateVersion == "25.11";
      message = "finite-lat-2 must keep lat1's stateVersion for a behavior-identical service stack";
    }
    {
      assertion = config.fileSystems."/".device == "/dev/md/root";
      message = "finite-lat-2 root must be the named root MD array";
    }
    {
      assertion = config.fileSystems."/data".device == "/dev/md/data";
      message = "finite-lat-2 /data must be the named data MD array";
    }
    {
      # Sites' tier-2 apps run through the shared Kata host runtime; the KVM
      # module must be loaded or every Kata guest launch fails at runtime.
      assertion =
        !config.virtualisation.containerd.enable
        || builtins.elem "kvm-amd" config.boot.kernelModules;
      message = "finite-lat-2 Kata app runtime requires the kvm-amd kernel module";
    }
  ];

  networking.useDHCP = false;
  networking.useNetworkd = true;
  systemd.network.enable = true;
  systemd.network.networks = {
    "10-wan" = {
      matchConfig.MACAddress = "REPLACE-ME-wan-nic-mac";
      address = [
        "64.34.80.19/31"
        "REPLACE-ME-ipv6-address"
      ];
      routes = [
        { Gateway = "REPLACE-ME-ipv4-gateway"; }
        { Gateway = "REPLACE-ME-ipv6-gateway"; }
      ];
      networkConfig = {
        DHCP = "no";
        IPv6AcceptRA = false;
      };
      linkConfig = {
        RequiredForOnline = "routable";
        RequiredFamilyForOnline = "ipv4";
      };
    };

    "20-unused-lan" = {
      matchConfig.MACAddress = "REPLACE-ME-lan-nic-mac";
      networkConfig = {
        DHCP = "no";
        IPv6AcceptRA = false;
        LinkLocalAddressing = "no";
      };
      linkConfig.RequiredForOnline = "no";
    };
  };
  networking.nameservers = [
    "1.1.1.1"
    "8.8.8.8"
  ];

  # This host inherits lat1's role as the wg-finite overlay hub at
  # 10.254.3.1: the Core socket proxy and Identity Authority proxy live
  # here, and the runner hosts (lat3 now, lat4 later) peer with it.
  networking.wireguard.interfaces."wg-finite" = {
    ips = [ "10.254.3.1/29" ];
    listenPort = 51820;
    privateKeyFile = "/etc/finite/wireguard-private-key";
    peers = [
      {
        # finite-lat-3 runner host.
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
    allowedUDPPorts = [ 51820 ];
    # Preserve the bounded live rules: only finite-lat-3's public address can
    # establish WireGuard, and only that authenticated overlay address can
    # reach the private Core and Identity proxies. extraCommands run before
    # the final reject rule in the iptables firewall.
    extraCommands = ''
      iptables -w -A nixos-fw \
        -s 207.188.7.157/32 -d 64.34.80.19/32 \
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
        -p tcp --dport 18790 \
        -m comment --comment finite-lat3-identity \
        -j nixos-fw-accept
    '';
  };

  # Private finite-lat Runner access to Core (lat3 and future lat4 runners
  # lease creation through this proxy; this host runs no runner itself).
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
  # route. Give only the authenticated runner WireGuard peers a private
  # path to the same loopback Authority used by trusted same-host products.
  systemd.sockets.finite-identity-private-proxy = {
    description = "Private finite-lat Runner access to Identity Authority";
    wantedBy = [ "sockets.target" ];
    listenStreams = [ "10.254.3.1:18790" ];
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
  users.users = {
    root.openssh.authorizedKeys.keys = operatorKeys;
    ubuntu = {
      isNormalUser = true;
      extraGroups = [ "wheel" ];
      openssh.authorizedKeys.keys = operatorKeys;
    };
  };
  security.sudo.wheelNeedsPassword = false;

  boot.loader = {
    timeout = 5;
    efi.canTouchEfiVariables = false;
    grub = {
      enable = true;
      efiSupport = true;
      efiInstallAsRemovable = true;
      configurationLimit = 20;
      mirroredBoots = [
        {
          path = "/boot-a";
          devices = [ "nodev" ];
        }
        {
          path = "/boot-b";
          devices = [ "nodev" ];
        }
      ];
    };
  };

  boot.initrd = {
    systemd.enable = true;
    availableKernelModules = [
      "nvme"
      "xhci_pci"
      "ahci"
      "usbhid"
      "sd_mod"
      "ext4"
      "vfat"
      "md_mod"
      "raid1"
    ];
  };
  # Sites' tier-2 Kata apps run guests through QEMU/KVM via the shared Kata
  # host runtime: the KVM module must be loaded at boot even though this
  # host runs no Agent Runner. Assertion below fails the build if the
  # runtime and the module ever drift apart.
  boot.kernelModules = [ "kvm-amd" ];
  # The BMC's ASPEED adapter owns the host console on this chassis class; the
  # unused iGPU has no firmware on this headless server and otherwise logs
  # fatal amdgpu initialization errors on every boot.
  boot.blacklistedKernelModules = [ "amdgpu" ];
  boot.kernelParams = [ "panic=30" ];
  boot.swraid = {
    enable = true;
    mdadmConf = ''
      HOMEHOST <ignore>
      MAILADDR root
      ARRAY /dev/md/root metadata=1.2 UUID=${ids.mdUuids.root}
      ARRAY /dev/md/data metadata=1.2 UUID=${ids.mdUuids.data}
    '';
  };

  fileSystems."/".neededForBoot = true;
  fileSystems."/data".neededForBoot = false;
  fileSystems."/boot-a".neededForBoot = false;
  fileSystems."/boot-b".neededForBoot = false;

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

  services.fstrim.enable = true;
  services.smartd = {
    enable = true;
    autodetect = true;
    notifications.wall.enable = true;
  };
  services.journald.extraConfig = ''
    Storage=persistent
  '';

  systemd.tmpfiles.rules = [
    "d /data/backups 0700 root root - -"
    "z /etc/finite/wireguard-private-key 0600 root root - -"
  ];

  # Container-shaped services (dashboard, finite-search) run under podman.
  virtualisation.podman.enable = true;
  virtualisation.oci-containers.backend = "podman";

  environment.systemPackages = with pkgs; [
    e2fsprogs
    gptfdisk
    mdadm
    nvme-cli
    pciutils
    smartmontools
  ];

  time.timeZone = "UTC";
  system.stateVersion = "25.11";
}
