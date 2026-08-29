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
  imports = [ ./kata-host-runtime.nix ];

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
      pkgs.borgbackup
      pkgs.containerd
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
      # kata-runner-host.nix prepends the Nix-rendered shared non-secret
      # defaults, so this file keeps credentials, the promoted Runtime
      # artifact pin (FC_RUNNER_RUNTIME_ARTIFACT_ID), and bounded incident
      # overrides.
      EnvironmentFile = [ "/etc/finite/runner.env" ];
      Environment = [
        "HOME=/var/lib/finite-saas-runner"
        "CNI_PATH=${kataPackages.cni-plugins}/bin"
        "CONTAINERD_ADDRESS=/run/containerd/containerd.sock"
      ];
      KillMode = "process";
    };
  };

  # One bounded lease attempt every 5 seconds. Adapter readiness and Core
  # capacity matching happen before a Project can be claimed.
  systemd.timers.finite-saas-runner = {
    description = "Run Finite Kata runtime worker";
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnBootSec = "30s";
      OnUnitInactiveSec = "5s";
      AccuracySec = "1s";
      Unit = "finite-saas-runner.service";
    };
  };
}
