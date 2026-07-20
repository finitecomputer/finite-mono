{
  config,
  lib,
  pkgs,
  ...
}:
let
  ids = import ./storage-ids.nix;
  paulKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHqbHvWlrXRkTc0403ubkqNE/Ge4YbPvKwWuRBoLPVAW paul@paul.lol";
in
{
  imports = [
    ./disko.nix
    ./invariants.nix
    ./storage-health.nix
    ../../modules/finite-saas-runner.nix
  ];

  networking.hostName = "finite-lat-3";

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
  ];

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
    root.openssh.authorizedKeys.keys = [ paulKey ];
    ubuntu = {
      isNormalUser = true;
      extraGroups = [ "wheel" ];
      openssh.authorizedKeys.keys = [ paulKey ];
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

  # Install and exercise the Runner while it is explicitly drained. The
  # timer becomes a declarative boot service only after the synthetic Agent
  # passes; until then operators invoke the oneshot unit deliberately.
  systemd.timers.finite-saas-runner.wantedBy = lib.mkForce [ ];
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
