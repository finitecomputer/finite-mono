#!/usr/bin/env python3
"""Validate backup credentials and save daemon arguments for fixed Supervisor jobs."""

import json
import os
from pathlib import Path
import stat
import sys
import tempfile


ROOT_UID = 0
SUPERVISOR_CONFIG = Path("/etc/sites-supervisor.conf")


def private_write(path, contents):
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as output:
            output.write(contents)
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def configure(
    argv,
    environment,
    *,
    run_dir=Path("/run"),
):
    if os.geteuid() != ROOT_UID:
        raise ValueError("Sites backup supervision requires root")
    repository = environment.get("FINITE_SITES_BACKUP_REPOSITORY", "")
    if not repository.strip():
        raise ValueError("FINITE_SITES_BACKUP_REPOSITORY is required")
    credentials = Path(
        environment.get(
            "FINITE_SITES_BACKUP_CREDENTIALS_DIR",
            "/var/lib/finitecomputer/backups/rsync-net",
        )
    )
    try:
        info = credentials.lstat()
    except OSError:
        raise ValueError(
            "Sites backup requires an existing root-only credential directory"
        ) from None
    if (
        not credentials.is_absolute()
        or not stat.S_ISDIR(info.st_mode)
        or info.st_uid != ROOT_UID
        or info.st_mode & 0o077
    ):
        raise ValueError(
            "Sites backup credential directory must be root-owned and root-only"
        )
    for name in ("id_ed25519", "known_hosts", "borg-passphrase"):
        path = credentials / name
        info = path.lstat()
        if not stat.S_ISREG(info.st_mode) or info.st_uid != ROOT_UID:
            raise ValueError(
                "Sites backup credentials must be root-owned regular files"
            )
        # Fly mounts these inside the private directory; tighten file modes
        # before starting either the daemon or the backup job.
        path.chmod(0o600)

    # Match finitesitesd's --flag value pairs without changing its original argv.
    data = []
    index = 1
    while index < len(argv):
        if argv[index].startswith("--"):
            if index + 1 >= len(argv):
                raise ValueError("Sites serve flags require values")
            if argv[index] == "--data":
                data.append(argv[index + 1])
            index += 2
        else:
            index += 1
    if not argv or argv[0] != "serve" or len(data) != 1 or not data[0]:
        raise ValueError("Sites backup requires exactly one --data DIR")

    config_path = SUPERVISOR_CONFIG
    backup_path = run_dir / "sites-backup.json"
    config = {
        "data": data[0],
        "root": environment.get(
            "FINITE_SITES_BACKUP_ROOT", "/var/backups/finite-sites"
        ),
        "repository": repository,
        "remote_path": environment.get("FINITE_SITES_BACKUP_REMOTE_PATH", "borg12"),
        "credentials_dir": str(credentials),
        "control": ["supervisorctl", "-c", str(config_path)],
        "service": "sites",
    }
    # Keep arbitrary daemon arguments out of Supervisor's command parser.
    private_write(run_dir / "sites-argv.json", json.dumps(argv) + "\n")
    private_write(backup_path, json.dumps(config, indent=2) + "\n")
    return config_path


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv and argv[0] == "exec-sites":
        original = json.loads(Path(argv[1]).read_text())
        os.execvp(
            "setpriv",
            [
                "setpriv",
                "--reuid=65532",
                "--regid=65532",
                "--clear-groups",
                "--no-new-privs",
                "finitesitesd",
                *original,
            ],
        )
        return
    os.umask(0o077)
    config_path = configure(argv, os.environ)
    os.execvp("supervisord", ["supervisord", "-n", "-c", str(config_path)])


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError):
        # Do not echo configuration or environment values into image logs.
        print(
            "Sites backup supervisor configuration failed; check required paths and permissions",
            file=sys.stderr,
        )
        sys.exit(1)
