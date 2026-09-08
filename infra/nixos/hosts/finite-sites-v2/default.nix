# finite-sites-v2 — dedicated NixOS validation host for ADR 0028.
#
# This host runs only the static-only Finite Sites v2 service and its edge. It
# is intentionally separate from the current app-plane hosts so v2 can be
# validated before canonical finite.chat DNS moves.
{
  config,
  finitePackages,
  lib,
  ...
}:
let
  operatorKeys = [
    # Paul (same key that already administers the fleet).
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHqbHvWlrXRkTc0403ubkqNE/Ge4YbPvKwWuRBoLPVAW paul@paul.lol"
    # Austin Kelsay.
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICYF8JxzaET1DBnD1WVVpGBj4Sw76950OYip0TrPk+bV austinkelsay@protonmail.com"
    # Alex L.
    "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQC67WRctF9vtJU/3z3q0MCNnAJeQUPHOCY8QIhg4ji6L99gH/IRmyiAE3qA9jivIBCVc6PrbBElcC/w36C9+CymLTWT3K9NLSuC91BZWZpp/HWvyQ6+36P9Qru5UsmzKecHEq2UerJhCJR+Y/qU+r1oLxYqIRx5RkfxurI5PbkvM+Z88/7bBYuxQU3e7PzI11M/bqZsxgCIdkGwPalUVv0iKmxhh8U63How4DqpIJf29bYkti7qKdj+mWaixF+chXu4Xo+JTdarLnGYpxnBjDiYbBhsUF0E+zoHiCSomRne9YVNfS4FIfAn7wSVdrgF2h+opBdwNZ2lRr1StK/x/n7qQJ8wlDLl61pWO3dp3ljTCbVRlIu6y2QNHyFgh8298MiUyuKPFSvELsC9EaPxEXNwnE+rYpdP+fve6sbzjt9lNw2SGUHJONEQgM/0QTiPdxetS83aoL+/kKh1AqaLiz9bh5KOi09EfxgYdRWeHdITCufT6j20JNoBcIBJmrEvlSs= alex@AlexLs-MacBook-Pro.local"
  ];
in
{
  imports = [
    ../../modules/finitesitesd.nix
    ../../modules/caddy-sites-v2.nix
    ../../modules/finite-sites-v2-backups.nix
  ];

  networking.hostName = "finite-sites-v2";
  networking.useDHCP = lib.mkDefault true;
  networking.firewall = {
    enable = true;
    allowedTCPPorts = [
      22
      80
      443
    ];
    logRefusedConnections = true;
  };

  finite.sites = {
    mode = "static-v2";
    package = finitePackages.finitesitesd;
    baseDomain = "v2.finite.chat";
    apiUrl = "https://v2.finite.chat";
    gitUrl = "https://v2.finite.chat";
  };
  finite.sitesV2Backup.enable = true;

  services.openssh = {
    enable = true;
    settings = {
      PermitRootLogin = "prohibit-password";
      PasswordAuthentication = false;
      KbdInteractiveAuthentication = false;
    };
  };
  users.users.root.openssh.authorizedKeys.keys = operatorKeys;

  services.journald.extraConfig = ''
    Storage=persistent
  '';

  boot.loader.grub.enable = lib.mkDefault true;
  boot.loader.grub.device = lib.mkDefault "/dev/sda";
  boot.initrd.availableKernelModules = [
    "nvme"
    "xhci_pci"
    "ahci"
    "usbhid"
    "sd_mod"
    "ext4"
  ];
  fileSystems."/" = {
    device = "/dev/disk/by-label/nixos";
    fsType = "ext4";
  };

  time.timeZone = "UTC";
  system.stateVersion = "26.05";

  assertions = [
    {
      assertion = config.system.nixos.release == "26.05";
      message = "finite-sites-v2 must stay on the same qualified NixOS 26.05 platform pin as the LAT hosts";
    }
    {
      assertion =
        config.networking.firewall.allowedTCPPorts == [
          22
          80
          443
        ];
      message = "finite-sites-v2 public exposure must stay exactly ssh/http/https";
    }
  ];
}
