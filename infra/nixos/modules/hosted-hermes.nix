# Opt-in native Hermes ingress for configured Runner hosts.
{
  config,
  lib,
  pkgs,
  finitePackages,
  ...
}:
let
  cfg = config.finite.hostedHermes;
  deployment = pkgs.writeText "hosted-hermes-deployment.json" (
    builtins.toJSON {
      public_origin = cfg.publicOrigin;
      listen = "${cfg.listenAddress}:${toString cfg.listenPort}";
      allowed_origins = cfg.allowedOrigins;
    }
  );
  runnerGate = pkgs.writeShellApplication {
    name = "hosted-hermes-runner-gate";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gnugrep
      pkgs.systemd
    ];
    text = builtins.readFile ./hosted-hermes-runner-gate.sh;
  };
in
{
  options.finite.hostedHermes = {
    enable = lib.mkEnableOption "Runner-managed hosted Hermes ingress";
    publicOrigin = lib.mkOption {
      type = lib.types.str;
      default = "";
      description = "Exact HTTPS origin also configured for this source host in Core.";
    };
    runtimeCoreUrl = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "HTTPS origin of Core's dedicated runtime router; enables scoped bootstrap delivery on launch/upgrade.";
    };
    listenAddress = lib.mkOption {
      type = lib.types.str;
      default = "0.0.0.0";
      description = "Dedicated Hermes listener IP (bracket IPv6 addresses).";
    };
    listenPort = lib.mkOption {
      type = lib.types.port;
      default = 443;
      description = "Dedicated TLS port, opened in the host TCP firewall.";
    };
    allowedOrigins = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "Exact dashboard browser origins allowed by the native API edge.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.publicOrigin != "" && config.finite.saasRunner;
        message = "Hosted Hermes requires a Runner host and a configured public origin.";
      }
    ];
    finite.metrics.journalLogUnits = [ "finite-hosted-hermes.service" ];
    networking.firewall.allowedTCPPorts = [ cfg.listenPort ];
    systemd.tmpfiles.rules = [ "d /run/finite-hosted-hermes 0700 root root -" ];
    systemd.services.finite-saas-runner = {
      environment = {
        FC_RUNNER_HOSTED_HERMES_CONFIG = toString deployment;
      }
      // lib.optionalAttrs (cfg.runtimeCoreUrl != null) {
        FC_RUNNER_RUNTIME_CORE_URL = cfg.runtimeCoreUrl;
      };
      serviceConfig = {
        # A successful main-process exit must not release the timer while a
        # nerdctl/CNI child can still mutate saved port ownership. Containerd,
        # its shims and agent VMs belong to separate services/cgroups.
        ExecStartPre = [
          "${runnerGate}/bin/hosted-hermes-runner-gate ${finitePackages.finite-saas-runner}/bin/finite-saas-runner"
        ];
        Type = lib.mkForce "exec";
        ExitType = "cgroup";
        KillMode = lib.mkForce "control-group";
        SendSIGKILL = true;
        Delegate = false;
        # A retirement uploads a whole home (up to about 120 GB) to
        # rsync.net and reads it back, all in one run: about 4 hours at a
        # clean day's ~20 MB/s. 12 hours still fits it at about 6 MB/s, and
        # an upload cut short resumes from borg's last checkpoint.
        # The cost, accepted: while one long retirement runs, this host's
        # other Runner work waits for it. Staff retire big homes one at a
        # time, at quiet hours.
        RuntimeMaxSec = "12h";
        # Borg commits a checkpoint on SIGINT, never on SIGTERM, and starts
        # its ssh ignoring SIGINT so the checkpoint can still reach the
        # repository. The runner, nerdctl and ctr exit on SIGINT just as on
        # SIGTERM. After its checkpoint borg goes on uploading, so a stop
        # mid-upload runs to this timeout and ends in SIGKILL; 60 s lets the
        # checkpoint commit even on a slow link. If it doesn't, the last
        # periodic checkpoint (every 5 minutes) still stands.
        KillSignal = "SIGINT";
        TimeoutStopSec = "60s";
      };
    };
    systemd.services.finite-hosted-hermes = {
      description = "Runner-managed native Hermes ingress";
      after = [ "network-online.target" ];
      # Intentionally no wantedBy, socket activation, reload, restart or resume:
      # only a fresh Runner authority projection may start this process.
      serviceConfig = {
        Type = "notify";
        ExecStart = "${pkgs.caddy}/bin/caddy run --config /run/finite-hosted-hermes/caddy.json";
        Restart = "no";
        KillMode = "control-group";
        SendSIGKILL = true;
        TimeoutStartSec = "10s";
        TimeoutStopSec = "5s";
        User = "root";
        Group = "root";
        UMask = "0077";
        StateDirectory = "finite-hosted-hermes";
        Environment = [
          "HOME=/var/lib/finite-hosted-hermes"
          "XDG_DATA_HOME=/var/lib/finite-hosted-hermes/data"
          "XDG_CONFIG_HOME=/var/lib/finite-hosted-hermes/config"
        ];
      };
    };
  };
}
