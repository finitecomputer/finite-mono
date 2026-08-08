# Shared Kata Runner host role for finite-lat-1 and finite-lat-3. Both hosts
# run the same finite-saas-runner worker, but their operator-managed
# /etc/finite/runner.env copies drifted (a hand-set 30s Kata stop timeout on
# both hosts caused two false upgrade failures and halted a 25-Agent rollout
# on 2026-08-05 until operators raised it to 180). This module is the ONE
# declaration of the role: the shared non-secret environment is rendered once
# to /etc/finite/runner-shared.env, and host configs pass only genuine
# differences through finite.kataRunnerHost.*. Runner-role changes land here;
# host configs hold only per-host values.
#
# The operator-managed /etc/finite/runner.env is still loaded AFTER the shared
# file, so its credentials stay host-only and a bounded operator override
# remains available during an incident. Secret values never enter the shared
# file or this repo.
#
# Host configs import finite-saas-runner.nix directly BEFORE this module:
# routing the base module only through this module's own import changes
# definition merge order and needlessly rewrites rendered unit lines.
{
  config,
  lib,
  ...
}:
let
  cfg = config.finite.kataRunnerHost;

  # Non-secret environment identical on both Kata Runner hosts. The
  # runner.env.example files document the remaining operator-managed keys
  # (credentials, drain state, deliberate overrides).
  sharedEnvironment = {
    FC_RUNNER_CLASS = "kata";
    FC_RUNNER_RUNTIME_ARTIFACT_ID = "finite-agent-runtime-2026-08-01.1";
    # N-1 expand fallback only for rows without RuntimeSpec. New leases
    # receive these same public values from Core's FC_CORE_RUNTIME_ENV_JSON
    # and ignore this Runner-side map entirely.
    FC_RUNNER_RUNTIME_ENV_JSON = builtins.toJSON {
      FINITE_SITES_API = "https://api.finite.chat";
      FINITE_BRAIN_SERVER_URL = "https://brain.finite.computer";
      FINITE_BRAIN_PUBLIC_BASE_URL = "https://brain.finite.computer";
    };
    FC_RUNNER_RUNTIME_SECRET_ENV_FILE = "/etc/finite/runtime-secrets.env";

    FC_RUNNER_KATA_NAMESPACE = "finite";
    FC_RUNNER_KATA_OCI_RUNTIME = "io.containerd.kata.v2";
    FC_RUNNER_KATA_NAME_PREFIX = "finite-kata";
    FC_RUNNER_KATA_CONTAINER_PORT = "8080";
    FC_RUNNER_KATA_CPUS = "4";
    FC_RUNNER_KATA_MEMORY = "8G";
    FC_RUNNER_KATA_PULL_POLICY = "missing";
    # Raised from the stock 30s: stopping a busy Kata sandbox regularly
    # exceeds 30s, and the short timeout caused false upgrade failures.
    FC_RUNNER_KATA_STOP_TIMEOUT_SECS = "180";
    # Runtime Retirement stays disabled until its dedicated, restricted Borg
    # namespace passes the gates in docs/runs/runtime-retirement-readiness.md.
    FC_RUNNER_KATA_RETIREMENT_ENABLED = "false";

    FC_RUNNER_FINITE_PRIVATE_BASE_URL = "https://kimi-k2-6.finite.containers.tinfoil.dev/v1";
    # The hostname is a historical compatibility route. DeepSeek is the
    # product model; glm-5-2 remains a server-side mixed-version alias only.
    FC_RUNNER_FINITE_PRIVATE_MODEL = "deepseek-v4-flash-0731";

    FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS = "300";
    FC_RUNNER_RUNTIME_READY_INTERVAL_MS = "2000";
    FC_RUNNER_LAUNCH_TIMEOUT_SECS = "900";
    FC_RUNNER_COMMAND_TIMEOUT_SECS = "30";
  };

  hostEnvironment = {
    FC_CORE_URL = cfg.coreUrl;
    FC_RUNNER_ID = cfg.runnerId;
    FC_RUNNER_SOURCE_HOST_ID = cfg.sourceHostId;
    FC_RUNNER_WORK_ROOT = cfg.workRoot;
    FC_RUNNER_MAX_SANDBOXES = toString cfg.maxSandboxes;
  }
  // lib.optionalAttrs (cfg.kataHostAddress != null) {
    FC_RUNNER_KATA_HOST_ADDRESS = cfg.kataHostAddress;
  };

  renderEnvironment =
    environment:
    lib.concatStringsSep "\n" (lib.mapAttrsToList (name: value: "${name}=${value}") environment);
in
{
  imports = [ ./finite-saas-runner.nix ];

  options.finite.kataRunnerHost = {
    coreUrl = lib.mkOption {
      type = lib.types.str;
      example = "http://10.254.3.1:14200";
      description = "Core URL this Runner leases from: loopback on the Core host, the private WireGuard proxy on a remote Runner.";
    };
    runnerId = lib.mkOption {
      type = lib.types.str;
      example = "finite-kata-runner-3";
      description = "Worker identity Core binds the route-scoped Runner credential to.";
    };
    sourceHostId = lib.mkOption {
      type = lib.types.str;
      example = "finite-lat-3";
      description = "Source host recorded for every Runtime this Runner launches; must match the NixOS host name.";
    };
    workRoot = lib.mkOption {
      type = lib.types.str;
      example = "/data/finite-saas-runner";
      description = "Durable agent-state root for this host's storage layout.";
    };
    kataHostAddress = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "10.254.3.2";
      description = "Address Core records for reaching launched sandboxes. Null keeps the runner's loopback default, which is correct only when Core and the Runner share a host.";
    };
    maxSandboxes = lib.mkOption {
      type = lib.types.ints.positive;
      example = 32;
      description = "Hard sandbox ceiling this host advertises to Core; sized to the host's real capacity.";
    };
  };

  config = {
    assertions = [
      {
        assertion = cfg.sourceHostId == config.networking.hostName;
        message = "the Kata Runner source host id must match the NixOS host name";
      }
    ];

    # Nix-owned, non-secret shape of the Runner role. EnvironmentFile order
    # matters: the operator-managed runner.env loads after this file, so its
    # credentials and bounded incident overrides still win.
    environment.etc."finite/runner-shared.env".text = ''
      # Rendered by infra/nixos/modules/kata-runner-host.nix. Do not edit on
      # the host: change the shared module (shared keys) or the host config
      # (per-host keys) and deploy. Credentials and drain state belong to
      # /etc/finite/runner.env, which systemd loads after this file.
      ${renderEnvironment sharedEnvironment}

      # Per-host values declared by the host config.
      ${renderEnvironment hostEnvironment}
    '';

    systemd.services.finite-saas-runner.serviceConfig.EnvironmentFile = lib.mkBefore [
      "/etc/finite/runner-shared.env"
    ];
  };
}
