# finite-saas-runner — provider-neutral runtime worker. Production advertises
# the Kata runner class; Core leases only Projects that selected that class.
# Product features stay inside the agent and finite-agentd, never in this unit.
{
  finitePackages,
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
  # only). Patch the defaults to a 2 vCPU / 8 GiB envelope (Paul, 2026-07-14).
  environment.etc."kata-containers/configuration.toml".source =
    pkgs.runCommand "kata-configuration-qemu-finite.toml" { } ''
      sed -e 's/^default_vcpus = .*/default_vcpus = 2/' \
          -e 's/^default_memory = .*/default_memory = 8192/' \
          ${kataPackages.kata-runtime}/share/defaults/kata-containers/configuration-qemu.toml \
          > "$out"
      grep -q '^default_vcpus = 2$' "$out"
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
  ];

  systemd.services.finite-saas-runner = {
    description = "Finite Kata runtime worker";
    wants = [ "network-online.target" ];
    requires = [ "containerd.service" ];
    after = [
      "network-online.target"
      "containerd.service"
    ];
    path = [
      kataPackages.kata-runtime
      kataPackages.nerdctl
      kataPackages.cni-plugins
      pkgs.iproute2
      pkgs.iptables
    ];

    serviceConfig = {
      Type = "oneshot";
      ExecStart = "${finitePackages.finite-saas-runner}/bin/finite-saas-runner run-once";

      # Rootful nerdctl needs the containerd socket and CNI network namespace
      # capabilities. This unit remains narrow: it only runs the typed runtime
      # adapter and never hosts product features or edits agent state.
      DynamicUser = lib.mkForce false;
      User = "root";
      Group = "root";
      UMask = "0077";
      StateDirectory = "finite-saas-runner";
      WorkingDirectory = "/var/lib/finite-saas-runner";

      # Operator-created root:root 0600. Names and examples live in
      # infra/hosts/lat1/systemd/runner.env.example; values stay host-only.
      EnvironmentFile = "/etc/finite/runner.env";
      Environment = [
        "HOME=/var/lib/finite-saas-runner"
        "CNI_PATH=${kataPackages.cni-plugins}/bin"
        "CONTAINERD_ADDRESS=/run/containerd/containerd.sock"
      ];
      KillMode = "process";
    };
  };

  # One bounded lease attempt every 20 seconds. Adapter readiness and Core
  # capacity matching happen before a Project can be claimed.
  systemd.timers.finite-saas-runner = {
    description = "Run Finite Kata runtime worker";
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnBootSec = "30s";
      OnUnitInactiveSec = "20s";
      AccuracySec = "1s";
      Unit = "finite-saas-runner.service";
    };
  };
}
