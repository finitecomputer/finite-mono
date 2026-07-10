#!/usr/bin/env python3
"""Small, runtime-local Finite utility.

This intentionally owns only explicit local workflows. It is not a Core,
Runner, or Runtime Management client.
"""

from __future__ import annotations

import argparse
import ctypes
import errno
import fcntl
import hashlib
import os
import shutil
import stat
import sys
import tempfile
from pathlib import Path

DEFAULT_BUNDLED_SKILLS = Path("/runtime/finite-skills")
DEFAULT_AGENT_HOME = Path("/data/agent")
REQUIRED_SKILLS = (
    Path("software-development/finitebrain/SKILL.md"),
    Path("software-development/finite-sites-publishing-finite/SKILL.md"),
)
TEST_MODE_ENV = "FINITE_SKILLS_SYNC_TESTING"
TEST_SOURCE_ENV = "FINITE_SKILLS_SYNC_TEST_SOURCE"
TEST_AGENT_HOME_ENV = "FINITE_SKILLS_SYNC_TEST_AGENT_HOME"
TEST_FAILPOINT_ENV = "FINITE_SKILLS_SYNC_TEST_FAILPOINT"


class SyncError(RuntimeError):
    """A visible, safely handled sync failure."""


def _testing() -> bool:
    return os.environ.get(TEST_MODE_ENV) == "1"


def _paths() -> tuple[Path, Path]:
    if _testing():
        source = Path(os.environ.get(TEST_SOURCE_ENV, DEFAULT_BUNDLED_SKILLS))
        agent_home = Path(os.environ.get(TEST_AGENT_HOME_ENV, DEFAULT_AGENT_HOME))
        return source, agent_home

    test_overrides = (TEST_SOURCE_ENV, TEST_AGENT_HOME_ENV, TEST_FAILPOINT_ENV)
    leaked = [name for name in test_overrides if os.environ.get(name)]
    if leaked:
        raise SyncError(
            f"test-only skills sync overrides require {TEST_MODE_ENV}=1: {', '.join(leaked)}"
        )
    return DEFAULT_BUNDLED_SKILLS, DEFAULT_AGENT_HOME


def _failpoint(name: str) -> None:
    if _testing() and os.environ.get(TEST_FAILPOINT_ENV) == name:
        raise SyncError(f"injected test failure at {name}")


def _frontmatter(path: Path) -> tuple[str, str]:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as exc:
        raise SyncError(f"cannot read skill metadata at {path}: {exc}") from exc
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        raise SyncError(f"skill is missing YAML frontmatter: {path}")
    try:
        closing = next(
            index for index, line in enumerate(lines[1:], start=1) if line.strip() == "---"
        )
    except StopIteration as exc:
        raise SyncError(f"skill frontmatter is not closed: {path}") from exc

    fields: dict[str, str] = {}
    for line in lines[1:closing]:
        if ":" not in line or line.startswith((" ", "\t", "-")):
            continue
        key, value = line.split(":", 1)
        fields[key.strip()] = value.strip().strip("\"'")
    name = fields.get("name", "")
    description = fields.get("description", "")
    if not name or not description:
        raise SyncError(f"skill requires non-empty name and description: {path}")
    return name, description


def _regular_tree_files(root: Path) -> list[Path]:
    if not root.is_dir() or root.is_symlink():
        raise SyncError(f"bundled Finite Skills directory is unavailable: {root}")

    files: list[Path] = []
    for path in sorted(root.rglob("*")):
        try:
            mode = path.lstat().st_mode
        except OSError as exc:
            raise SyncError(f"cannot inspect bundled path {path}: {exc}") from exc
        if stat.S_ISLNK(mode):
            raise SyncError(f"bundled Finite Skills must not contain symlinks: {path}")
        if stat.S_ISDIR(mode):
            continue
        if not stat.S_ISREG(mode):
            raise SyncError(f"bundled Finite Skills contains a non-file entry: {path}")
        files.append(path)
    return files


def _validate(root: Path) -> tuple[str, int]:
    files = _regular_tree_files(root)
    skill_files = [path for path in files if path.name == "SKILL.md"]
    if not skill_files:
        raise SyncError(f"bundled Finite Skills contains no skills: {root}")

    names: dict[str, Path] = {}
    for path in skill_files:
        name, _description = _frontmatter(path)
        previous = names.get(name)
        if previous is not None:
            paths = f"{previous.relative_to(root)} and {path.relative_to(root)}"
            raise SyncError(f"duplicate skill name {name!r}: {paths}")
        names[name] = path

    for relative in REQUIRED_SKILLS:
        if not (root / relative).is_file():
            raise SyncError(f"bundled Finite Skills is missing required skill: {relative}")

    digest = hashlib.sha256()
    digest.update(b"finite-skills-tree-v1\0")
    for path in files:
        relative = path.relative_to(root).as_posix().encode("utf-8")
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        content_digest = hashlib.sha256()
        content_size = 0
        with path.open("rb") as handle:
            while chunk := handle.read(1024 * 1024):
                content_size += len(chunk)
                content_digest.update(chunk)
        digest.update(content_size.to_bytes(8, "big"))
        digest.update(content_digest.digest())
    return digest.hexdigest(), len(skill_files)


def _fsync_tree(root: Path) -> None:
    files = _regular_tree_files(root)
    for path in files:
        descriptor = os.open(path, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    directories = [path for path in root.rglob("*") if path.is_dir()]
    for path in sorted(directories, key=lambda value: len(value.parts), reverse=True):
        _fsync_directory(path)
    _fsync_directory(root)


def _fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _atomic_exchange(left: Path, right: Path) -> None:
    """Atomically exchange two same-filesystem directory entries."""

    libc = ctypes.CDLL(None, use_errno=True)
    left_bytes = os.fsencode(left)
    right_bytes = os.fsencode(right)
    result: int

    if sys.platform.startswith("linux"):
        renameat2 = getattr(libc, "renameat2", None)
        if renameat2 is None:
            raise SyncError("this Linux runtime does not provide atomic rename exchange")
        renameat2.argtypes = (
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_uint,
        )
        renameat2.restype = ctypes.c_int
        result = renameat2(  # pyright: ignore[reportAny]
            -100, left_bytes, -100, right_bytes, 2
        )
    elif sys.platform == "darwin":
        renamex_np = getattr(libc, "renamex_np", None)
        if renamex_np is None:
            raise SyncError("this macOS host does not provide atomic rename exchange")
        renamex_np.argtypes = (ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint)
        renamex_np.restype = ctypes.c_int
        result = renamex_np(  # pyright: ignore[reportAny]
            left_bytes, right_bytes, 2
        )
    else:
        raise SyncError(f"atomic skills replacement is unsupported on {sys.platform}")

    if result != 0:
        error_number = ctypes.get_errno()
        if error_number in (errno.ENOSYS, errno.EOPNOTSUPP, errno.EXDEV):
            raise SyncError("the managed-skills filesystem does not support atomic replacement")
        raise OSError(error_number, os.strerror(error_number), str(left), str(right))


def _remove_staging(staging_root: Path) -> None:
    try:
        shutil.rmtree(staging_root)
    except FileNotFoundError:
        return


def sync_skills() -> tuple[str, int]:
    source, agent_home = _paths()
    managed_root = agent_home / "managed-skills"
    managed_parent = managed_root / "finite"
    current = managed_parent / "current"
    for path in (managed_root, managed_parent):
        if os.path.lexists(path) and (path.is_symlink() or not path.is_dir()):
            raise SyncError(f"managed-skills path is not a directory: {path}")
        path.mkdir(parents=True, exist_ok=True)

    lock_path = managed_parent / ".sync.lock"
    lock_flags = os.O_CREAT | os.O_RDWR
    if hasattr(os, "O_NOFOLLOW"):
        lock_flags |= os.O_NOFOLLOW
    lock_descriptor = os.open(lock_path, lock_flags, 0o600)
    with os.fdopen(lock_descriptor, "a", encoding="utf-8") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)

        current_exists = os.path.lexists(current)
        if current_exists and (not current.is_dir() or current.is_symlink()):
            raise SyncError(f"managed baseline is not a directory: {current}")

        # Validate the immutable image source before copying, then validate the
        # exact staged bytes that will become active.
        source_digest, source_skill_count = _validate(source)
        staging_root = Path(tempfile.mkdtemp(prefix=".skills-sync-", dir=managed_parent))
        staged = staging_root / "baseline"
        swapped_existing = False
        installed_new = False
        preserve_staging = False
        try:
            _ = shutil.copytree(source, staged, symlinks=True)
            digest, skill_count = _validate(staged)
            if (digest, skill_count) != (source_digest, source_skill_count):
                raise SyncError("staged Finite Skills does not match the Runtime image bundle")
            _fsync_tree(staged)
            _fsync_directory(staging_root)
            _fsync_directory(managed_parent)
            _failpoint("before_swap")

            if current_exists:
                _atomic_exchange(current, staged)
                swapped_existing = True
            else:
                os.replace(staged, current)
                installed_new = True

            try:
                _failpoint("after_swap")
                _fsync_directory(managed_parent)
            except BaseException:
                try:
                    if swapped_existing:
                        _atomic_exchange(current, staged)
                        _fsync_directory(managed_parent)
                    elif installed_new:
                        os.replace(current, staged)
                        _fsync_directory(managed_parent)
                except BaseException as rollback_error:
                    preserve_staging = True
                    detail = f"staging was retained at {staging_root}"
                    raise SyncError(
                        f"skills sync rollback could not be durably confirmed; {detail}"
                    ) from rollback_error
                raise
        except BaseException:
            if not preserve_staging:
                _remove_staging(staging_root)
            raise

        # Cleanup is deliberately after the committed swap. A cleanup failure
        # cannot make the new baseline partial, so it is not a sync failure.
        try:
            _remove_staging(staging_root)
        except OSError as exc:
            print(
                f"warning: synced skills but could not remove prior staging: {exc}",
                file=sys.stderr,
            )
        return digest, skill_count


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="finite",
        description="Explicit local workflows for this Finite agent runtime.",
    )
    commands = parser.add_subparsers(dest="command", required=True)
    skills = commands.add_parser("skills", help="manage Finite-managed skills")
    skills_commands = skills.add_subparsers(dest="skills_command", required=True)
    _ = skills_commands.add_parser(
        "sync", help="adopt the tested Finite Skills bundle in this runtime image"
    )
    return parser


def main() -> int:
    _ = _parser().parse_args()
    try:
        digest, skill_count = sync_skills()
    except (OSError, SyncError) as exc:
        print(f"finite skills sync failed: {exc}", file=sys.stderr)
        return 1
    print(f"Finite Skills synced from this Runtime image: {skill_count} skills (sha256:{digest}).")
    print("New skill names appear after Hermes /reload-skills; no Runtime reboot is needed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
