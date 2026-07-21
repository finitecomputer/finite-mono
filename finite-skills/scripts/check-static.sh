#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

test -f README.md
test -f skills/AGENTS.md

python3 - <<'PY'
from pathlib import Path
import re
import sys

root = Path("skills")
skill_files = sorted(root.rglob("SKILL.md"))
if not skill_files:
    print("no SKILL.md files found", file=sys.stderr)
    sys.exit(1)

names: dict[str, Path] = {}
errors: list[str] = []
frontmatter_by_path: dict[Path, dict[str, str]] = {}

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
    for index, line in enumerate(frontmatter):
        if ":" not in line or line.startswith((" ", "\t", "-")):
            continue
        key, value = line.split(":", 1)
        value = value.strip().strip('"')
        if value in (">", "|"):
            continuation_lines: list[str] = []
            for continuation in frontmatter[index + 1 :]:
                if not continuation.startswith((" ", "\t")):
                    break
                continuation_lines.append(continuation.strip())
            value = " ".join(continuation_lines)
        fields[key.strip()] = value
    for required in ("name", "description"):
        if not fields.get(required):
            errors.append(f"{path}: missing non-empty {required!r} frontmatter")
    frontmatter_by_path[path] = fields
    name = fields.get("name")
    if name:
        previous = names.get(name)
        if previous is not None:
            errors.append(f"{path}: duplicate skill name {name!r}; first seen at {previous}")
        names[name] = path

sites_path = root / "software-development/finite-sites-publishing-finite/SKILL.md"
if not sites_path.is_file():
    errors.append(f"{sites_path}: canonical Finite Sites skill is required")
else:
    fields = frontmatter_by_path.get(sites_path, {})
    if fields.get("name") != "finite-sites-publishing-finite":
        errors.append(
            f"{sites_path}: name must be 'finite-sites-publishing-finite'"
        )
    description = fields.get("description", "").lower()
    required_description_terms = (
        "finite sites",
        "finite-sites",
        "fsite",
        "site/website",
        "publish",
        "preview",
        "list",
        "share",
        "document",
        "stateful app",
    )
    for term in required_description_terms:
        if term not in description:
            errors.append(
                f"{sites_path}: description must explicitly include {term!r}"
            )

    sites_text = sites_path.read_text(encoding="utf-8")
    required_contract_markers = (
        "fsite` 0.4.0",
        'kind = "site"',
        'kind = "document"',
        'kind = "app"',
        "fsite project list --output json",
        "fsite project status PROJECT --output json",
        "fsite view URL_OR_NAME --output json",
        "fsite project share PROJECT OUTPUT",
        "0.0.0.0:$PORT",
        "DATA_DIR",
    )
    for marker in required_contract_markers:
        if marker not in sites_text:
            errors.append(f"{sites_path}: missing fsite 0.4.0 contract marker {marker!r}")

brain_path = root / "software-development/finitebrain/SKILL.md"
if not brain_path.is_file():
    errors.append(f"{brain_path}: canonical FiniteBrain skill is required")
else:
    brain_fields = frontmatter_by_path.get(brain_path, {})
    brain_description = brain_fields.get("description", "").lower()
    for term in (
        "brain/wiki",
        "personal",
        "knowledge-base",
        "llm-wiki-finite",
        ".wiki/",
        "~/wiki/",
        "configured wiki hub",
    ):
        if term not in brain_description:
            errors.append(
                f"{brain_path}: description must explicitly route {term!r} requests"
            )

    component_brain_path = Path("../finite-brain/skills/finitebrain/SKILL.md")
    if not component_brain_path.is_file():
        errors.append(f"{component_brain_path}: FiniteBrain reference copy is required")
    elif component_brain_path.read_text(encoding="utf-8") != brain_path.read_text(
        encoding="utf-8"
    ):
        errors.append(f"{component_brain_path}: must match canonical {brain_path}")

    brain_text = brain_path.read_text(encoding="utf-8")
    for marker in (
        'SERVER="${FINITE_BRAIN_SERVER_URL:?',
        'FBRAIN_CONFIG_DIR',
        'FBRAIN_WORKING_TREE_ROOT',
        'BRAIN="replace-with-brain-id"',
        "A Working Tree remembers the server",
        "bootstrap-personal",
        "role `personal_agent`",
        "do not require exact",
        "`remoteChanges[].actorNpub`",
        "signed actor evidence",
        "different actor means another principal changed the Brain",
        "otherwise report",
        "the cause as unknown",
    ):
        if marker not in brain_text:
            errors.append(f"{brain_path}: missing runtime routing marker {marker!r}")

    behavior_contracts = (
        (r"clearly\s+says\s+Personal Brain\s+or\s+Organization/Org Brain", "explicit Brain types proceed"),
        (r"ask\s+one\s+short\s+natural-language\s+question", "ambiguous type clarification"),
        (r"Personal Brain.*already exists", "existing Personal Brain handling"),
        (r"same-named Organization Brain", "same-named Organization Brain handling"),
        (r"\[Open Brain\]\(\.\/brain\?brainId=", "Open Brain navigation"),
        (r"navigation only; it does not\s+grant access", "navigation is not authority"),
    )
    for pattern, behavior in behavior_contracts:
        if not re.search(pattern, brain_text, re.IGNORECASE | re.DOTALL):
            errors.append(f"{brain_path}: missing managed Brain behavior for {behavior}")

    brain_reference_path = brain_path.parent / "references/fbrain-cli.md"
    component_brain_reference_path = Path(
        "../finite-brain/skills/finitebrain/references/fbrain-cli.md"
    )
    if not brain_reference_path.is_file():
        errors.append(f"{brain_reference_path}: canonical FiniteBrain CLI reference is required")
    elif not component_brain_reference_path.is_file():
        errors.append(
            f"{component_brain_reference_path}: FiniteBrain CLI reference copy is required"
        )
    elif component_brain_reference_path.read_text(
        encoding="utf-8"
    ) != brain_reference_path.read_text(encoding="utf-8"):
        errors.append(
            f"{component_brain_reference_path}: must match canonical {brain_reference_path}"
        )
    for forbidden_server in (
        'SERVER="https://finite.computer"',
        'SERVER="https://brain.smoke.finite.computer"',
    ):
        if forbidden_server in brain_text:
            errors.append(
                f"{brain_path}: active server must come from the runtime, not {forbidden_server!r}"
            )
    for retired_contract in (
        "/brain setup",
        "personal-brain-bootstrap-authorizations",
        "role `member`",
    ):
        if retired_contract in brain_text:
            errors.append(
                f"{brain_path}: retired Personal Agent contract remains {retired_contract!r}"
            )

compat_path = root / "software-development/publish-web-apps-finite/SKILL.md"
if not compat_path.is_file():
    errors.append(f"{compat_path}: compatibility router is required")
else:
    compat_text = compat_path.read_text(encoding="utf-8")
    if "finite-sites-publishing-finite" not in compat_text:
        errors.append(f"{compat_path}: must route to finite-sites-publishing-finite")
    if len(compat_text.splitlines()) > 30:
        errors.append(f"{compat_path}: compatibility router must remain thin")

website_path = root / "software-development/website-building-finite/SKILL.md"
if website_path.is_file():
    website_text = website_path.read_text(encoding="utf-8")
    for marker in ("finite-sites-publishing-finite", "fsite"):
        if marker not in website_text:
            errors.append(f"{website_path}: must use current Finite Sites marker {marker!r}")

llm_wiki_path = root / "research/llm-wiki-finite/SKILL.md"
if llm_wiki_path.is_file():
    llm_wiki_fields = frontmatter_by_path.get(llm_wiki_path, {})
    llm_wiki_description = llm_wiki_fields.get("description", "").lower()
    for marker in (
        "repository llm-wiki",
        "explicitly invokes",
        ".wiki/",
        "~/wiki/",
        "configured hub",
        "brain/wiki",
        "personal",
        "knowledge-base",
        "finitebrain",
    ):
        if marker not in llm_wiki_description:
            errors.append(
                f"{llm_wiki_path}: missing routing boundary marker {marker!r}"
            )

legacy_command = re.compile(r"\bfinitec\s+(?:publish|repo|skills)\b", re.IGNORECASE)
text_extensions = {".md", ".sh", ".py", ".json", ".toml", ".yaml", ".yml", ".txt"}
for path in sorted(root.rglob("*")):
    if not path.is_file() or path.suffix.lower() not in text_extensions:
        continue
    text = path.read_text(encoding="utf-8")
    match = legacy_command.search(text)
    if match:
        line = text.count("\n", 0, match.start()) + 1
        errors.append(
            f"{path}:{line}: retired managed-baseline command {match.group(0)!r}"
        )

if errors:
    for error in errors:
        print(error, file=sys.stderr)
    sys.exit(1)

print(f"finite-skills static checks passed ({len(skill_files)} skills)")
PY
