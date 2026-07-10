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

  # The generic kata-v2 shim reads this path; use Cloud Hypervisor and the
  # kernel/image pinned inside the independently locked Kata package.
  environment.etc."kata-containers/configuration.toml".source =
    "${kataPackages.kata-runtime}/share/defaults/kata-containers/configuration-clh.toml";

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

  # containerd discovers runtime-v2 shims by PATH. The Kata package also
  # carries Cloud Hypervisor and the pinned guest assets referenced above.
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
