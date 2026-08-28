#!/usr/bin/env python3
"""Keep runbook operational facts aligned with their in-repo authorities.

Runbooks hand-copy facts the repo already contains as code, and they drift
(2026-08-20 audit, M8: a rollout runbook named table `agent_departure_facts`
for `brain_agent_departure_facts`, stated a stale migration ceiling, and used
the binary name where the systemd unit is `finite-saas-sites`). Each drift
cost a mid-incident detour. Three precise checks over infra/runbooks/*.md:

1. Table references — an uppercase SQL keyword (FROM/JOIN/INTO/UPDATE/
   TRUNCATE) followed by a lowercase identifier, in a fenced or inline code
   span — must resolve to a CREATE TABLE or RENAME TO target in first-party
   Rust/SQL sources. Catalog relations (pg_*, sqlite_*) are excluded.
2. Migration references resolve against the numbered authorities:
   `Migration NNNN` must exist under finite-saas-core/migrations, a
   `SCHEMA_V<n>` token must exist in finite-brain-store's schema constants,
   and an explicit "migration ceiling"/"schema ceiling" claim must equal the
   actual maximum. Historical mentions of specific versions are not ceilings.
3. Unit names in unit-reference contexts (systemctl verb arguments,
   journalctl -u values) resolve against infra/nixos module definitions,
   infra/hosts/*/systemd unit files, or the documented legacy set below.
   Globs (`finite-*`) and binary-name mentions (e.g. finitesitesd) outside
   systemctl/journalctl arguments are not unit references and stay unchecked.
4. Retired ledgers stay retired. `compat/matrix.toml` was a hand-maintained
   deployment ledger that nothing read and that drifted from the pins that
   run (2026-08-21 ownership audit, O7); the runbooks now point at release
   tags, Core's runtime-artifact table, the NixOS closure, and
   infra/deployment-changelog.md. Any runbook mention of the file fails.
"""

from __future__ import annotations

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

IGNORED_DIRS = {".git", "target", "node_modules", "finite-mono-worktrees"}
SOURCE_SUFFIXES = {".rs", ".sql"}

CORE_MIGRATIONS = (
    ROOT / "finitecomputer-v2" / "crates" / "finite-saas-core" / "migrations"
)
BRAIN_SCHEMA = (
    ROOT / "finite-brain" / "crates" / "finite-brain-store" / "src" / "schema.rs"
)

# Units break-glass.md references deliberately that belonged to the pre-NixOS
# hosts; they are not defined anywhere in this repo.
LEGACY_UNITS = {
    "k3s",
    "fc-offsite-backup",
    "fc-agent-cluster-http-bridge",
    "fc-agent-cluster-https-bridge",
}

# NixOS modules that provide a same-named systemd unit when enabled.
NIXOS_SERVICE_UNITS = ("caddy", "postgresql")

TABLE_REF = re.compile(r"\b(?:FROM|JOIN|INTO|UPDATE|TRUNCATE)\s+([a-z][a-z0-9_]*)")
CATALOG_PREFIXES = ("pg_", "sqlite_")

CEILING_CLAIM = re.compile(
    r"\b(migration|schema)\s+ceiling\b\s*(?:is|=|at|:)?\s*V?0*(\d+)", re.IGNORECASE
)
CORE_MIGRATION_REF = re.compile(r"\b[Mm]igration\s+(\d{4})\b")
BRAIN_SCHEMA_REF = re.compile(r"\bSCHEMA_V(\d+)\b")

JOURNAL_UNIT = re.compile(r"(?:^|\s)-[a-zA-Z]*u\s+['\"]?([a-zA-Z0-9@:._*+-]+)")
SYSTEMCTL = re.compile(
    r"\bsystemctl\b(?:\s+--?[\w=.-]+)*\s+"
    r"(cat|status|start|stop|restart|try-restart|reload|is-active|is-enabled|"
    r"is-failed|show|enable|disable|mask|unmask|list-units)\b"
    r"((?:\s+--?[\w=.-]+)*)(.*)"
)
UNIT_TOKEN = re.compile(r"^[a-zA-Z][a-zA-Z0-9@:._*+-]*$")

RETIRED_LEDGER = re.compile(r"\bmatrix\.toml\b")


def runbooks() -> list[Path]:
    return sorted((ROOT / "infra" / "runbooks").glob("*.md"))


def source_files() -> list[Path]:
    files = []
    for path in ROOT.rglob("*"):
        if any(part in IGNORED_DIRS for part in path.relative_to(ROOT).parts):
            continue
        if path.suffix in SOURCE_SUFFIXES:
            files.append(path)
    return files


def known_tables() -> set[str]:
    tables = set()
    create = re.compile(
        r"CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-zA-Z_]\w*)", re.IGNORECASE
    )
    rename = re.compile(r"\bRENAME\s+TO\s+([a-zA-Z_]\w*)", re.IGNORECASE)
    for path in source_files():
        text = path.read_text(encoding="utf-8")
        tables.update(match.group(1).lower() for match in create.finditer(text))
        tables.update(match.group(1).lower() for match in rename.finditer(text))
    return tables


def code_lines(path: Path) -> list[tuple[int, str]]:
    """Numbered code lines: fenced blocks plus inline backtick spans."""
    lines = []
    in_fence = False
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if line.strip().startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            lines.append((lineno, line))
        else:
            lines.extend((lineno, span) for span in re.findall(r"`([^`]+)`", line))
    return lines


def check_tables() -> list[str]:
    tables = known_tables()
    failures = []
    for path in runbooks():
        for lineno, line in code_lines(path):
            for match in TABLE_REF.finditer(line):
                name = match.group(1)
                if name.startswith(CATALOG_PREFIXES) or name in tables:
                    continue
                failures.append(
                    f"{path.relative_to(ROOT)}:{lineno}: table `{name}` matches no"
                    " CREATE TABLE/RENAME TO in repo sources"
                )
    return failures


def migration_ceilings() -> tuple[int, int]:
    core = max(int(p.name[:4]) for p in CORE_MIGRATIONS.glob("????_*.sql"))
    brain = max(
        int(v)
        for v in BRAIN_SCHEMA_REF.findall(BRAIN_SCHEMA.read_text(encoding="utf-8"))
    )
    return core, brain


def check_migrations() -> list[str]:
    core_ceiling, brain_ceiling = migration_ceilings()
    failures = []
    for path in runbooks():
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(ROOT)
        for match in CORE_MIGRATION_REF.finditer(text):
            if not list(CORE_MIGRATIONS.glob(f"{match.group(1)}_*.sql")):
                failures.append(
                    f"{relative}: migration {match.group(1)} has no file in"
                    f" {CORE_MIGRATIONS.relative_to(ROOT)}"
                )
        for match in BRAIN_SCHEMA_REF.finditer(text):
            if int(match.group(1)) > brain_ceiling:
                failures.append(
                    f"{relative}: SCHEMA_V{match.group(1)} exceeds the"
                    f" finite-brain schema ceiling V{brain_ceiling}"
                )
        for match in CEILING_CLAIM.finditer(text):
            actual = (
                core_ceiling if match.group(1).lower() == "migration" else brain_ceiling
            )
            if int(match.group(2)) != actual:
                failures.append(
                    f"{relative}: stated {match.group(1).lower()} ceiling {match.group(2)}"
                    f" != actual ceiling {actual}"
                )
    return failures


def first_level_keys(text: str, attr: str) -> list[str]:
    """First-level `name = {` keys of an `attr = { ... };` nix attrset."""
    keys = []
    for match in re.finditer(rf"^( *){re.escape(attr)} = \{{$", text, re.MULTILINE):
        indent = match.group(1)
        for line in text[match.end() :].splitlines()[1:]:
            if not line.strip():
                continue
            if not line.startswith(indent + "  "):
                break
            entry = re.match(rf"{indent}  ([\w.-]+) = \{{$", line)
            if entry:
                keys.append(entry.group(1))
    return keys


def authority_units() -> set[str]:
    units = set(LEGACY_UNITS)
    for path in sorted((ROOT / "infra" / "nixos").rglob("*.nix")):
        text = path.read_text(encoding="utf-8")
        strings = dict(re.findall(r"^ *(\w+) = \"([^\"]+)\";", text, re.MULTILINE))
        services = re.finditer(
            r"systemd\.(?:services|timers)\.(?:\"([^\"]+)\"|([a-zA-Z0-9@_-]+)|\$\{(\w+)\}) =",
            text,
        )
        for match in services:
            quoted, bare, variable = match.groups()
            name = quoted or bare or strings.get(variable)
            if name:
                units.add(name)
        jobs = re.finditer(
            r"services\.borgbackup\.jobs\.(?:\"([^\"]+)\"|([a-zA-Z0-9_-]+)) =", text
        )
        units.update("borgbackup-job-" + (m.group(1) or m.group(2)) for m in jobs)
        containers = re.finditer(
            r"oci-containers\.containers\.(?:\"([^\"]+)\"|([a-zA-Z0-9_-]+)) =", text
        )
        units.update("podman-" + (m.group(1) or m.group(2)) for m in containers)
        units.update(
            "podman-" + key
            for key in first_level_keys(
                text, "virtualisation.oci-containers.containers"
            )
        )
        for service in NIXOS_SERVICE_UNITS:
            if re.search(rf"services\.{service}(\.enable = true| = \{{)", text):
                units.add(service)
        # Per-db replicator units are finite-litestream-<db.name>; the db names
        # are authored in the host config (modules/finite-litestream.nix).
        if "finite.litestream" in text:
            for block in re.finditer(r"\bdbs = \[(.*?)\];", text, re.DOTALL):
                units.update(
                    "finite-litestream-" + name
                    for name in re.findall(r"\bname = \"([^\"]+)\";", block.group(1))
                )
    for unit_file in list(
        (ROOT / "infra" / "hosts").glob("*/systemd/*.service")
    ) + list((ROOT / "infra" / "hosts").glob("*/systemd/*.timer")):
        units.add(unit_file.name.rsplit(".", 1)[0])
    return units


def systemctl_units(line: str) -> list[str]:
    names = []
    for match in SYSTEMCTL.finditer(line):
        # An inline-code command ends at its closing backtick; prose after it
        # is not a unit list.
        rest = match.group(3).split("`", 1)[0]
        for token in rest.split():
            token = token.strip("'\"`")
            if not UNIT_TOKEN.match(token) or token in (
                "systemctl",
                "journalctl",
                "sudo",
            ):
                break
            if "*" not in token:
                names.append(token)
    return names


def journalctl_units(line: str) -> list[str]:
    """-u values, scoped to journalctl commands (sudo -u postgres is not one)."""
    names = []
    for match in re.finditer(r"\bjournalctl\b", line):
        command = re.split(r"[`;]", line[match.start() :])[0]
        names.extend(
            m.group(1) for m in JOURNAL_UNIT.finditer(command) if "*" not in m.group(1)
        )
    return names


def check_units() -> list[str]:
    units = authority_units()
    failures = []
    for path in runbooks():
        lines = path.read_text(encoding="utf-8").splitlines()
        lineno = 0
        while lineno < len(lines):
            lineno += 1
            line = lines[lineno - 1]
            # A systemctl command inside an unbalanced quote continues on the
            # next line (e.g. a multi-line `ssh host 'systemctl ...'`).
            while (
                "systemctl" in line and line.count("'") % 2 == 1 and lineno < len(lines)
            ):
                line += " " + lines[lineno]
                lineno += 1
            referenced = journalctl_units(line) + systemctl_units(line)
            for name in referenced:
                base = name.removesuffix(".service").removesuffix(".timer")
                if base not in units:
                    failures.append(
                        f"{path.relative_to(ROOT)}:{lineno}: unit `{name}` matches no"
                        " infra/nixos module, infra/hosts unit file, or the legacy set"
                    )
    return failures


def check_retired_ledgers() -> list[str]:
    failures = []
    for path in runbooks():
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if RETIRED_LEDGER.search(line):
                failures.append(
                    f"{path.relative_to(ROOT)}:{lineno}: names the retired compat"
                    " matrix; point at the source of truth or"
                    " infra/deployment-changelog.md instead"
                )
    return failures


def main() -> None:
    failures = (
        check_tables() + check_migrations() + check_units() + check_retired_ledgers()
    )
    if failures:
        raise SystemExit(
            "runbook facts drifted from repo authorities:\n" + "\n".join(failures)
        )
    print("runbook facts contract: ok")


if __name__ == "__main__":
    main()
