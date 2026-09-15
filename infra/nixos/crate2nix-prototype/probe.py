"""Throwaway cache-identity experiment. Does not build or publish services."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import time


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def run(args, cwd, log):
    started = time.monotonic()
    with log.open("w") as output, Path(str(log) + ".stderr").open("w") as errors:
        subprocess.run(args, cwd=cwd, stdout=output, stderr=errors, check=True)
    return round(time.monotonic() - started, 3)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def compare(before, after):
    packages = {}
    old_all, new_all = set(), set()
    for name, new in after["packages"].items():
        old = before["packages"][name]
        old_crates = {c["drv"] for c in old["crates"]}
        new_crates = {c["drv"] for c in new["crates"]}
        old_all.update(old_crates)
        new_all.update(new_crates)
        packages[name] = {
            "crane_changed": old["crane"] != new["crane"],
            "crane_deps_changed": old["craneDeps"] != new["craneDeps"],
            "candidate_changed": old["candidate"] != new["candidate"],
            "candidate_crates": len(new_crates),
            "candidate_preserved": len(old_crates & new_crates),
            "candidate_new": [c for c in new["crates"] if c["drv"] not in old_crates],
        }
    return {
        "packages": packages,
        "candidate_unique_crates": len(new_all),
        "candidate_preserved": len(old_all & new_all),
        "candidate_new": len(new_all - old_all),
        "crane_dependency_groups_changed": len(
            {
                after["packages"][n]["craneDeps"]
                for n, p in packages.items()
                if p["crane_deps_changed"]
            }
        ),
    }


def feature_spot_check(source, data, output):
    """Compare declared feature sets; this is not a Cargo compilation-unit oracle."""
    metadata_path = output / "cargo-metadata.json"
    tree_path = output / "sites-cargo-tree.txt"
    run(
        ["cargo", "metadata", "--locked", "--all-features", "--format-version", "1"],
        source,
        metadata_path,
    )
    run(
        [
            "cargo",
            "tree",
            "--locked",
            "-p",
            "finitesitesd",
            "--target",
            "x86_64-unknown-linux-gnu",
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--format",
            "{p}|{f}",
        ],
        source,
        tree_path,
    )
    metadata = json.loads(metadata_path.read_text())
    declared = {
        (p["name"], p["version"]): set(p["features"]) for p in metadata["packages"]
    }
    # crate2nix also emits implicit dependency/default cfg names absent from Cargo's
    # declared feature map. Keep the raw flags in the per-case graph for later audit.
    candidate = {
        (
            c["name"],
            c["version"],
            tuple(sorted(set(c["features"]) & declared[c["name"], c["version"]])),
        )
        for c in data["packages"]["finitesitesd"]["crates"]
    }
    cargo = set()
    for line in tree_path.read_text().splitlines():
        package, features = line.split("|")
        match = re.match(r"(\S+) v(\S+)", package)
        cargo.add(
            (
                match[1],
                match[2],
                tuple(sorted(filter(None, features.replace(" (*)", "").split(",")))),
            )
        )
    return {
        "scope": "finitesitesd, Linux, normal/build edges, declared features only",
        "cargo_variants": len(cargo),
        "candidate_variants": len(candidate),
        "only_cargo": sorted(cargo - candidate),
        "only_candidate": sorted(candidate - cargo),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--ref", default="HEAD", help="Committed source snapshot to probe"
    )
    parser.add_argument(
        "--output", type=Path, help="New scratch directory; must not exist"
    )
    args = parser.parse_args()
    if args.output:
        args.output.mkdir(parents=True, exist_ok=False)
        output = args.output.resolve()
    else:
        output = Path(tempfile.mkdtemp(prefix="finite-crate2nix-prototype.")).resolve()
    print(f"Evidence: {output}", flush=True)
    revision = subprocess.check_output(
        ["git", "rev-parse", f"{args.ref}^{{commit}}"], cwd=ROOT, text=True
    ).strip()
    archive = output / "source.tar"
    run(["git", "archive", revision, "-o", str(archive)], ROOT, output / "archive.log")
    pristine = output / "pristine"
    pristine.mkdir()
    with tarfile.open(archive) as tar:
        tar.extractall(pristine, filter="data")
    report = {
        "revision": revision,
        "kind": "derivation identity, not a build benchmark",
        "cases": {},
    }
    baseline = None
    for case in [
        "baseline",
        "identical",
        "sites-source",
        "sites-dependency",
        "lock-comment",
    ]:
        print(f"Running {case}", flush=True)
        source = output / case
        shutil.copytree(pristine, source)
        if case == "sites-source":
            path = source / "finite-sites/crates/finitesites-engine/src/lib.rs"
            path.write_text(
                path.read_text() + "\n// Throwaway cache invalidation probe.\n"
            )
        if case == "sites-dependency":
            path = source / "finite-sites/crates/finitesites-engine/Cargo.toml"
            path.write_text(
                path.read_text().replace(
                    "[dependencies]\n",
                    "[dependencies]\nhex = { workspace = true }\n",
                    1,
                )
            )
            # Cargo alone updates the scratch lockfile; no custom dependency resolver.
            run(
                ["cargo", "metadata", "--format-version", "1", "--all-features"],
                source,
                output / f"{case}-metadata.log",
            )
        lock_before = digest(source / "Cargo.lock")
        generation_s = run(
            ["crate2nix", "generate"], source, output / f"{case}-generation.log"
        )
        assert digest(source / "Cargo.lock") == lock_before, (
            "Generator changed the lockfile"
        )
        if case == "lock-comment":
            # Semantically inert change isolates whether lockfile bytes leak into cache keys.
            path = source / "Cargo.lock"
            path.write_text(
                path.read_text() + "\n# Throwaway cache invalidation probe.\n"
            )
        evaluation = output / f"{case}.json"
        started = time.monotonic()
        with (
            evaluation.open("w") as stdout,
            (output / f"{case}-evaluation.log").open("w") as stderr,
        ):
            subprocess.run(
                [
                    "nix-instantiate",
                    "--eval",
                    "--strict",
                    "--json",
                    str(HERE / "evaluate.nix"),
                    "--argstr",
                    "source",
                    str(source),
                ],
                cwd=ROOT,
                stdout=stdout,
                stderr=stderr,
                check=True,
            )
        evaluation_s = round(time.monotonic() - started, 3)
        data = json.loads(evaluation.read_text())
        if case == "baseline":
            report["feature_spot_check"] = feature_spot_check(source, data, output)
        baseline = baseline or data
        report["cases"][case] = {
            "generation_s": generation_s,
            "evaluation_s": evaluation_s,
            "lock_sha256": digest(source / "Cargo.lock"),
            **compare(baseline, data),
        }
        report["system"] = data["system"]
        report["crate2nix_version"] = data["crate2nixVersion"]
        report["packaging_rust_version"] = data["packagingRustVersion"]
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        result = report["cases"][case]
        print(
            f"  Crane groups changed: {result['crane_dependency_groups_changed']}; "
            f"crate2nix crates preserved: {result['candidate_preserved']}/"
            f"{result['candidate_unique_crates']}",
            flush=True,
        )
    assert all(
        not p["candidate_changed"] and not p["crane_changed"]
        for p in report["cases"]["identical"]["packages"].values()
    )
    print(f"Report: {output / 'report.json'}", flush=True)


if __name__ == "__main__":
    main()
