#!/usr/bin/env python3
"""Build a static CodeCharta output; dependencies come from .#code-map."""

import argparse
import base64
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

VERSION = "2.2.0"
INTEGRITY = "HeXkuUDsGZMqK5HWh6duEo5wpYl6Ub48bNz4cdzc90I0MBkCP/Ld2zwjfq7lHwtv/UhCFFxGSSPQcp201KEe3w=="
ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=ROOT)
    parser.add_argument("--ref", default="HEAD")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    if output.exists():
        parser.error("output already exists; choose a fresh build directory")
    with tempfile.TemporaryDirectory(prefix="finite-code-map-build-") as scratch:
        temp = Path(scratch)
        site = temp / "site"
        site.mkdir()
        subprocess.run(
            [
                sys.executable,
                str(ROOT / "infra/code-map/metrics.py"),
                str(args.repo.resolve()),
                "--ref",
                args.ref,
                "--output",
                str(site),
            ],
            check=True,
        )
        url = f"https://registry.npmjs.org/codecharta-visualization/-/codecharta-visualization-{VERSION}.tgz"
        with urllib.request.urlopen(url, timeout=60) as response:
            archive = response.read()
        if base64.b64encode(hashlib.sha512(archive).digest()).decode() != INTEGRITY:
            raise RuntimeError("CodeCharta package integrity mismatch")
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
            tar.extractall(temp / "vendor", filter="data")
        package = temp / "vendor/package"
        shutil.copytree(package / "dist/bundler/browser", site / "viewer")
        shutil.copyfile(package / "LICENSE.md", site / "CODECHARTA-LICENSE.md")
        # Root-relative metric URLs keep the same artifact usable locally and hosted.
        (site / "index.html").write_text("""<!doctype html>
<meta charset="utf-8">
<title>Finite code map</title>
<meta http-equiv="refresh" content="0;url=viewer/?file=/finite-mono.cc.json&amp;area=rloc&amp;height=complexity_estimate&amp;color=touches_30d">
<a href="viewer/?file=/finite-mono.cc.json&amp;area=rloc&amp;height=complexity_estimate&amp;color=touches_30d">Open the Finite code map</a>
""")
        summary = json.loads((site / "summary.json").read_text())
        (site / "build.json").write_text(
            json.dumps(
                {
                    "source_commit": summary["commit"],
                    "snapshot_time": summary["snapshot_time"],
                    "history_since": summary["history_since"],
                    "viewer_version": VERSION,
                    "scanner_version": summary["scc_version"],
                },
                indent=2,
            )
            + "\n"
        )
        shutil.copytree(site, output)
    print(f"Static output: {output}")


if __name__ == "__main__":
    main()
