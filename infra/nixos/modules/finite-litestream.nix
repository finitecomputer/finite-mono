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
# Why one replicator instance per database: the replicator must live strictly
# inside its owning service's lifecycle (PartOf=, see the first-opener note
# below). With a single shared process, restarting ANY owning service paused
# replication for EVERY enrolled database — a routine Brain deploy would gap
# chat replication. Per-db `finite-litestream-<name>.service` instances keep
# those lifecycles independent. `/etc/litestream.yml` still renders the
# combined view of all enrolled databases as the canonical config for on-host
# CLI work (`litestream restore` / `litestream ltx`); never run
# `litestream replicate` against it by hand — that would fight the per-db
# services.
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
  dbEntry = db: {
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
  };
  # Combined view of every enrolled database. Canonical for on-host CLI use
  # (restore/ltx) only; the replicate processes each run from their own
  # single-db config below.
  cliConfigFile = settingsFormat.generate "litestream.yml" {
    dbs = map dbEntry cfg.dbs;
  };
  replicateConfigFor = db:
    settingsFormat.generate "litestream-${db.name}.yml" {
      addr = db.metricsAddress;
      snapshot = {
        interval = "24h";
        retention = "168h";
      };
      dbs = [ (dbEntry db) ];
    };
  replicatorUnitFor = db: "finite-litestream-${db.name}";
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
        LITESTREAM_SECRET_ACCESS_KEY. If absent every per-db replicator unit
        is condition-skipped (the owning services keep serving) and
        finite-litestream-health fails loudly every five minutes until it
        exists.
      '';
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
        description = "S3-compatible endpoint URL (non-secret), e.g. https://objects.chi.storage.sh";
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
      description = "SQLite databases to replicate continuously, one replicator instance each.";
      type = lib.types.listOf (lib.types.submodule {
        options = {
          name = lib.mkOption {
            type = lib.types.str;
            description = ''
              Replica path inside the bucket, meta directory name, and the
              `finite-litestream-<name>` unit suffix. Renaming abandons the
              existing replica generation in the bucket — don't.
            '';
          };
          path = lib.mkOption {
            type = lib.types.path;
            description = "Absolute path of the live SQLite database file.";
          };
          owningService = lib.mkOption {
            type = lib.types.str;
            description = "systemd unit that owns and writes this database, e.g. finitechat-server.service.";
          };
          metricsAddress = lib.mkOption {
            type = lib.types.str;
            description = ''
              Loopback address for this instance's Prometheus /metrics.
              Must be unique per database (each instance binds its own).
            '';
          };
        };
      });
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion =
          lib.length (lib.unique (map (db: db.metricsAddress) cfg.dbs))
          == lib.length cfg.dbs;
        message = "finite.litestream: every dbs entry needs a unique metricsAddress.";
      }
      {
        assertion =
          lib.length (lib.unique (map (db: db.name) cfg.dbs)) == lib.length cfg.dbs;
        message = "finite.litestream: every dbs entry needs a unique name.";
      }
    ];

    # Canonical combined config so a bare `litestream restore`/`ltx` works
    # on-host across all enrolled databases.
    environment.etc."litestream.yml".source = cliConfigFile;
    environment.systemPackages = [ pkgs.litestream ];

    systemd.services = lib.mkMerge (
      [
        {
          # Backup-family health pattern: paired oneshot + short timer that
          # fails loudly into the journal; finite-status reads the success
          # stamp. This is deliberately NOT part of the contract-locked
          # finite-healthcheck curl loop, so a missing secret cannot fail
          # deploy-time aggregate health.
          finite-litestream-health = {
            description = "Fail if enrolled SQLite replication to object storage is missing, erroring, or stale";
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
            # Freshness = replication LAG, not write recency: a quiet database
            # legitimately produces no new LTX for hours (this false-alarmed
            # on 2026-08-13). Green when the newest replicated txid has caught
            # up to the local write position (`litestream_txid` metric,
            # decimal, vs the hex max_txid column of `litestream ltx` — both
            # verified against 0.5.11); only a replica that is BEHIND falls
            # through to the LTX-age bound, which then tolerates in-flight
            # syncs but flags a wedged one.
            script = ''
              set -euo pipefail
              env_file=${lib.escapeShellArg cfg.environmentFile}
              if [ ! -f "$env_file" ]; then
                echo "finite-litestream-health: $env_file is not installed; replication is NOT running" >&2
                exit 1
              fi
              grep -q '^LITESTREAM_ACCESS_KEY_ID=' "$env_file"
              grep -q '^LITESTREAM_SECRET_ACCESS_KEY=' "$env_file"
              now=$(date +%s)
              ${lib.concatStringsSep "\n" (map (db: ''
                systemctl is-active --quiet ${replicatorUnitFor db}.service
                metrics=$(curl -fsS --max-time 10 http://${db.metricsAddress}/metrics)
                printf '%s\n' "$metrics" | grep -q '^litestream_'
                ltx_rows=$(litestream ltx -config /etc/litestream.yml ${lib.escapeShellArg db.path} | tail -n +2)
                if [ -z "$ltx_rows" ]; then
                  echo "finite-litestream-health: no replicated LTX entries for ${db.name}" >&2
                  exit 1
                fi
                replica_txid=$(( 16#$(printf '%s\n' "$ltx_rows" | awk '{print $3}' | sort | tail -1) ))
                db_txid=$(printf '%s\n' "$metrics" \
                  | awk -v needle='db="${db.path}"' \
                      'index($0, "litestream_txid{") == 1 && index($0, needle) > 0 {printf "%.0f", $2}')
                if [ -z "$db_txid" ]; then
                  # Metric label sets can vary by version; without the local
                  # position, fall back to the age bound alone.
                  db_txid=-1
                fi
                if [ "$db_txid" -ge 0 ] && [ "$replica_txid" -ge "$db_txid" ]; then
                  : # fully replicated; quiet databases stay green
                else
                  newest=$(printf '%s\n' "$ltx_rows" | awk '{print $NF}' | sort | tail -1)
                  newest_epoch=$(date -d "$newest" +%s)
                  age=$(( now - newest_epoch ))
                  if [ "$age" -gt ${toString cfg.maximumReplicaAgeSeconds} ]; then
                    echo "finite-litestream-health: ${db.name} replica is behind (db txid $db_txid, replica txid $replica_txid) and stale ($age seconds; newest LTX $newest)" >&2
                    exit 1
                  fi
                fi
              '') cfg.dbs)}
              date +%s > /var/lib/finite-litestream/health-last-success
            '';
          };
        }
      ]
      ++ map (db: {
        ${replicatorUnitFor db} = {
          description = "Litestream replication for ${db.name} (DR-only, not a standby)";
          wants = [ "network-online.target" ];
          after = [ "network-online.target" db.owningService ];
          requires = [ db.owningService ];
          partOf = [ db.owningService ];
          wantedBy = [ "multi-user.target" ];
          # Loud-but-safe secret degrade: no crash-loop when the operator
          # has not installed the credential file yet; the health timer
          # reports it.
          unitConfig.ConditionPathExists = cfg.environmentFile;
          path = [ pkgs.coreutils ];
          preStart = firstOpenerGuard db;
          serviceConfig = {
            ExecStart = "${pkgs.litestream}/bin/litestream replicate -config ${replicateConfigFor db}";
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
            ReadWritePaths = [ (dirOf db.path) ];
          };
        };
      }) cfg.dbs
      # The owning service's start pulls its replicator back after the
      # pre-deploy snapshot fence stops everything and restarts services one
      # by one.
      ++ map (db: {
        ${lib.removeSuffix ".service" db.owningService}.wants =
          [ "${replicatorUnitFor db}.service" ];
      }) cfg.dbs
    );

    systemd.timers.finite-litestream-health = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        # OnActiveSec, not OnBootSec: on a long-up host a freshly started
        # timer with OnBootSec in the past fires DURING the activation that
        # introduces it, before the initial snapshot upload has produced any
        # LTX entries — failing the oneshot and making switch-to-configuration
        # report status 4 (the #466 monitoring-unit deploy-noise class). The
        # first check lands 20 minutes after (re)activation instead.
        OnActiveSec = "20min";
        OnUnitActiveSec = "5min";
      };
    };
  };
}
