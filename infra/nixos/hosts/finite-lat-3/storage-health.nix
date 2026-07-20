{ pkgs, ... }:
let
  ids = import ./storage-ids.nix;

  storageHealth = pkgs.writeShellApplication {
    name = "finite-storage-health";
    runtimeInputs = with pkgs; [
      coreutils
      e2fsprogs
      gawk
      gnugrep
      gnused
      mdadm
      procps
      smartmontools
      util-linux
    ];
    text = ''
      fail() {
        echo "finite storage health: $*" >&2
        exit 1
      }

      check_array() {
        name="$1"
        expected_component_sectors="$2"
        device="/dev/md/$name"
        [[ -b "$device" ]] || fail "$device is missing"
        block="$(basename "$(readlink -f "$device")")"
        sys="/sys/block/$block/md"
        [[ "$(<"$sys/degraded")" == 0 ]] || fail "$device is degraded"
        [[ "$(<"$sys/sync_action")" == idle ]] || fail "$device is synchronizing"
        [[ "$(<"$sys/metadata_version")" == 1.2 ]] || fail "$device metadata is not 1.2"
        [[ "$(<"$sys/component_size")" == "$expected_component_sectors" ]] || fail "$device component size drifted"
        [[ "$(<"$sys/bitmap/location")" != none ]] || fail "$device has no write-intent bitmap"
        [[ "$(<"$sys/mismatch_cnt")" == 0 ]] || fail "$device mismatch count is nonzero"
        mdadm --detail --test "$device" >/dev/null || fail "$device failed mdadm detail test"
      }

      check_mount_uuid() {
        mountpoint="$1"
        expected_uuid="$2"
        expected_type="$3"
        target="$(findmnt -rn -o TARGET --target "$mountpoint" || true)"
        [[ "$target" == "$mountpoint" ]] || fail "$mountpoint fell through to another filesystem"
        source="$(findmnt -rn -o SOURCE --target "$mountpoint")"
        actual_uuid="$(blkid -s UUID -o value "$source")"
        actual_type="$(blkid -s TYPE -o value "$source")"
        [[ "''${actual_uuid^^}" == "''${expected_uuid^^}" ]] || fail "$mountpoint filesystem UUID drifted"
        [[ "$actual_type" == "$expected_type" ]] || fail "$mountpoint filesystem type drifted"
      }

      check_esp() {
        mountpoint="$1"
        expected_partuuid="$2"
        expected_uuid="$3"
        check_mount_uuid "$mountpoint" "$expected_uuid" vfat
        source="$(findmnt -rn -o SOURCE --target "$mountpoint")"
        actual_partuuid="$(blkid -s PARTUUID -o value "$source" | tr '[:upper:]' '[:lower:]')"
        [[ "$actual_partuuid" == "$expected_partuuid" ]] || fail "$mountpoint PARTUUID drifted"
      }

      # Exact component_size values observed from sysfs and mdadm on the
      # physical host; keep the health gate aligned with the creation sizes.
      check_array root 464519168
      check_array data 1871708160
      check_mount_uuid / ${ids.filesystemUuids.root} ext4
      check_mount_uuid /data ${ids.filesystemUuids.data} ext4
      check_esp /boot-a ${ids.partuuids.espA} ${ids.filesystemUuids.espA}
      check_esp /boot-b ${ids.partuuids.espB} ${ids.filesystemUuids.espB}

      data_source="$(findmnt -rn -o SOURCE --target /data)"
      data_features="$(tune2fs -l "$data_source" | sed -n 's/^Filesystem features:[[:space:]]*//p')"
      [[ " $data_features " == *" quota "* ]] || fail "/data lacks quota feature"
      [[ " $data_features " == *" project "* ]] || fail "/data lacks project feature"
      data_options="$(findmnt -rn -o OPTIONS --target /data)"
      [[ ",$data_options," == *,prjquota,* ]] || fail "/data is not mounted with prjquota"

      [[ -f /swapfile ]] || fail "/swapfile is missing"
      [[ "$(stat -c %s /swapfile)" == 68719476736 ]] || fail "/swapfile is not exactly 64 GiB"
      swapon --show=NAME --noheadings | awk '{print $1}' | grep -Fx /swapfile >/dev/null || fail "/swapfile is not active"
      [[ "$(</sys/module/zswap/parameters/enabled)" == Y ]] || fail "zswap is disabled"
      [[ "$(</sys/module/zswap/parameters/max_pool_percent)" == 10 ]] || fail "zswap pool is not 10 percent"
      [[ "$(sysctl -n vm.swappiness)" == 20 ]] || fail "vm.swappiness is not 20"

      for device in \
        ${ids.disks.rootA} \
        ${ids.disks.rootB} \
        ${ids.disks.dataA} \
        ${ids.disks.dataB}; do
        smartctl -H "$device" | grep -F "PASSED" >/dev/null || fail "$device SMART health failed"
      done
    '';
  };

  mdCheck = pkgs.writeShellApplication {
    name = "finite-md-check";
    runtimeInputs = with pkgs; [
      coreutils
      util-linux
    ];
    text = ''
      exec 9>/run/lock/finite-md-check.lock
      flock -n 9 || {
        echo "another MD maintenance operation holds the lock" >&2
        exit 75
      }

      for name in root data; do
        device="/dev/md/$name"
        [[ -b "$device" ]]
        block="$(basename "$(readlink -f "$device")")"
        sys="/sys/block/$block/md"
        [[ "$(<"$sys/degraded")" == 0 ]]
        [[ "$(<"$sys/sync_action")" == idle ]]
        echo check >"$sys/sync_action"
        while [[ "$(<"$sys/sync_action")" != idle ]]; do
          sleep 30
        done
        mismatch="$(<"$sys/mismatch_cnt")"
        if [[ "$mismatch" != 0 ]]; then
          echo "$device mismatch count is $mismatch; refusing automatic repair" >&2
          exit 1
        fi
      done
    '';
  };

  prepareData = pkgs.writeShellScript "finite-prepare-data-root" ''
    set -euo pipefail
    source="$(${pkgs.util-linux}/bin/findmnt -rn -o SOURCE --target /data)"
    uuid="$(${pkgs.util-linux}/bin/blkid -s UUID -o value "$source")"
    [[ "$uuid" == ${ids.filesystemUuids.data} ]]
    ${pkgs.coreutils}/bin/install -d -m 0700 /data/agents /data/staging
    printf '%s\n' ${ids.filesystemUuids.data} > /data/.finite-filesystem-identity
    ${pkgs.coreutils}/bin/chmod 0600 /data/.finite-filesystem-identity
  '';
in
{
  systemd.services = {
    finite-prepare-data-root = {
      description = "Prepare bounded finite-lat-3 data roots on the exact data filesystem";
      requires = [ "data.mount" ];
      after = [ "data.mount" ];
      wantedBy = [ "multi-user.target" ];
      unitConfig.ConditionPathIsMountPoint = "/data";
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = prepareData;
      };
    };

    finite-storage-health = {
      description = "Fail-closed finite-lat-3 storage, ESP, swap, and SMART health";
      after = [
        "local-fs.target"
        "swap.target"
      ];
      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${storageHealth}/bin/finite-storage-health";
      };
    };

    finite-md-check = {
      description = "Serialized finite-lat-3 MD consistency check";
      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${mdCheck}/bin/finite-md-check";
        TimeoutStartSec = "12h";
      };
    };
  };

  systemd.timers = {
    finite-storage-health = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnBootSec = "10min";
        OnUnitActiveSec = "5min";
        Unit = "finite-storage-health.service";
      };
    };

    finite-md-check = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnCalendar = "*-*-01 03:00:00";
        RandomizedDelaySec = "2h";
        Persistent = true;
        Unit = "finite-md-check.service";
      };
    };
  };
}
