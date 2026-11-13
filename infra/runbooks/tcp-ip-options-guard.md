# IPv4 TCP options guard

The Latitude hosts import `nixos/modules/tcp-ip-options-guard.nix` for
[CVE-2026-98432](https://git.kernel.org/pub/scm/linux/security/vulns.git/plain/cve/published/2026/CVE-2026-98432.mbox).
It pins the fixed 6.18 kernel source without advancing the OS, application,
or Kata inputs. The early boot service runs
`infra/scripts/mitigate-cve-2026-98432`: drop IPv4 TCP packets with IP options
in raw PREROUTING, then disable MPTCP. Ordinary TCP, IPv6, and UDP (including
WireGuard) do not match. IP options are distinct from TCP options such as MSS.

The filter applies before conntrack, host listeners, and routed guest traffic.
It does not establish protection for guest-local or directly bridged traffic
that bypasses the host IPv4 stack. Keep it until the affected host and reachable
guest kernels are fixed. Disabling MPTCP alone does not retire existing
listeners, so do not remove the filter just because the sysctl is zero.

## Observe and verify

Run the current complete `finite-status` bundle as root on each host:

```sh
scripts/finite-status --json
scripts/finite-status --tcp-security
iptables -t raw -L PREROUTING -nvx --line-numbers
```

The TCP probe requires the exact barrier to be the first raw PREROUTING rule,
MPTCP disabled, the guard service active, and a persistent unit plus matching
boot dependency on disk. A loaded `WantedBy` property alone is insufficient:
runtime-enabled units and dependencies under `/run` disappear on reboot.
Green means the mitigation is present, not that the kernel is patched. Compare
`uname -r` after reboot against the reviewed fixed kernel artifact. No reboot
or product-service restart is needed to install the filter itself.

Reapplying the guard repairs ordering by inserting a new first rule whenever
the existing barrier is absent or below another rule. It keeps older copies
instead of deleting by rule number, preserving protection and unrelated rules
throughout the repair. Repeated application with the barrier first adds nothing.

## Emergency service and declarative handoff

The emergency installer uses a writable operator unit because NixOS owns
`/etc/systemd/system` as a read-only store symlink. Its files are:

- `/etc/finite/cve-2026-98432.sh` (the repository mitigation script)
- `/etc/finite/cve-2026-98432.service` (the operator unit source)
- `/etc/systemd/system.control/finite-cve-2026-98432.service` (symlink)
- `/etc/systemd/system.control/multi-user.target.wants/finite-cve-2026-98432.service`
  (boot dependency symlink)

The unit is a oneshot ordered before `network-pre.target`, after module loading,
with `DefaultDependencies=no`, `RemainAfterExit=yes`, and no `ExecStop`.
It invokes the script with `/run/current-system/sw/bin/bash` and
`PATH=/run/current-system/sw/bin:/run/wrappers/bin`. The operator unit remains
authoritative across reboot and NixOS activation until explicitly removed.

Deploy the reviewed CI closure through the host's normal runbook. Verify that
the new closure contains the guard unit before removing the operator override:

```sh
test -e /run/current-system/etc/systemd/system/finite-cve-2026-98432.service
rm /etc/systemd/system.control/multi-user.target.wants/finite-cve-2026-98432.service
rm /etc/systemd/system.control/finite-cve-2026-98432.service
systemctl daemon-reload
systemctl restart finite-cve-2026-98432.service
scripts/finite-status --tcp-security
```

Restart only the guard: it has no stop action and its apply operation is
idempotent, so the existing filter stays installed throughout the handoff.
Verify the probe's `unit_fragment` resolves to the declarative unit, then remove
the two obsolete `/etc/finite/cve-2026-98432.*` files. Follow the normal
Runner drain and reboot procedures to replace the running kernel; switching
the closure alone does not replace it. Do not combine this with Runtime
replacement or user-data repair.

## Rollback boundary

The live mitigation changes only packet filtering, the MPTCP sysctl, and its
operator boot service. It does not change chat stores, identity, Runtime
assignments, volumes, or recovery material. No data restore is needed to undo
it. Pre-change firewall, sysctl, service, and canonical status evidence is kept
in `/run/finite-cve-2026-98432/` until reboot; copy required evidence off-host
before rebooting. Never restore the entire old firewall over concurrent changes.

Removing this mitigation reopens the vulnerability on an unpatched kernel.
If separately authorized because it causes an incident, remove only the guard's
unit and exact rule, and restore the recorded pre-change MPTCP value:

```sh
systemctl stop finite-cve-2026-98432.service
# Remove its boot symlink/unit using the operator or declarative owner above.
while iptables -w 10 -t raw -C PREROUTING -p tcp -m u32 --u32 '0>>24&0xF=6:15' \
  -m comment --comment finite-cve-2026-98432 -j DROP; do
  iptables -w 10 -t raw -D PREROUTING -p tcp -m u32 --u32 '0>>24&0xF=6:15' \
    -m comment --comment finite-cve-2026-98432 -j DROP
done
sysctl -w net.mptcp.enabled="$(cat /run/finite-cve-2026-98432/mptcp-before)"
```

Run both status modes and product probes before and after any rollback. A
binary or boot rollback to the previous vulnerable kernel must retain the
guard and operator boot unit.
