"""Read-only CVE-2026-98432 mitigation evidence for the Latitude fleet."""

from __future__ import annotations

import platform
import shlex
import subprocess
from datetime import datetime, timezone


RULE = [
    "-A", "PREROUTING", "-p", "tcp", "-m", "u32", "--u32",
    "0x0>>0x18&0xf=0x6:0xf", "-m", "comment", "--comment",
    "finite-cve-2026-98432", "-j", "DROP",
]


def verdict(mptcp: str, rules: str, unit: dict[str, str]) -> dict:
    """Require the early packet barrier, not just a disabled MPTCP sysctl."""
    ordered = [shlex.split(line) for line in rules.splitlines() if line.startswith("-A ")]
    barrier = bool(ordered and ordered[0] == RULE)
    boot_attached = "multi-user.target" in unit.get("WantedBy", "").split()
    active = unit.get("ActiveState") == "active"
    mitigated = barrier and mptcp.strip() == "0" and active and boot_attached
    return {
        "status": "green" if mitigated else "red",
        "state": "mitigated" if mitigated else "mitigation-incomplete",
        "mptcp_disabled": mptcp.strip() == "0",
        "ipv4_tcp_options_drop_first": barrier,
        "unit_active": active,
        "unit_attached_to_boot": boot_attached,
        "unit_fragment": unit.get("FragmentPath"),
        "scope": "host IPv4 PREROUTING, including routed guest traffic; not a guest-kernel patch audit",
        "kernel_fix": "6.18.56 is the first fixed upstream 6.18 release; mitigation is not a kernel patch",
    }


def collect() -> dict:
    def run(*args: str) -> str:
        result = subprocess.run(args, capture_output=True, text=True, timeout=15, check=True)
        return result.stdout

    try:
        mptcp = run("sysctl", "-n", "net.mptcp.enabled")
        rules = run("iptables", "-w", "10", "-t", "raw", "-S", "PREROUTING")
        properties = run(
            "systemctl", "show", "finite-cve-2026-98432.service",
            "-p", "ActiveState", "-p", "WantedBy", "-p", "FragmentPath",
        )
        unit = dict(line.split("=", 1) for line in properties.splitlines() if "=" in line)
        section = verdict(mptcp, rules, unit)
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        section = {"status": "unknown", "state": "unavailable", "error": str(error)}
    section["running_kernel"] = platform.release()
    return {
        "schema_version": "finite.status.v1",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "overall_status": section["status"],
        "exit_code": {"green": 0, "red": 1, "unknown": 2}[section["status"]],
        "sections": {"tcp_security": section},
    }
