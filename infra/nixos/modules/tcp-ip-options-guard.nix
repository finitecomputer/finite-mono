# Shared 6.18 host kernel and CVE-2026-98432 guard for host listeners and
# routed Kata traffic.
{ pkgs, ... }:
let
  # First fixed release in the fleet's qualified 6.18 branch. Override only
  # the kernel source, keeping the OS, applications, and Kata pins unchanged.
  kernelVersion = "6.18.56";
  kernel = pkgs.linux_6_18.override {
    argsOverride = {
      version = kernelVersion;
      modDirVersion = kernelVersion;
      src = pkgs.fetchurl {
        url = "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${kernelVersion}.tar.xz";
        sha256 = "4633bbda2ab95eaf2a7af4041236451d6759c72e6946265531af4b3aacefbf89";
      };
    };
  };
in
{
  boot.kernelPackages = pkgs.linuxPackagesFor kernel;

  # FIN-754: drop this backport when the shared kernel pin includes the fix.
  boot.kernelPatches = [
    {
      name = "sctp-f2540da93a11";
      patch = pkgs.fetchurl {
        url = "https://github.com/torvalds/linux/commit/f2540da93a113d3f0a70fd0961e00eb849ce8342.patch";
        hash = "sha256-6QUsVaBA7JP7gmtZrZVjF3JxuAYi3qZq1aXagziBbZw=";
      };
    }
  ];

  boot.kernel.sysctl."net.mptcp.enabled" = 0;

  systemd.services.finite-cve-2026-98432 = {
    description = "Block IPv4 TCP IP options (CVE-2026-98432)";
    wantedBy = [ "multi-user.target" ];
    wants = [ "network-pre.target" ];
    before = [ "network-pre.target" ];
    after = [ "systemd-modules-load.service" ];
    unitConfig.DefaultDependencies = false;
    path = [
      pkgs.iptables
      pkgs.procps
    ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
    };
    # Deliberately no ExecStop: stopping/replacing this unit must not reopen
    # vulnerable listeners during a NixOS switch or rollback.
    script = builtins.readFile ../../scripts/mitigate-cve-2026-98432;
  };
}
