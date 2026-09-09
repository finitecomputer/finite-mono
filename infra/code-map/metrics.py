"""Generate a local CodeCharta 2.0 metrics file from committed first-party source."""

import argparse
import collections
import datetime as dt
import hashlib
import json
import pathlib
import re
import subprocess
import tempfile

p = argparse.ArgumentParser()
p.add_argument("repo", type=pathlib.Path)
p.add_argument("--scc", default="scc")
p.add_argument("--ref", default="HEAD")
p.add_argument("--output", required=True, type=pathlib.Path)
args = p.parse_args()
repo = args.repo.resolve()
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=True)


def git(*cmd):
    return subprocess.check_output(["git", "-C", str(repo), *cmd])


sha = git("rev-parse", args.ref).decode().strip()
snapshot_time = dt.datetime.fromisoformat(
    git("show", "-s", "--format=%cI", sha).decode().strip()
)
cutoff = (snapshot_time - dt.timedelta(days=30)).date().isoformat()
tracked = set(git("ls-tree", "-rz", "--name-only", sha).decode().split("\0"))
with tempfile.TemporaryDirectory(prefix="finite-code-map-") as scratch:
    archive = pathlib.Path(scratch) / "source.tar"
    archive.write_bytes(git("archive", sha))
    source = pathlib.Path(scratch) / "source"
    source.mkdir()
    subprocess.run(["tar", "-xf", str(archive), "-C", str(source)], check=True)
    raw = subprocess.check_output(
        [args.scc, "--by-file", "--format", "json", "--no-cocomo", "--no-size", "."],
        cwd=source,
    )
    groups = json.loads(raw)
    allowed = {
        "Rust",
        "TypeScript",
        "JavaScript",
        "Python",
        "Shell",
        "BASH",
        "Nix",
        "SQL",
        "CSS",
        "SCSS",
        "HTML",
        "C",
        "C Header",
        "C++",
        "Swift",
        "Go",
        "Ruby",
        "Fish",
        "Dockerfile",
        "Makefile",
        "Just",
    }
    rows = []
    excluded = collections.Counter()
    for group in groups:
        for f in group.get("Files", []):
            path = f["Location"].removeprefix("./")
            if path not in tracked or group["Name"] not in allowed:
                excluded[group["Name"]] += 1
                continue
            parts = pathlib.PurePosixPath(path).parts
            text = (source / path).read_text(errors="replace")
            generated = re.search(
                r"(?i)(@generated|automatically generated|code generated .*do not edit|generated file.*do not edit)",
                text[:1500],
            )
            if generated or any(
                x in {"vendor", "node_modules", "dist", "target", "third_party"}
                for x in parts
            ):
                excluded["generated/vendor"] += 1
                continue
            test = any(
                x in {"tests", "__tests__", "test", "fixtures", "testdata"}
                for x in parts
            ) or bool(re.search(r"(^test_|_test\.|\.test\.|\.spec\.)", parts[-1]))
            rows.append(
                {
                    "path": path,
                    "language": group["Name"],
                    "code": f["Code"],
                    "lines": f["Lines"],
                    "complexity": f["Complexity"],
                    "test": test,
                    "inline_tests": "#[cfg(test)]" in text,
                }
            )

# Count touches and line churn on the first-parent history, including merged changes
# once. Root imports are excluded. Renames are deliberately not followed.
touches = collections.Counter()
added = collections.Counter()
deleted = collections.Counter()
log = git(
    "log",
    sha,
    "--first-parent",
    "--diff-merges=first-parent",
    "--min-parents=1",
    "--no-renames",
    f"--since={cutoff}",
    "--format=COMMIT:%H",
    "--numstat",
).decode()
for line in log.splitlines():
    m = re.fullmatch(r"(\d+)\t(\d+)\t(.+)", line)
    if m:
        a, d, name = m.groups()
        touches[name] += 1
        added[name] += int(a)
        deleted[name] += int(d)


def node_id(path):
    return hashlib.sha256(path.encode()).hexdigest()[:16]


root = {"id": node_id("root"), "name": "root", "type": "Folder", "children": []}
folders = {"": root}
attributes = {}
summary = collections.defaultdict(
    lambda: {"files": 0, "code": 0, "complexity": 0, "dedicated_test_code": 0}
)
for row in sorted(rows, key=lambda r: r["path"]):
    parts = row["path"].split("/")
    parent = root
    for i, name in enumerate(parts[:-1]):
        key = "/".join(parts[: i + 1])
        if key not in folders:
            folders[key] = {
                "id": node_id(key),
                "name": name,
                "type": "Folder",
                "children": [],
            }
            parent["children"].append(folders[key])
        parent = folders[key]
    ident = node_id(row["path"])
    parent["children"].append(
        {
            "id": ident,
            "name": parts[-1],
            "type": "File",
            "link": f"https://github.com/finitecomputer/finite-mono/blob/{sha}/{row['path']}",
        }
    )
    attributes[ident] = {
        "rloc": row["code"],
        "complexity_estimate": row["complexity"],
        "branches_per_100_loc": round(100 * row["complexity"] / max(row["code"], 1), 2),
        "main_file_loc": 0 if row["test"] else row["code"],
        "dedicated_test_loc": row["code"] if row["test"] else 0,
        "contains_inline_rust_tests": int(row["inline_tests"]),
        "touches_30d": touches[row["path"]],
        "added_30d": added[row["path"]],
        "deleted_30d": deleted[row["path"]],
        "churn_30d": added[row["path"]] + deleted[row["path"]],
    }
    s = summary[parts[0] if len(parts) > 1 else "(root)"]
    s["files"] += 1
    s["code"] += row["code"]
    s["complexity"] += row["complexity"]
    s["dedicated_test_code"] += row["code"] if row["test"] else 0

descriptions = {
    "rloc": "Source lines excluding comments and blanks. Includes tests; excludes docs, data, manifests, lockfiles, detected generated/vendor files.",
    "complexity_estimate": "scc branch/loop token count, not AST-based cyclomatic complexity. Compare within a language. Includes inline tests.",
    "branches_per_100_loc": "scc complexity estimate / code lines * 100; compare within a language. Tiny files can have high ratios.",
    "main_file_loc": "Code outside dedicated test/fixture files. INLINE TESTS REMAIN INCLUDED; this is not pure production LOC.",
    "dedicated_test_loc": "Code in files classified as tests/fixtures by path or filename.",
    "contains_inline_rust_tests": "1 if file contains #[cfg(test)]; no attempt to subtract inline test code.",
    "touches_30d": f"First-parent commit touches since {cutoff}; merged changes counted once; root imports excluded; renames not followed.",
    "added_30d": "Added lines in recent first-parent history. Includes comments, blanks, rewritten code; not net growth.",
    "deleted_30d": "Deleted lines in recent first-parent history. Includes comments and blanks.",
    "churn_30d": "Added plus deleted lines in recent first-parent history. Activity, not quality or net growth.",
}
data = {
    "meta": {"projectName": f"Finite Mono — {sha[:8]}", "apiVersion": "2.0"},
    "files": [root],
    "lenses": {
        "metrics": {
            "attributes": attributes,
            "attributeTypes": {
                k: (
                    "relative"
                    if k in {"branches_per_100_loc", "contains_inline_rust_tests"}
                    else "absolute"
                )
                for k in descriptions
            },
            "attributeDescriptors": {
                k: {"title": k, "description": v} for k, v in descriptions.items()
            },
        }
    },
}
(out / "finite-mono.cc.json").write_text(json.dumps(data, indent=2))
report = {
    "commit": sha,
    "snapshot_time": snapshot_time.isoformat(),
    "history_since": cutoff,
    "scc_version": subprocess.check_output([args.scc, "--version"]).decode().strip(),
    "components": dict(sorted(summary.items(), key=lambda x: -x[1]["code"])),
    "excluded_files": dict(excluded),
    "files": rows,
}
(out / "summary.json").write_text(json.dumps(report, indent=2))
assert len(attributes) == len(rows) == len({r["path"] for r in rows})
assert sum(a["rloc"] for a in attributes.values()) == sum(
    s["code"] for s in summary.values()
)
print(
    f"{sha[:12]}: {len(rows)} source files, {sum(r['code'] for r in rows):,} code lines"
)
