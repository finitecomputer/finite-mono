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
  # Operator routes are deliberately absent from the public Identity vhost.
  # Reach the loopback Authority through lat1's peer-scoped WireGuard proxy.
  identityAuthority = "http://10.254.3.1:18790";
  identityOperatorEnvironmentFile = "/etc/finite/identity-operator.env";
  revision =
    if config.system.configurationRevision == null then "" else config.system.configurationRevision;
in
{
  imports = [
    ./disko.nix
    ./invariants.nix
    ./storage-health.nix
    ../../modules/finite-saas-runner.nix
    ../../modules/kata-runner-host.nix
    ../../modules/metrics.nix
  ];

  networking.hostName = "finite-lat-3";

  # Shared Kata Runner role (modules/kata-runner-host.nix); only genuine host
  # differences are declared here. Core is remote, so sandboxes are reached
  # through this host's private WireGuard overlay address.
  finite.kataRunnerHost = {
    coreUrl = "http://10.254.3.1:14200";
    runnerId = "finite-kata-runner-3";
    sourceHostId = "finite-lat-3";
    workRoot = "/data/finite-saas-runner";
    kataHostAddress = "10.254.3.2";
    maxSandboxes = 32;
  };

  finite.metrics = {
    enable = true;
    staticVersionMetrics = ''
      finite_component_build_info{host="finite-lat-3",component="finite-saas-runner",version="${finitePackages.finite-saas-runner.version}",git_sha="${revision}",image_digest="",source="nix"} 1
      finite_component_version_mismatch{host="finite-lat-3",component="finite-saas-runner"} 0
      finite_component_build_info{host="finite-lat-3",component="nixos-system-profile",version="${config.system.nixos.version}",git_sha="${revision}",image_digest="",source="nix"} 1
      finite_component_version_mismatch{host="finite-lat-3",component="nixos-system-profile"} 0
    '';
  };

  assertions = [
    {
      assertion = lib.versions.majorMinor config.boot.kernelPackages.kernel.version == "6.18";
      message = "finite-lat-3 arrays must be created and booted with the pinned Linux 6.18 NixOS 26.05 kernel";
    }
    {
      assertion = config.fileSystems."/".device == "/dev/md/root";
      message = "finite-lat-3 root must be the named root MD array";
    }
    {
      assertion = config.fileSystems."/data".device == "/dev/md/data";
      message = "finite-lat-3 /data must be the named data MD array";
    }
    {
      assertion =
        config.systemd.services.finite-saas-runner.environment.FINITE_IDENTITY_AUTHORITY
        == identityAuthority;
      message = "finite-lat-3 Runner must use the private production Identity Authority";
    }
    {
      assertion = builtins.elem identityOperatorEnvironmentFile config.systemd.services.finite-saas-runner.serviceConfig.EnvironmentFile;
      message = "finite-lat-3 Runner must load its Identity Authority operator credential";
    }
  ];

  # Managed Agent Email registration is part of successful creation. Keep the
  # replaceable operator credential in its own root-only file and out of the
  # general Runner environment template.
  systemd.services.finite-saas-runner = {
    environment.FINITE_IDENTITY_AUTHORITY = identityAuthority;
    serviceConfig.EnvironmentFile = lib.mkAfter [ identityOperatorEnvironmentFile ];
  };

  networking.useDHCP = false;
  networking.useNetworkd = true;
  systemd.network.enable = true;
  systemd.network.networks = {
    "10-wan" = {
      matchConfig.MACAddress = "90:5a:08:31:e5:17";
      address = [
        "207.188.7.157/31"
        "2605:6440:5002:202::2/64"
      ];
      routes = [
        { Gateway = "207.188.7.156"; }
        { Gateway = "2605:6440:5002:202::1"; }
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
      matchConfig.MACAddress = "90:5a:08:31:e5:16";
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
    "2606:4700:4700::1111"
    "2001:4860:4860::8888"
  ];

  networking.wireguard.interfaces."wg-finite" = {
    ips = [ "10.254.3.2/30" ];
    listenPort = 51820;
    privateKeyFile = "/etc/finite/wireguard-private-key";
    peers = [
      {
        publicKey = "UM5bBdhEj15t+bt+UWz7q4iXH0EgYx9p+CQY/E+31Us=";
        allowedIPs = [ "10.254.3.1/32" ];
        endpoint = "64.34.82.77:51820";
        persistentKeepalive = 25;
      }
    ];
  };

  networking.firewall = {
    enable = true;
    allowedTCPPorts = [ 22 ];
    allowedUDPPorts = [ 51820 ];
    # containerd asks the kernel for a dynamic host port. It binds only the
    # overlay address; only the sole authenticated WireGuard peer can enter.
    interfaces."wg-finite".allowedTCPPortRanges = [
      {
        from = 32768;
        to = 60999;
      }
    ];
    logRefusedConnections = true;
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
  boot.kernelModules = [ "kvm-amd" ];
  # The BMC's ASPEED adapter owns the host console. The unused Raphael iGPU
  # has no firmware on this headless runner and otherwise logs fatal amdgpu
  # initialization errors on every boot.
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

  boot.kernel.sysctl."net.ipv4.ip_local_port_range" = "32768 60999";

  systemd.tmpfiles.rules = [
    "d /data/finite-saas-runner 0700 root root - -"
    "z /etc/finite/wireguard-private-key 0600 root root - -"
  ];

  # Accept new Standard-Agent creation on the recurring Runner schedule.
  # /etc/finite/runner.env supplies the host credential, drain state, and
  # hard sandbox limit.
  systemd.services.finite-saas-runner.unitConfig.ConditionPathExists = "/etc/finite/runner.env";

  environment.systemPackages = with pkgs; [
    e2fsprogs
    gptfdisk
    mdadm
    nvme-cli
    pciutils
    quota
    smartmontools
  ];

  time.timeZone = "UTC";
  system.stateVersion = "26.05";
}
