# Shared Kata host runtime: rootful containerd with the kata-v2 runtime
# handler, the pinned guest configuration, and the finite0 CNI network.
#
# The finite-saas-runner worker imports this via finite-saas-runner.nix. This
# module deliberately carries NO runner role — no worker unit, no capacity,
# no credentials. Sites no longer consumes this runtime after ADR 0028.
{
  kataPackages,
  lib,
  pkgs,
  ...
}:
{
  # Rootful containerd owns replaceable Kata compute. Durable agent state is
  # outside containerd under /var/lib/finite-saas-runner/kata and is never
  # deleted by restart/stop/destroy runtime-control operations.
  virtualisation.containerd = {
    enable = true;
    settings.plugins."io.containerd.grpc.v1.cri".containerd.runtimes.kata = {
      runtime_type = "io.containerd.kata.v2";
      privileged_without_host_devices = true;
    };
  };

  # The generic kata-v2 shim reads this path. Use the QEMU backend shipped by
  # the locked Nix Kata package; its generated config pins the QEMU binary,
  # guest kernel, and guest image in the same closure.
  # The stock defaults (1 vCPU / 2048 MiB) size every sandbox VM: the runner
  # launches through nerdctl, so OCI-level --cpus/--memory limits never reach
  # hypervisor sizing (static_sandbox_resource_mgmt reads CRI pod annotations
  # only). Patch the defaults to the declared Standard 4 vCPU / 8 GiB envelope.
  environment.etc."kata-containers/configuration.toml".source =
    pkgs.runCommand "kata-configuration-qemu-finite.toml" { }
      ''
        sed -e 's/^default_vcpus = .*/default_vcpus = 4/' \
            -e 's/^default_memory = .*/default_memory = 8192/' \
            ${kataPackages.kata-runtime}/share/defaults/kata-containers/configuration-qemu.toml \
            > "$out"
        grep -q '^default_vcpus = 4$' "$out"
        grep -q '^default_memory = 8192$' "$out"
      '';

  # nerdctl's rootful CNI network is declared rather than generated at first
  # launch, keeping host rebuilds reproducible and port publishing available.
  environment.etc."cni/net.d/10-finite.conflist".text = builtins.toJSON {
    cniVersion = "1.0.0";
    name = "finite";
    plugins = [
      {
        type = "bridge";
        bridge = "finite0";
        isGateway = true;
        ipMasq = true;
        hairpinMode = true;
        ipam = {
          type = "host-local";
          ranges = [
            [
              { subnet = "10.89.0.0/16"; }
            ]
          ];
          routes = [
            { dst = "0.0.0.0/0"; }
          ];
        };
      }
      {
        type = "portmap";
        capabilities.portMappings = true;
      }
      {
        type = "firewall";
      }
      {
        type = "tuning";
      }
    ];
  };

  # containerd discovers runtime-v2 shims by PATH. The Kata package closure
  # also carries QEMU and the pinned guest assets referenced above.
  systemd.services.containerd.path = lib.mkAfter [ kataPackages.kata-runtime ];

  environment.systemPackages = [
    kataPackages.kata-runtime
    kataPackages.nerdctl
    kataPackages.cni-plugins
    pkgs.borgbackup
    pkgs.containerd
  ];
}
