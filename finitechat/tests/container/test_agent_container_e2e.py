"""Container test helpers shared by Docker runtime smokes."""

from __future__ import annotations

import contextlib
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]


def run(args, *, timeout=600, check=True, **kwargs):
    result = subprocess.run(
        args, capture_output=True, text=True, timeout=timeout, check=False, **kwargs
    )
    if check and result.returncode != 0:
        raise AssertionError(
            f"command failed with exit {result.returncode}: {args!r}\n"
            f"stdout:\n{result.stdout[-4000:]}\n"
            f"stderr:\n{result.stderr[-4000:]}"
        )
    return result


def stage_build_context(ctx: Path) -> None:
    ctx.mkdir(parents=True, exist_ok=True)
    for name, source in (("finitechat", REPO_ROOT),):
        run(
            [
                "rsync",
                "-a",
                "--exclude",
                ".git",
                "--exclude",
                "target",
                "--exclude",
                "__pycache__",
                "--exclude",
                ".DS_Store",
                "--exclude",
                ".env",
                "--exclude",
                ".env.*",
                "--exclude",
                ".finitechat",
                "--exclude",
                ".state",
                "--exclude",
                "secrets",
                f"{source}/",
                str(ctx / name),
            ]
        )


class BuildContextStagingTest(unittest.TestCase):
    def test_stage_build_context_excludes_local_state_and_secrets(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source_files = [
                REPO_ROOT / ".finitechat/canary-secret",
                REPO_ROOT / ".state/canary-secret",
                REPO_ROOT / "secrets/canary-secret",
            ]
            for path in source_files:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("do-not-copy", encoding="utf-8")
            try:
                ctx = Path(tmp) / "ctx"
                stage_build_context(ctx)
                self.assertFalse((ctx / "finitechat/.finitechat/canary-secret").exists())
                self.assertFalse((ctx / "finitechat/.state/canary-secret").exists())
                self.assertFalse((ctx / "finitechat/secrets/canary-secret").exists())
            finally:
                for path in source_files:
                    path.unlink(missing_ok=True)
                for dirname in ("secrets", ".state"):
                    with contextlib.suppress(OSError):
                        (REPO_ROOT / dirname).rmdir()
