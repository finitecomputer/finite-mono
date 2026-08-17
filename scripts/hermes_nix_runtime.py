#!/usr/bin/env python3
"""Helpers for staging the pinned Nix-built Hermes runtime into image contexts."""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
from dataclasses import dataclass
from pathlib import Path

HERMES_NIX_CONTEXT_DIR = ".finite-hermes-nix-store"
HERMES_PACKAGE_ATTR = ".#packages.{system}.hermes-agent"
HERMES_RUNTIME_ATTR = ".#packages.{system}.hermes-agent-runtime"
HERMES_RUNTIME_PYTHON_ATTR = ".#packages.{system}.hermes-agent-runtime-python"


@dataclass(frozen=True)
class HermesRuntimeClosure:
    attr: str
    python_attr: str
    nix_system: str
    store_path: str
    python_store_path: str
    version: str
    closure_count: int


def run(
    args: list[str],
    *,
    cwd: Path,
    timeout: int = 3600,
    capture: bool = True,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=cwd,
        capture_output=capture,
        text=True,
        check=True,
        timeout=timeout,
    )


def nix_system_for_platform(platform: str) -> str:
    parts = platform.split("/")
    if len(parts) < 2 or parts[0] != "linux":
        raise SystemExit(f"unsupported Hermes runtime image platform: {platform}")
    return {
        "amd64": "x86_64-linux",
        "x86_64": "x86_64-linux",
        "arm64": "aarch64-linux",
        "aarch64": "aarch64-linux",
    }.get(parts[1]) or _unsupported_platform(platform)


def native_nix_system() -> str:
    return run(
        ["nix", "eval", "--raw", "--impure", "--expr", "builtins.currentSystem"], cwd=Path.cwd()
    ).stdout.strip()


def runtime_attr(system: str) -> str:
    return HERMES_RUNTIME_ATTR.format(system=system)


def runtime_python_attr(system: str) -> str:
    return HERMES_RUNTIME_PYTHON_ATTR.format(system=system)


def package_attr(system: str) -> str:
    return HERMES_PACKAGE_ATTR.format(system=system)


def build_attr(repo_root: Path, attr: str, *, timeout: int = 7200) -> str:
    try:
        result = run(
            ["nix", "build", "--no-link", "--print-out-paths", attr],
            cwd=repo_root,
            timeout=timeout,
        )
    except subprocess.CalledProcessError as exc:
        raise SystemExit(
            f"Nix build failed for {attr} with exit code {exc.returncode}\n"
            f"stdout:\n{exc.stdout or ''}\n"
            f"stderr:\n{exc.stderr or ''}"
        ) from exc
    paths = [line.strip() for line in result.stdout.splitlines() if line.startswith("/nix/store/")]
    if not paths:
        raise SystemExit(f"Nix did not print a store path for {attr}")
    return paths[-1]


def eval_runtime_version(repo_root: Path, system: str) -> str:
    return run(
        ["nix", "eval", "--raw", f"{package_attr(system)}.version"], cwd=repo_root
    ).stdout.strip()


def stage_runtime_closure(
    repo_root: Path,
    context: Path,
    *,
    system: str,
    timeout: int = 7200,
) -> HermesRuntimeClosure:
    attr = runtime_attr(system)
    python_attr = runtime_python_attr(system)
    version = eval_runtime_version(repo_root, system)
    store_path = build_attr(repo_root, attr, timeout=timeout)
    python_store_path = build_attr(repo_root, python_attr, timeout=timeout)
    closure = run(
        ["nix", "path-info", "--recursive", store_path],
        cwd=repo_root,
        timeout=timeout,
    ).stdout.splitlines()
    closure_paths = [path.strip() for path in closure if path.startswith("/nix/store/")]
    if not closure_paths:
        raise SystemExit(f"Nix closure for {store_path} was empty")
    if python_store_path not in closure_paths:
        raise SystemExit(
            f"Nix closure for {store_path} did not include Hermes Python runtime {python_store_path}"
        )

    store_context = context / HERMES_NIX_CONTEXT_DIR
    if store_context.exists():
        # Staged Nix store files are read-only; make them deletable first.
        def _make_writable(func, target, _error):
            os.chmod(target, 0o700 | stat.S_IMODE(os.lstat(target).st_mode))
            func(target)

        shutil.rmtree(store_context, onexc=_make_writable)
    store_root = store_context / "nix" / "store"
    store_root.mkdir(parents=True, exist_ok=True)

    for path in closure_paths:
        run(["rsync", "-a", path, f"{store_root}/"], cwd=repo_root, timeout=timeout, capture=False)

    return HermesRuntimeClosure(
        attr=attr,
        python_attr=python_attr,
        nix_system=system,
        store_path=store_path,
        python_store_path=python_store_path,
        version=version,
        closure_count=len(closure_paths),
    )


def _unsupported_platform(platform: str) -> str:
    raise SystemExit(f"unsupported Hermes runtime image platform: {platform}")
