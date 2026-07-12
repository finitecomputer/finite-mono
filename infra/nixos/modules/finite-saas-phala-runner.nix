# Dark Phala worker definition. This is deliberately a second one-class
# worker, not a provider switch in the Kata service. It has no wantedBy/timer
# and remains drained until a later, separately authorized generation enables
# Confidential placement after the API adapter and recovery gates pass.
{
  config,
  finitePackages,
  lib,
  ...
}:
let
  serviceName = "finite-saas-runner-phala";
  service = config.systemd.services.${serviceName};
in
{
  systemd.services.${serviceName} = {
    description = "Finite Phala confidential runtime worker (dark)";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];

    # Intentionally no wantedBy and no timer. Merely deploying this module
    # cannot contact Phala, advertise capacity, or claim a Core lease.
    wantedBy = [ ];
    path = [ ];
    startLimitIntervalSec = 300;
    startLimitBurst = 3;

    # These non-secret facts bind this process to exactly one internal class
    # and one worker/source-host identity. Core must bind the separate token in
    # phala-runner.env to these same facts.
    environment = {
      FC_CORE_URL = "http://127.0.0.1:4200";
      FC_RUNNER_ID = "finite-phala-runner-1";
      FC_RUNNER_SOURCE_HOST_ID = "finite-lat-1-phala-control-1";
      FC_RUNNER_CLASS = "phala";
      FC_RUNNER_WORK_ROOT = "/var/lib/finite-saas-runner-phala";

      # Dark means no new creation leases. Existing-runtime controls remain a
      # separate contract; changing this value requires a reviewed Nix deploy.
      FC_RUNNER_DRAIN = "true";
      FC_RUNNER_MAX_SANDBOXES = "1";

      # The HTTPS adapter pins the API origin/version in code. Do not add a
      # provider URL override or a CLI binary here. The only intended network
      # destinations are loopback Core and cloud-api.phala.com:443.
      FC_RUNNER_PHALA_INSTANCE_TYPE = "tdx.medium";
      FC_RUNNER_PHALA_DISK_SIZE = "40G";
      FC_RUNNER_PHALA_KMS = "PHALA";
      FC_RUNNER_PHALA_PUBLIC_LOGS = "false";
      FC_RUNNER_PHALA_PUBLIC_SYSINFO = "false";
    };

    serviceConfig = {
      Type = "simple";
      ExecStart = "${finitePackages.finite-saas-runner}/bin/finite-saas-runner serve";

      # Operator-created root:root 0600. It contains only this worker's
      # route-scoped Core token, Phala API key, and promoted artifact id.
      EnvironmentFile = "/etc/finite/phala-runner.env";

      DynamicUser = true;
      User = serviceName;
      Group = serviceName;
      UMask = "0077";
      StateDirectory = serviceName;
      StateDirectoryMode = "0700";
      WorkingDirectory = "/var/lib/${serviceName}";

      # No provider CLI, host runtime, Kata, CNI, containerd, Docker, Podman,
      # or device authority is available to this API-only worker.
      CapabilityBoundingSet = "";
      AmbientCapabilities = "";
      DevicePolicy = "closed";
      InaccessiblePaths = [
        "-/run/containerd"
        "-/run/k3s"
        "-/run/podman"
        "-/run/docker.sock"
        "-/var/run/docker.sock"
        "-/etc/cni"
        "-/opt/cni"
      ];
      NoNewPrivileges = true;
      PrivateDevices = true;
      PrivateMounts = true;
      PrivateTmp = true;
      ProtectClock = true;
      ProtectControlGroups = true;
      ProtectHome = true;
      ProtectHostname = true;
      ProtectKernelLogs = true;
      ProtectKernelModules = true;
      ProtectKernelTunables = true;
      ProtectProc = "invisible";
      ProtectSystem = "strict";
      ProcSubset = "pid";
      LockPersonality = true;
      MemoryDenyWriteExecute = true;
      RemoveIPC = true;
      RestrictAddressFamilies = [
        "AF_UNIX"
        "AF_INET"
        "AF_INET6"
      ];
      RestrictNamespaces = true;
      RestrictRealtime = true;
      RestrictSUIDSGID = true;
      SystemCallArchitectures = "native";
      SystemCallFilter = [
        "@system-service"
        "~@privileged"
        "~@resources"
      ];

      Restart = "on-failure";
      RestartSec = "30s";
      TimeoutStartSec = "120s";
      TimeoutStopSec = "30s";
    };
  };

  # Evaluation-time guardrails keep later refactors from accidentally turning
  # the dark API worker into a privileged or automatically started provider
  # shell. The root CI Nix eval exercises these assertions.
  assertions = [
    {
      assertion = service.wantedBy == [ ];
      message = "the dark Phala worker must not be wanted by any target";
    }
    {
      assertion = !(builtins.hasAttr serviceName config.systemd.timers);
      message = "the dark Phala worker must not have a timer";
    }
    {
      assertion = service.environment.FC_RUNNER_CLASS == "phala";
      message = "the Phala worker must advertise only the phala class";
    }
    {
      assertion = service.environment.FC_RUNNER_DRAIN == "true";
      message = "the dark Phala worker must reject new creation leases";
    }
    {
      assertion = service.environment.FC_RUNNER_MAX_SANDBOXES == "1";
      message = "the ordinary Phala billable-resource cap must remain one";
    }
    {
      assertion = builtins.all (
        entry:
        let
          rendered = toString entry;
        in
        !(lib.hasInfix "phala" rendered)
        && !(lib.hasInfix "kata" rendered)
        && !(lib.hasInfix "nerdctl" rendered)
        && !(lib.hasInfix "containerd" rendered)
      ) service.path;
      message = "the Phala worker must not gain provider or host-runtime CLI packages";
    }
    {
      assertion = !(builtins.elem "containerd.service" service.requires);
      message = "the Phala worker must not require containerd";
    }
    {
      assertion = service.serviceConfig.DynamicUser == true;
      message = "the Phala worker must remain unprivileged and dynamically allocated";
    }
    {
      assertion = service.serviceConfig.StateDirectory == serviceName;
      message = "the Phala worker must keep a distinct durable adapter state directory";
    }
  ];
}
