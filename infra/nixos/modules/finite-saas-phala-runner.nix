# Single-canary Phala worker definition. This is deliberately a second
# one-class worker, not a provider switch in the Kata service. Core's durable
# provider-operation journal plus the exact one-resource reservation fence are
# the only creation path.
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
    description = "Finite Phala confidential runtime worker (single canary)";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];

    wantedBy = [ "multi-user.target" ];
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

      # The ACTIVE readiness run authorizes exactly one internal Confidential
      # launch. Provider inventory plus Core in-flight reservations enforce
      # this cap across worker restarts and ambiguous provider responses.
      FC_RUNNER_DRAIN = "false";
      FC_RUNNER_MAX_SANDBOXES = "1";
      FC_RUNNER_PHALA_EXPECTED_WORKSPACE_ID = "wks_YKRQqRea";
      FC_RUNNER_PHALA_EXPECTED_WORKSPACE_SLUG = "finite";
      FC_RUNNER_RUNTIME_ARTIFACT_ID = "finite-agent-runtime-2026-07-22.1";
      FC_RUNNER_RUNTIME_ENV_JSON = builtins.toJSON {
        FINITE_SITES_API = "https://api.finite.chat";
        FINITE_BRAIN_SERVER_URL = "https://brain.finite.computer";
        FINITE_BRAIN_PUBLIC_BASE_URL = "https://brain.finite.computer";
      };
      # systemd expands %d to the private credential directory. The worker
      # receives a read-only copy without gaining access to /etc/finite.
      FC_RUNNER_RUNTIME_SECRET_ENV_FILE = "%d/runtime-secrets.env";

      # The HTTPS adapter pins the API origin/version, exact Medium/40 GB
      # shape, Cloud KMS, and private-log policy in code. None is a deploy-time
      # provider knob. The only intended network destinations are loopback
      # Core and cloud-api.phala.com:443.
    };

    serviceConfig = {
      Type = "simple";
      ExecStart = "${finitePackages.finite-saas-runner}/bin/finite-saas-runner serve";

      # Operator-created root:root 0600. It contains only this worker's
      # route-scoped Core token, Phala API key, and specialization credential.
      EnvironmentFile = [ "/etc/finite/phala-runner.env" ];
      LoadCredential = "runtime-secrets.env:/etc/finite/runtime-secrets.env";

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

  # Evaluation-time guardrails keep later refactors from widening the
  # authorized one-canary API worker into a privileged provider shell.
  assertions = [
    {
      assertion = service.wantedBy == [ "multi-user.target" ];
      message = "the authorized Phala canary worker must start at multi-user.target";
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
      assertion = service.environment.FC_RUNNER_DRAIN == "false";
      message = "the authorized Phala canary worker must admit its single creation lease";
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
