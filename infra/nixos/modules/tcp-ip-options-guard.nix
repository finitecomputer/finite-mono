# CVE-2026-98432: protect host listeners and routed Kata traffic while the
# fleet moves to fixed host and guest kernels. No service restart is needed.
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
