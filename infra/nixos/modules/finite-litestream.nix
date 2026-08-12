# finite-litestream — continuous SQLite replication to S3-compatible object
# storage (Latitude.sh). DR-only restore lane: seconds-of-writes RPO with a
# documented manual `litestream restore`, NOT a warm standby. A restored copy
# must never run alongside the live server (single-writer doctrine,
# infra/runbooks/deploy-finitechat-server.md).
#
# Why this exists: the deploy-triggered Recovery Snapshot is stop-the-world
# (its 15-minute timer was removed 2026-07-14 because the fence broke live
# chat streams), so between deploys the only recovery points were nightly
# Borg archives. Litestream closes that gap by shipping WAL frames
# continuously. It is additive: the snapshot and Borg lanes stay.
#
# Why root: the first consumer is finitechat-server's SQLite under
# /var/lib/private/finite-chat (DynamicUser; parent is root 0700). A fixed
# litestream user cannot traverse that path — which is why the upstream
# nixpkgs services.litestream module is unsuitable — and migrating the chat
# server to a static user is a riskier change than running the replicator as
# root, which is already house precedent for backup units (backups.nix).
#
# Why the ExecStartPre guard: root must never be the FIRST opener of a
# DynamicUser service's database. A root-created WAL/SHM broke the
# 2026-08-11 chat deploy (see the PR #477 rollout notes). The guard refuses
# to start until the owning service has the DB and its WAL open under a
# non-root uid, and per-db meta state lives under /var/lib/finite-litestream
# so nothing foreign is ever created inside the owning StateDirectory.
#
# Replication cost knob: sync-interval 1s ≈ 86k bucket PUTs/day per active
# db. Raising it to 5-10s still meets the seconds-RPO bar if request pricing
# ever warrants.
{ config, lib, pkgs, ... }:
let
  cfg = config.finite.litestream;
  settingsFormat = pkgs.formats.yaml { };
  configFile = settingsFormat.generate "litestream.yml" {
    addr = cfg.metricsAddress;
    snapshot = {
      interval = "24h";
      retention = "168h";
    };
    dbs = map (db: {
      path = db.path;
      meta-path = "/var/lib/finite-litestream/${db.name}";
      replica = {
        type = "s3";
        bucket = cfg.replica.bucket;
        path = db.name;
        endpoint = cfg.replica.endpoint;
        region = cfg.replica.region;
        force-path-style = true;
        sync-interval = "1s";
      };
    }) cfg.dbs;
  };
  owningServices = lib.unique (map (db: db.owningService) cfg.dbs);
  firstOpenerGuard = db: ''
    db=${lib.escapeShellArg db.path}
    if [ ! -f "$db" ]; then
      echo "finite-litestream: $db does not exist; start ${db.owningService} first" >&2
      exit 1
    fi
    db_owner=$(stat -c %u "$db")
    if [ "$db_owner" = 0 ]; then
      echo "finite-litestream: $db is root-owned; refusing (litestream must never be the first opener)" >&2
      exit 1
    fi
    if [ ! -f "$db-wal" ]; then
      echo "finite-litestream: $db-wal missing; start ${db.owningService} so the owning service opens the database first" >&2
      exit 1
    fi
    wal_owner=$(stat -c %u "$db-wal")
    if [ "$wal_owner" != "$db_owner" ]; then
      echo "finite-litestream: $db-wal owner $wal_owner != db owner $db_owner; refusing" >&2
      exit 1
    fi
  '';
in
{
  options.finite.litestream = {
    enable = lib.mkEnableOption "continuous SQLite replication to object storage";

    environmentFile = lib.mkOption {
      type = lib.types.str;
      default = "/etc/finite/litestream-latitude.env";
      description = ''
        Operator-placed env file carrying LITESTREAM_ACCESS_KEY_ID and
        LITESTREAM_SECRET_ACCESS_KEY. If absent the replicator unit is
        condition-skipped (chat is unaffected) and finite-litestream-health
        fails loudly every five minutes until it exists.
      '';
    };

    metricsAddress = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:9351";
      description = "Loopback address for litestream's Prometheus /metrics.";
    };

    maximumReplicaAgeSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 900;
      description = ''
        finite-litestream-health fails when the newest replicated LTX entry
        for any enrolled db is older than this bound.
      '';
    };

    replica = {
      endpoint = lib.mkOption {
        type = lib.types.str;
        description = "S3-compatible endpoint URL (non-secret), e.g. https://objects.nyc.storage.sh";
      };
      bucket = lib.mkOption {
        type = lib.types.str;
        description = "Bucket name (non-secret).";
      };
      region = lib.mkOption {
        type = lib.types.str;
        default = "us-east-1";
        description = "Region string for SigV4 scope; many S3-compatible providers accept any value.";
      };
    };

    dbs = lib.mkOption {
      default = [ ];
      description = "SQLite databases to replicate continuously.";
      type = lib.types.listOf (lib.types.submodule {
        options = {
          name = lib.mkOption {
            type = lib.types.str;
            description = "Replica path inside the bucket and meta directory name.";
          };
          path = lib.mkOption {
            type = lib.types.path;
            description = "Absolute path of the live SQLite database file.";
          };
          owningService = lib.mkOption {
            type = lib.types.str;
            description = "systemd unit that owns and writes this database, e.g. finitechat-server.service.";
          };
        };
      });
    };
  };

  config = lib.mkIf cfg.enable {
    # Canonical config path so a bare `litestream restore` works on-host.
    environment.etc."litestream.yml".source = configFile;
    environment.systemPackages = [ pkgs.litestream ];

    systemd.services = lib.mkMerge (
      [
        {
          finite-litestream = {
            description = "Litestream continuous SQLite replication (DR-only, not a standby)";
            wants = [ "network-online.target" ];
            after = [ "network-online.target" ] ++ owningServices;
            requires = owningServices;
            partOf = owningServices;
            wantedBy = [ "multi-user.target" ];
            restartTriggers = [ configFile ];
            # Loud-but-safe secret degrade: no crash-loop when the operator
            # has not installed the credential file yet; the health timer
            # reports it.
            unitConfig.ConditionPathExists = cfg.environmentFile;
            path = [ pkgs.coreutils ];
            preStart = lib.concatStringsSep "\n" (map firstOpenerGuard cfg.dbs);
            serviceConfig = {
              ExecStart = "${pkgs.litestream}/bin/litestream replicate -config /etc/litestream.yml";
              EnvironmentFile = cfg.environmentFile;
              User = "root";
              StateDirectory = "finite-litestream";
              Restart = "on-failure";
              RestartSec = 5;
              NoNewPrivileges = true;
              PrivateTmp = true;
              ProtectSystem = "strict";
              ProtectHome = true;
              ProtectKernelTunables = true;
              ProtectControlGroups = true;
              RestrictSUIDSGID = true;
              ReadWritePaths = lib.unique (map (db: dirOf db.path) cfg.dbs);
            };
          };

          # Backup-family health pattern: paired oneshot + short timer that
          # fails loudly into the journal; finite-status reads the success
          # stamp. This is deliberately NOT part of the contract-locked
          # finite-healthcheck curl loop, so a missing secret cannot fail
          # deploy-time aggregate health.
          finite-litestream-health = {
            description = "Fail if chat replication to object storage is missing, erroring, or stale";
            path = [
              pkgs.coreutils
              pkgs.curl
              pkgs.gawk
              pkgs.gnugrep
              pkgs.litestream
              pkgs.systemd
            ];
            serviceConfig = {
              Type = "oneshot";
              User = "root";
              StateDirectory = "finite-litestream";
              # Optional-load ('-') so a missing secret reaches the script's
              # explicit check and produces a clear journal message instead
              # of a unit load-time error.
              EnvironmentFile = "-${cfg.environmentFile}";
            };
            # `litestream ltx` column format verified against 0.5.11
            # (nixpkgs-lat3 pin): header row, then
            # `level min_txid max_txid size created` with an ISO-8601 UTC
            # timestamp in the last column.
            script = ''
              set -euo pipefail
              env_file=${lib.escapeShellArg cfg.environmentFile}
              if [ ! -f "$env_file" ]; then
                echo "finite-litestream-health: $env_file is not installed; replication is NOT running" >&2
                exit 1
              fi
              grep -q '^LITESTREAM_ACCESS_KEY_ID=' "$env_file"
              grep -q '^LITESTREAM_SECRET_ACCESS_KEY=' "$env_file"
              systemctl is-active --quiet finite-litestream.service
              curl -fsS --max-time 10 http://${cfg.metricsAddress}/metrics | grep -q '^litestream_'
              now=$(date +%s)
              ${lib.concatStringsSep "\n" (map (db: ''
                newest=$(litestream ltx -config /etc/litestream.yml ${lib.escapeShellArg db.path} \
                  | tail -n +2 | awk '{print $NF}' | sort | tail -1)
                if [ -z "$newest" ]; then
                  echo "finite-litestream-health: no replicated LTX entries for ${db.name}" >&2
                  exit 1
                fi
                newest_epoch=$(date -d "$newest" +%s)
                age=$(( now - newest_epoch ))
                if [ "$age" -gt ${toString cfg.maximumReplicaAgeSeconds} ]; then
                  echo "finite-litestream-health: ${db.name} replica is stale ($age seconds; newest LTX $newest)" >&2
                  exit 1
                fi
              '') cfg.dbs)}
              date +%s > /var/lib/finite-litestream/health-last-success
            '';
          };
        }
      ]
      # The owning service's start pulls replication back after the
      # pre-deploy snapshot fence stops everything and restarts services one
      # by one.
      ++ map (unit: {
        ${lib.removeSuffix ".service" unit}.wants = [ "finite-litestream.service" ];
      }) owningServices
    );

    systemd.timers.finite-litestream-health = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnBootSec = "20min";
        OnUnitActiveSec = "5min";
      };
    };
  };
}
