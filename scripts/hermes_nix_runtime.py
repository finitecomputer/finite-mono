#!/usr/bin/env python3
"""Helpers for staging pinned Nix runtimes into Agent Runtime image contexts."""

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
TOOLCHAIN_ATTR = ".#packages.{system}.agent-runtime-toolchains"


@dataclass(frozen=True)
class HermesRuntimeClosure:
    attr: str
    python_attr: str
    toolchain_attr: str
    nix_system: str
    store_path: str
    python_store_path: str
    toolchain_store_path: str
    playwright_browsers_path: str
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


def _rmtree_readonly(root: Path) -> None:
    """Delete a staged Nix store copy whose files and directories are 0555.

    Both the files and their containing directories must gain write bits
    before unlink/rmdir can succeed; chmod bottom-up, then remove.
    """

    def _chmod(target: str, mode: int) -> None:
        try:
            os.chmod(target, mode)
        except OSError:
            pass

    for dirpath, dirnames, filenames in os.walk(root, topdown=False):
        for name in filenames:
            _chmod(os.path.join(dirpath, name), 0o700)
        for name in dirnames:
            _chmod(os.path.join(dirpath, name), 0o700)
    _chmod(str(root), 0o700)
    shutil.rmtree(root)


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


def toolchain_attr(system: str) -> str:
    return TOOLCHAIN_ATTR.format(system=system)


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


def eval_playwright_browsers_path(repo_root: Path, attr: str) -> str:
    path = run(["nix", "eval", "--raw", f"{attr}.browsersPath"], cwd=repo_root).stdout.strip()
    if not path.startswith("/nix/store/"):
        raise SystemExit(f"Nix did not print a Playwright browsers store path for {attr}")
    return path


def recursive_store_paths(repo_root: Path, store_path: str, *, timeout: int) -> list[str]:
    closure = run(
        ["nix", "path-info", "--recursive", store_path],
        cwd=repo_root,
        timeout=timeout,
    ).stdout.splitlines()
    paths = [path.strip() for path in closure if path.startswith("/nix/store/")]
    if not paths:
        raise SystemExit(f"Nix closure for {store_path} was empty")
    return paths


def stage_store_paths(
    repo_root: Path,
    context: Path,
    store_paths: list[str],
    *,
    timeout: int,
) -> None:
    store_context = context / HERMES_NIX_CONTEXT_DIR
    if store_context.exists():
        _rmtree_readonly(store_context)
    store_root = store_context / "nix" / "store"
    store_root.mkdir(parents=True, exist_ok=True)
    seen: set[str] = set()
    for path in store_paths:
        if path in seen:
            continue
        seen.add(path)
        run(["rsync", "-a", path, f"{store_root}/"], cwd=repo_root, timeout=timeout, capture=False)


def image_build_args(runtime: HermesRuntimeClosure, *, hermes_agent_version: str) -> list[str]:
    pairs = (
        ("HERMES_AGENT_VERSION", hermes_agent_version),
        ("HERMES_AGENT_STORE_PATH", runtime.store_path),
        ("HERMES_AGENT_PYTHON_PATH", runtime.python_store_path),
        ("HERMES_AGENT_NIX_ATTR", runtime.attr),
        ("HERMES_AGENT_NIX_SYSTEM", runtime.nix_system),
        ("AGENT_RUNTIME_TOOLCHAIN_PATH", runtime.toolchain_store_path),
        ("AGENT_RUNTIME_TOOLCHAIN_ATTR", runtime.toolchain_attr),
        ("PLAYWRIGHT_BROWSERS_PATH", runtime.playwright_browsers_path),
    )
    args: list[str] = []
    for name, value in pairs:
        if not value:
            raise SystemExit(f"missing runtime image build-arg {name}")
        args.extend(["--build-arg", f"{name}={value}"])
    return args


def stage_runtime_closure(
    repo_root: Path,
    context: Path,
    *,
    system: str,
    timeout: int = 7200,
) -> HermesRuntimeClosure:
    attr = runtime_attr(system)
    python_attr = runtime_python_attr(system)
    tools_attr = toolchain_attr(system)
    version = eval_runtime_version(repo_root, system)
    store_path = build_attr(repo_root, attr, timeout=timeout)
    python_store_path = build_attr(repo_root, python_attr, timeout=timeout)
    toolchain_store_path = build_attr(repo_root, tools_attr, timeout=timeout)
    playwright_browsers_path = eval_playwright_browsers_path(repo_root, tools_attr)

    hermes_paths = recursive_store_paths(repo_root, store_path, timeout=timeout)
    if python_store_path not in hermes_paths:
        raise SystemExit(
            f"Nix closure for {store_path} did not include Hermes Python runtime {python_store_path}"
        )
    toolchain_paths = recursive_store_paths(repo_root, toolchain_store_path, timeout=timeout)
    if playwright_browsers_path not in toolchain_paths:
        toolchain_paths = toolchain_paths + recursive_store_paths(
            repo_root, playwright_browsers_path, timeout=timeout
        )

    staged_paths = hermes_paths + toolchain_paths
    stage_store_paths(repo_root, context, staged_paths, timeout=timeout)

    return HermesRuntimeClosure(
        attr=attr,
        python_attr=python_attr,
        toolchain_attr=tools_attr,
        nix_system=system,
        store_path=store_path,
        python_store_path=python_store_path,
        toolchain_store_path=toolchain_store_path,
        playwright_browsers_path=playwright_browsers_path,
        version=version,
        closure_count=len({path for path in staged_paths}),
    )


def _unsupported_platform(platform: str) -> str:
    raise SystemExit(f"unsupported Hermes runtime image platform: {platform}")
