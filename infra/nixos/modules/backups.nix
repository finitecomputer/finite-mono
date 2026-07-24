{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.finite.recoveryBackup;
  snapshotRoot = "/data/recovery-snapshots/hosted-web-chat";
  identityBackupRoot = "/data/backups/identity";
  # Reuse the established finitecomputer Borg credential layout verbatim.
  # Values are copied host-to-host/off-host and never enter this public repo.
  borgStateRoot = "/var/lib/finitecomputer/backups";
  borgSecretRoot = "${borgStateRoot}/rsync-net";
in
{
  options.finite.recoveryBackup = {
    borgRepository = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "account@account.rsync.net:finitecomputer/finite-lat-1";
      description = "Off-host Borg repository used by a no-prune production job; destination-enforced append-only access is recommended. Null keeps archival disabled.";
    };
    borgRemotePath = lib.mkOption {
      type = lib.types.str;
      default = "borg12";
      description = "Remote Borg executable selected by the backup destination.";
    };
  };

  config = {
    systemd.services.finite-hosted-web-chat-snapshot = {
      description = "Service-consistent hosted Recovery Snapshot";
      after = [ "postgresql.service" ];
      requires = [ "postgresql.service" ];
      path = [
        config.services.postgresql.package
        pkgs.coreutils
        pkgs.findutils
        pkgs.gnused
        pkgs.sqlite
        pkgs.systemd
        pkgs.util-linux
      ];
      serviceConfig = {
        Type = "oneshot";
        User = "root";
        UMask = "0077";
      };
      script = ''
        set -euo pipefail
        root=${snapshotRoot}
        stamp=$(date -u +%Y%m%dT%H%M%SZ)
        staging="$root/.staging-$stamp"
        final="$root/$stamp"
        hosted_was_active=0
        chat_was_active=0
        core_was_active=0
        brain_was_active=0
        identity_was_active=0
        kata_was_active=0
        kata_timer_was_active=0
        phala_was_active=0

        cleanup() {
          cleanup_status=0
          rm -rf "$staging" || cleanup_status=1
          if [ "$chat_was_active" = 1 ]; then systemctl start finitechat-server.service || cleanup_status=1; fi
          if [ "$identity_was_active" = 1 ]; then systemctl start finite-identity.service || cleanup_status=1; fi
          if [ "$core_was_active" = 1 ]; then systemctl start finite-saas-core.service || cleanup_status=1; fi
          if [ "$brain_was_active" = 1 ]; then systemctl start finite-brain-app.service || cleanup_status=1; fi
          if [ "$hosted_was_active" = 1 ]; then systemctl start finitechat-hosted-device.service || cleanup_status=1; fi
          if [ "$kata_was_active" = 1 ]; then systemctl start finite-saas-runner.service || cleanup_status=1; fi
          if [ "$phala_was_active" = 1 ]; then systemctl start finite-saas-runner-phala.service || cleanup_status=1; fi
          if [ "$kata_timer_was_active" = 1 ]; then systemctl start finite-saas-runner.timer || cleanup_status=1; fi
          return "$cleanup_status"
        }
        trap 'cleanup || true' EXIT

        install -d -m 0700 \
          "$root" \
          "$staging/hosted-device" \
          "$staging/finite-chat" \
          "$staging/saas-core" \
          "$staging/finite-brain" \
          "$staging/finite-identity"
        systemctl is-active --quiet finitechat-hosted-device.service && hosted_was_active=1 || true
        systemctl is-active --quiet finitechat-server.service && chat_was_active=1 || true
        systemctl is-active --quiet finite-saas-core.service && core_was_active=1 || true
        systemctl is-active --quiet finite-brain-app.service && brain_was_active=1 || true
        systemctl is-active --quiet finite-identity.service && identity_was_active=1 || true
        systemctl is-active --quiet finite-saas-runner.service && kata_was_active=1 || true
        systemctl is-active --quiet finite-saas-runner.timer && kata_timer_was_active=1 || true
        systemctl is-active --quiet finite-saas-runner-phala.service && phala_was_active=1 || true

        if [ "$kata_timer_was_active" = 1 ]; then systemctl stop finite-saas-runner.timer; fi
        if [ "$kata_was_active" = 1 ]; then systemctl stop finite-saas-runner.service; fi
        if [ "$phala_was_active" = 1 ]; then systemctl stop finite-saas-runner-phala.service; fi
        if [ "$core_was_active" = 1 ]; then systemctl stop finite-saas-core.service; fi
        if [ "$brain_was_active" = 1 ]; then systemctl stop finite-brain-app.service; fi
        if [ "$hosted_was_active" = 1 ]; then systemctl stop finitechat-hosted-device.service; fi
        if [ "$identity_was_active" = 1 ]; then systemctl stop finite-identity.service; fi
        if [ "$chat_was_active" = 1 ]; then systemctl stop finitechat-server.service; fi

        # The brief write fence makes account, Principal, product, and encrypted
        # binding state one composition. SQLite databases are copied through
        # SQLite's online backup API; live db/WAL files are never artifacts.
        cp -a /var/lib/private/finitechat-hosted-device/. "$staging/hosted-device/"
        find "$staging/hosted-device" -type f \( -name 'client.sqlite3' -o -name 'client.sqlite3-wal' -o -name 'client.sqlite3-shm' \) -delete
        while IFS= read -r source; do
          relative="''${source#/var/lib/private/finitechat-hosted-device/}"
          destination="$staging/hosted-device/$relative"
          install -d -m 0700 "$(dirname "$destination")"
          sqlite3 "$source" ".backup '$destination'"
          test "$(sqlite3 "$destination" 'PRAGMA integrity_check;')" = ok
        done < <(find /var/lib/private/finitechat-hosted-device -type f -name client.sqlite3 -print)

        sqlite3 /var/lib/private/finite-chat/data/server.sqlite3 ".backup '$staging/finite-chat/server.sqlite3'"
        test "$(sqlite3 "$staging/finite-chat/server.sqlite3" 'PRAGMA integrity_check;')" = ok
        sqlite3 /var/lib/private/finitebrain/finite-brain.sqlite3 ".backup '$staging/finite-brain/finite-brain.sqlite3'"
        test "$(sqlite3 "$staging/finite-brain/finite-brain.sqlite3" 'PRAGMA integrity_check;')" = ok
        sqlite3 /var/lib/finite-identity/identity.db ".backup '$staging/finite-identity/identity.db'"
        test "$(sqlite3 "$staging/finite-identity/identity.db" 'PRAGMA integrity_check;')" = ok
        runuser -u postgres -- pg_dump --format=custom finite_core > "$staging/saas-core/finite_core.dump"
        pg_restore --list "$staging/saas-core/finite_core.dump" >/dev/null

        # #178 upgrades this format to a versioned, exact Recovery Set manifest
        # and supplies the corresponding empty-target restore contract.
        printf '%s\n' 'finite.hosted-web-chat-recovery-snapshot.v1' > "$staging/format"
        (
          cd "$staging"
          find format hosted-device finite-chat saas-core finite-brain finite-identity -type f -print0 \
            | sort -z \
            | xargs -0 sha256sum > manifest.sha256
        )
        mv "$staging" "$final"
        ln -sfn "$stamp" "$root/latest"
        find "$root" -mindepth 1 -maxdepth 1 -type d -name '20*T*Z' -mtime +2 -exec rm -rf -- {} +
        trap - EXIT
        cleanup
      '';
    };

    # 2026-07-14 (Paul): no calendar timer. The snapshot stops/starts services
    # for its write fence, and running that every 15 minutes broke live chat
    # streams and cold-restarted every hosted device runtime. The snapshot
    # service runs when a deploy triggers it or when started manually.
    systemd.services.finite-hosted-web-chat-snapshot-health = {
      description = "Fail if the hosted Recovery Snapshot is older than 7 days or corrupt";
      path = [ pkgs.coreutils ];
      serviceConfig = {
        Type = "oneshot";
        User = "root";
      };
      script = ''
        set -euo pipefail
        latest=${snapshotRoot}/latest
        test -L "$latest"
        age=$(( $(date +%s) - $(stat -Lc %Y "$latest") ))
        if [ "$age" -gt 604800 ]; then
          echo "Hosted Recovery Snapshot is stale ($age seconds); deploy or run finite-hosted-web-chat-snapshot.service" >&2
          exit 1
        fi
        (cd "$latest" && sha256sum --check manifest.sha256)
      '';
    };

    systemd.timers.finite-hosted-web-chat-snapshot-health = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnBootSec = "20min";
        OnUnitActiveSec = "5min";
      };
    };

    # Identity is small SQLite state and can use its online backup API without
    # stopping the service. This gives immutable bindings an ordinary recovery
    # point between deploy-triggered coordinated snapshots.
    systemd.services.finite-identity-backup = {
      description = "SQLite-safe Finite Identity backup";
      after = [ "finite-identity.service" ];
      requires = [ "finite-identity.service" ];
      path = [
        pkgs.coreutils
        pkgs.findutils
        pkgs.sqlite
      ];
      serviceConfig = {
        Type = "oneshot";
        User = "root";
        UMask = "0077";
      };
      script = ''
        set -euo pipefail
        source=/var/lib/finite-identity/identity.db
        root=${identityBackupRoot}
        test -f "$source"
        stamp=$(date -u +%Y%m%dT%H%M%SZ)
        name="identity-$stamp.db"
        staging="$root/.$name.tmp"
        destination="$root/$name"
        cleanup() {
          rm -f "$staging"
        }
        trap cleanup EXIT
        sqlite3 "$source" ".backup '$staging'"
        test "$(sqlite3 "$staging" 'PRAGMA integrity_check;')" = ok
        mv "$staging" "$destination"
        (
          cd "$root"
          sha256sum "$name" > "$name.sha256"
        )
        find "$root" -type f -name 'identity-20*T*Z.db*' -mtime +14 -delete
        trap - EXIT
      '';
    };

    systemd.timers.finite-identity-backup = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnBootSec = "10min";
        OnCalendar = "*-*-* 00/6:37:00";
        Persistent = true;
        Unit = "finite-identity-backup.service";
      };
    };

    systemd.services.finite-identity-backup-health = {
      description = "Fail if the Finite Identity backup is older than 7 hours or corrupt";
      path = [
        pkgs.coreutils
        pkgs.findutils
      ];
      serviceConfig = {
        Type = "oneshot";
        User = "root";
      };
      script = ''
        set -euo pipefail
        root=${identityBackupRoot}
        latest=$(find "$root" -maxdepth 1 -type f -name 'identity-20*T*Z.db' -print | sort | tail -1)
        test -n "$latest"
        age=$(( $(date +%s) - $(stat -c %Y "$latest") ))
        if [ "$age" -gt 25200 ]; then
          echo "Finite Identity backup is stale ($age seconds)" >&2
          exit 1
        fi
        (
          cd "$root"
          sha256sum --check "$(basename "$latest").sha256"
        )
      '';
    };

    systemd.timers.finite-identity-backup-health = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnBootSec = "20min";
        OnUnitActiveSec = "5min";
      };
    };

    services.borgbackup.jobs."finite-hosted-web-chat-offsite" = lib.mkIf (cfg.borgRepository != null) {
      paths = [
        snapshotRoot
        identityBackupRoot
      ];
      repo = cfg.borgRepository;
      archiveBaseName = "finite-lat-1-hosted-web-chat";
      encryption = {
        mode = "repokey-blake2";
        passCommand = "cat ${borgSecretRoot}/borg-passphrase";
      };
      environment.BORG_RSH = "ssh -i ${borgSecretRoot}/id_ed25519 -o BatchMode=yes -o UserKnownHostsFile=${borgSecretRoot}/known_hosts -o StrictHostKeyChecking=yes";
      extraArgs = [ "--remote-path=${cfg.borgRemotePath}" ];
      compression = "auto,zstd";
      failOnWarnings = true;
      persistentTimer = true;
      # Snapshots are deploy-triggered now; ship the latest one off-host
      # daily (Borg dedup makes re-shipping an unchanged snapshot cheap).
      startAt = "*-*-* 03:07:00";
      readWritePaths = [ borgStateRoot ];
      preHook = ''
        latest=${snapshotRoot}/latest
        test -L "$latest"
        (cd "$latest" && sha256sum --check manifest.sha256)
        identity_latest=$(
          ${pkgs.findutils}/bin/find ${identityBackupRoot} -maxdepth 1 -type f -name 'identity-20*T*Z.db' -print \
            | ${pkgs.coreutils}/bin/sort \
            | ${pkgs.coreutils}/bin/tail -1
        )
        test -n "$identity_latest"
        (
          cd ${identityBackupRoot}
          ${pkgs.coreutils}/bin/sha256sum --check "$(${pkgs.coreutils}/bin/basename "$identity_latest").sha256"
        )
      '';
      postCreate = ''
        date +%s > ${borgStateRoot}/hosted-web-chat-last-success
      '';
    };

    systemd.services.finite-hosted-web-chat-offsite-health = lib.mkIf (cfg.borgRepository != null) {
      description = "Fail if the hosted offsite archive is older than 50 hours";
      path = [ pkgs.coreutils ];
      serviceConfig = {
        Type = "oneshot";
        User = "root";
      };
      script = ''
        set -euo pipefail
        stamp=${borgStateRoot}/hosted-web-chat-last-success
        test -s "$stamp"
        last_success=$(cat "$stamp")
        age=$(( $(date +%s) - last_success ))
        if [ "$age" -gt 180000 ]; then
          echo "Hosted offsite archive is stale ($age seconds)" >&2
          exit 1
        fi
      '';
    };

    systemd.timers.finite-hosted-web-chat-offsite-health = lib.mkIf (cfg.borgRepository != null) {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnBootSec = "20min";
        OnUnitActiveSec = "5min";
      };
    };

    systemd.tmpfiles.rules = [
      "d /data/recovery-snapshots 0700 root root -"
      "d ${snapshotRoot} 0700 root root -"
      "d /data/backups 0755 root root -"
      "d ${identityBackupRoot} 0700 root root -"
      "d ${borgStateRoot} 0700 root root -"
      "d ${borgSecretRoot} 0700 root root -"
    ];
  };
}
