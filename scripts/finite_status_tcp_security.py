"""Read-only CVE-2026-98432 mitigation evidence for the Latitude fleet."""

from __future__ import annotations

import platform
import shlex
import subprocess
from datetime import datetime, timezone
from pathlib import Path


RULE = [
    "-A", "PREROUTING", "-p", "tcp", "-m", "u32", "--u32",
    "0x0>>0x18&0xf=0x6:0xf", "-m", "comment", "--comment",
    "finite-cve-2026-98432", "-j", "DROP",
]
UNIT = "finite-cve-2026-98432.service"
PERSISTENT_UNIT_DIRS = ("/etc/systemd/system.control", "/etc/systemd/system")


def persistent_boot(unit: dict[str, str], root: Path) -> dict:
    """Verify the loaded unit and a matching boot link survive loss of /run."""
    evidence = {"unit_persistent": False, "persistent_boot_link": None}

    def persistent_path(logical: str) -> Path:
        path = Path(logical)
        if not path.is_absolute() or ".." in path.parts:
            raise ValueError("invalid absolute unit path")
        # Check both the original location and symlink destinations. An /etc
        # alias to a /run unit (or wants directory) is still runtime-only.
        prefixes = ("/etc/", "/nix/store/")
        if not logical.startswith(prefixes):
            raise ValueError("unit path is not persistent")
        mapped = root / path.relative_to("/")
        for candidate in (mapped.parent, mapped):
            physical = candidate.resolve(strict=True)
            resolved = "/" + physical.relative_to(root).as_posix()
            if not resolved.startswith(prefixes):
                raise ValueError("unit path resolves outside persistent storage")
        if not mapped.is_file():
            raise ValueError("unit path is not a file")
        return mapped.resolve(strict=True)

    try:
        active = persistent_path(unit.get("FragmentPath", ""))
    except (OSError, RuntimeError, ValueError):
        return evidence
    for directory in PERSISTENT_UNIT_DIRS:
        try:
            logical = f"{directory}/{UNIT}"
            candidate = root / logical.lstrip("/")
            if candidate.exists() or candidate.is_symlink():
                evidence["unit_persistent"] = persistent_path(logical) == active
                break
        except (OSError, RuntimeError, ValueError):
            # A higher-priority mask, broken link, or runtime alias must not
            # be ignored in favor of a lower-priority persistent definition.
            return evidence
    if not evidence["unit_persistent"]:
        return evidence
    for directory in PERSISTENT_UNIT_DIRS:
        link = f"{directory}/multi-user.target.wants/{UNIT}"
        try:
            candidate = root / link.lstrip("/")
            if candidate.exists() or candidate.is_symlink():
                if candidate.is_symlink() and persistent_path(link) == active:
                    evidence["persistent_boot_link"] = link
                break
        except (OSError, RuntimeError, ValueError):
            break
    return evidence


def verdict(mptcp: str, rules: str, unit: dict[str, str], *, root: Path = Path("/")) -> dict:
    """Require the early packet barrier, not just a disabled MPTCP sysctl."""
    ordered = [shlex.split(line) for line in rules.splitlines() if line.startswith("-A ")]
    barrier = bool(ordered and ordered[0] == RULE)
    persistence = persistent_boot(unit, root)
    boot_attached = (
        "multi-user.target" in unit.get("WantedBy", "").split()
        and persistence["persistent_boot_link"] is not None
    )
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
        **persistence,
        "scope": "host IPv4 PREROUTING, including routed guest traffic; not a guest-kernel patch audit",
        "kernel_fix": "6.18.56 is the first fixed upstream 6.18 release; mitigation is not a kernel patch",
    }


def collect(*, root: Path = Path("/")) -> dict:
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
        section = verdict(mptcp, rules, unit, root=root)
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
