#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

test -f README.md
test -f skills/AGENTS.md

python3 - <<'PY'
from pathlib import Path
import sys

root = Path("skills")
skill_files = sorted(root.rglob("SKILL.md"))
if not skill_files:
    print("no SKILL.md files found", file=sys.stderr)
    sys.exit(1)

names: dict[str, Path] = {}
errors: list[str] = []

for path in skill_files:
    text = path.read_text(encoding="utf-8")
    if not text.strip():
        errors.append(f"{path}: empty file")
        continue
    lines = text.splitlines()
    if lines[0].strip() != "---":
        errors.append(f"{path}: missing YAML frontmatter opener")
        continue
    try:
        close_idx = lines[1:].index("---") + 1
    except ValueError:
        errors.append(f"{path}: missing YAML frontmatter closer")
        continue
    frontmatter = lines[1:close_idx]
    fields: dict[str, str] = {}
    for line in frontmatter:
        if ":" not in line or line.startswith((" ", "\t", "-")):
            continue
        key, value = line.split(":", 1)
        fields[key.strip()] = value.strip().strip('"')
    for required in ("name", "description"):
        if not fields.get(required):
            errors.append(f"{path}: missing non-empty {required!r} frontmatter")
    name = fields.get("name")
    if name:
        previous = names.get(name)
        if previous is not None:
            errors.append(f"{path}: duplicate skill name {name!r}; first seen at {previous}")
        names[name] = path

if errors:
    for error in errors:
        print(error, file=sys.stderr)
    sys.exit(1)

print(f"finite-skills static checks passed ({len(skill_files)} skills)")
PY
