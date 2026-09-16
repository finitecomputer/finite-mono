# Executed before the configured Runner binary, including an older binary on
# rollback. Nix store paths are immutable; only this lifecycle implementation
# writes the publisher identity. A missing/different identity requires exit.
set -eu
candidate=$(readlink -f "$1")
state=/run/finite-hosted-hermes
unit=finite-hosted-hermes.service
if test -f "$state/publisher" && test "$(cat "$state/publisher")" = "$candidate"; then
  exit 0
fi
before=$(systemctl show "$unit" --property=ControlGroup --value)
systemctl stop "$unit"
test "$(systemctl show "$unit" --property=MainPID --value)" = 0
case "$(systemctl show "$unit" --property=ActiveState --value)" in
  inactive|failed) ;;
  *) exit 1 ;;
esac
after=$(systemctl show "$unit" --property=ControlGroup --value)
for group in "$before" "$after"; do
  test -n "$group" || continue
  case "$group" in
    /system.slice/finite-hosted-hermes.service) ;;
    *) exit 1 ;;
  esac
  events="/sys/fs/cgroup$group/cgroup.events"
  # cgroup.events' populated value includes all descendant cgroups.
  if test -e "$events"; then
    grep -qx 'populated 0' "$events"
  fi
done
