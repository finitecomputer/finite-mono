{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.finite.sitesV2Backup;
  sites = config.finite.sites;
in
{
  options.finite.sitesV2Backup = {
    enable = lib.mkEnableOption "Finite Sites v2 local service-consistent backups";
    snapshotRoot = lib.mkOption {
      type = lib.types.str;
      default = "/var/backups/finite-sites-v2";
      description = "Local root for timestamped Finite Sites v2 snapshots.";
    };
    keep = lib.mkOption {
      type = lib.types.ints.positive;
      default = 14;
      description = "Number of timestamped local snapshots to retain.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.finite-sites-v2-snapshot = {
      description = "Service-consistent Finite Sites v2 snapshot";
      path = [
        pkgs.coreutils
        pkgs.findutils
        pkgs.sqlite
        pkgs.systemd
      ];
      serviceConfig = {
        Type = "oneshot";
        User = "root";
        UMask = "0077";
      };
      script = ''
        set -euo pipefail

        root=${cfg.snapshotRoot}
        data=${sites.dataDir}
        keep=${toString cfg.keep}
        stamp=$(date -u +%Y%m%dT%H%M%SZ)
        staging="$root/.staging-$stamp"
        final="$root/$stamp"
        sites_was_active=0

        cleanup() {
          status=0
          if [ -e "$staging" ]; then
            chmod -R u+w -- "$staging" || status=1
            rm -rf -- "$staging" || status=1
          fi
          if [ "$sites_was_active" = 1 ]; then
            systemctl start finite-saas-sites.service || status=1
          fi
          return "$status"
        }
        trap 'cleanup || true' EXIT

        install -d -m 0700 "$root" "$staging/finite-sites"
        systemctl is-active --quiet finite-saas-sites.service && sites_was_active=1 || true
        if [ "$sites_was_active" = 1 ]; then
          systemctl stop finite-saas-sites.service
        fi

        if [ -d "$data" ]; then
          cp -a "$data"/. "$staging/finite-sites"/
        fi
        rm -f \
          "$staging/finite-sites/registry.db" \
          "$staging/finite-sites/registry.db-wal" \
          "$staging/finite-sites/registry.db-shm"
        if [ -f "$data/registry.db" ]; then
          sqlite3 "$data/registry.db" ".backup '$staging/finite-sites/registry.db'"
          test "$(sqlite3 "$staging/finite-sites/registry.db" 'PRAGMA integrity_check;')" = ok
        fi

        printf 'finite-sites-v2\t%s\n' "$stamp" > "$staging/manifest.tsv"
        mv "$staging" "$final"
        if [ "$sites_was_active" = 1 ]; then
          systemctl start finite-saas-sites.service
          sites_was_active=0
        fi
        trap - EXIT

        mapfile -t snapshots < <(find "$root" -mindepth 1 -maxdepth 1 -type d -name '20*' | sort)
        remove_count=$((''${#snapshots[@]} - keep))
        if [ "$remove_count" -gt 0 ]; then
          for ((i = 0; i < remove_count; i++)); do
            rm -rf -- "''${snapshots[$i]}"
          done
        fi
      '';
    };

    systemd.timers.finite-sites-v2-snapshot = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnCalendar = "hourly";
        Persistent = true;
        RandomizedDelaySec = "5m";
      };
    };

    systemd.services.finite-sites-v2-restore-check = {
      description = "Verify the latest Finite Sites v2 snapshot";
      path = [
        pkgs.coreutils
        pkgs.findutils
        pkgs.sqlite
      ];
      serviceConfig = {
        Type = "oneshot";
        User = "root";
      };
      script = ''
        set -euo pipefail

        latest=$(find ${cfg.snapshotRoot} -mindepth 1 -maxdepth 1 -type d -name '20*' | sort | tail -n 1)
        test -n "$latest"
        test -f "$latest/manifest.tsv"
        if [ -f "$latest/finite-sites/registry.db" ]; then
          test "$(sqlite3 "$latest/finite-sites/registry.db" 'PRAGMA integrity_check;')" = ok
        fi
      '';
    };

    systemd.timers.finite-sites-v2-restore-check = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnCalendar = "daily";
        Persistent = true;
        RandomizedDelaySec = "15m";
      };
    };
  };
}
