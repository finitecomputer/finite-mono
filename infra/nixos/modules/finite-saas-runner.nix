# finite-saas-runner — the Finite agent-creation runner (Phala backend).
# Mirrors old lat1's finite-saas-runner.service/.timer: a oneshot `run-once`
# re-fired 20s after each completion (OnUnitInactiveSec, not a cron schedule).
#
# ## PHALA CLI TODO
# TODO: the runner shells out to the `phala` CLI (npm package `phala`,
# v1.1.19 on old lat1 at /usr/local/bin/phala). It is NOT packaged here yet:
# a pinned buildNpmPackage needs an npmDepsHash computed on a builder with
# network access. Until that lands, the operator installs it on the host
# (e.g. `nix profile` a node env or `npm i -g phala@1.1.19`) and points
# FC_RUNNER_PHALA_BIN (in /etc/finite/runner.env) at the binary. nodejs is on
# the unit PATH below so a plain npm-installed script works.
{ finitePackages, pkgs, ... }:
{
  systemd.services.finite-saas-runner = {
    description = "Finite agent creation runner";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    path = [ pkgs.nodejs_24 ]; # for the npm-installed phala CLI (see TODO above)

    serviceConfig = {
      Type = "oneshot";
      ExecStart = "${finitePackages.finite-saas-runner}/bin/finite-saas-runner run-once";
      DynamicUser = true;
      StateDirectory = "finite-saas-runner";
      WorkingDirectory = "/var/lib/finite-saas-runner";
      # Operator-created, root:root 0600. Variable NAMES only, from
      # infra/hosts/lat1/systemd/runner.env.example (22 live names; values
      # come from /etc/finite-computer/runner.env on old lat1):
      #   FC_CORE_URL                     -> set to http://127.0.0.1:4200 (core is local now;
      #                                      the old value baked in the k3s ClusterIP)
      #   FC_CORE_API_TOKEN
      #   FC_RUNNER_ID
      #   FC_RUNNER_SOURCE_HOST_ID
      #   FC_RUNNER_BACKEND
      #   FC_RUNNER_RUNTIME_ARTIFACT_ID
      #   FC_RUNNER_FINITE_PRIVATE_BASE_URL
      #   FC_RUNNER_FINITE_PRIVATE_MODEL
      #   PHALA_CLOUD_API_KEY
      #   FC_RUNNER_PHALA_BIN             -> must point at the phala CLI (see TODO)
      #   FC_RUNNER_PHALA_INSTANCE_TYPE
      #   FC_RUNNER_PHALA_DISK_SIZE
      #   FC_RUNNER_PHALA_KMS
      #   FC_RUNNER_PHALA_PUBLIC_LOGS
      #   FC_RUNNER_PHALA_PUBLIC_SYSINFO
      #   FC_RUNNER_PHALA_REGION          (optional)
      #   FC_RUNNER_WORK_ROOT             -> set to /var/lib/finite-saas-runner (was
      #                                      /var/lib/finite/saas-runner on old lat1)
      #   FC_RUNNER_DRAIN
      #   FC_RUNNER_MAX_SANDBOXES
      #   FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS
      #   FC_RUNNER_RUNTIME_READY_INTERVAL_MS
      #   FC_RUNNER_LAUNCH_TIMEOUT_SECS
      #   FC_RUNNER_COMMAND_TIMEOUT_SECS
      EnvironmentFile = "/etc/finite/runner.env";
      # phala CLI wants a writable HOME for its config/cache.
      Environment = [ "HOME=/var/lib/finite-saas-runner" ];
      KillMode = "process"; # from the captured unit
    };
  };

  # 20-second polling loop dressed as a timer — mirrors the captured
  # finite-saas-runner.timer exactly.
  systemd.timers.finite-saas-runner = {
    description = "Run Finite agent creation runner";
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnBootSec = "30s";
      OnUnitInactiveSec = "20s";
      AccuracySec = "1s";
      Unit = "finite-saas-runner.service";
    };
  };
}
