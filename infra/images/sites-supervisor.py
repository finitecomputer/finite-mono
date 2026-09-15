#!/usr/bin/env python3
"""Configure the opt-in Sites image Supervisor and daily cron backup."""

import base64
import json
import os
from pathlib import Path
import shlex
import stat
import sys
import tempfile


ROOT_UID = 0
PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"


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
    cron_path=Path("/etc/cron.d/sites-backup"),
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

    config_path = run_dir / "sites-supervisor.conf"
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
    # Supervisor interpolates percent signs and truncates semicolon comments even
    # inside quotes. Only a base64 JSON argument crosses its command parser.
    encoded = base64.b64encode(json.dumps(argv).encode()).decode("ascii")

    def command(args):
        return shlex.join(args).replace("%", "%%")

    clean_environment = ["/usr/bin/env", "-i", f"PATH={PATH}", "HOME=/root", "TZ=UTC"]
    programs = [
        (
            "sites",
            [
                "/usr/bin/python3",
                "/usr/local/bin/sites-supervisor.py",
                "exec-sites",
                encoded,
            ],
            100,
            "true",
            "true",
            "INT",
            30,
            1,
        ),
        (
            "sites-backup",
            [
                *clean_environment,
                "/usr/local/bin/sites-backup",
                "run",
                "--config",
                str(backup_path),
            ],
            200,
            "false",
            "false",
            "TERM",
            60,
            0,
        ),
        (
            "cron",
            [*clean_environment, "/usr/sbin/cron", "-f"],
            300,
            "true",
            "true",
            "TERM",
            10,
            1,
        ),
    ]
    socket_path = str(run_dir / "sites-supervisor.sock").replace("%", "%%")
    supervisor_config = f"""[unix_http_server]
file={socket_path}
chmod=0600
chown=0:0

[supervisord]
nodaemon=true
user=0
umask=0077
logfile=/dev/stdout
logfile_maxbytes=0
pidfile={str(run_dir / "sites-supervisor.pid").replace("%", "%%")}
childlogdir={str(run_dir).replace("%", "%%")}

[rpcinterface:supervisor]
supervisor.rpcinterface_factory=supervisor.rpcinterface:make_main_rpcinterface

[supervisorctl]
serverurl=unix://{socket_path}
"""
    for name, args, priority, autostart, restart, stop, timeout, startsecs in programs:
        supervisor_config += f"""
[program:{name}]
command={command(args)}
priority={priority}
autostart={autostart}
autorestart={restart}
startsecs={startsecs}
stopsignal={stop}
stopwaitsecs={timeout}
stopasgroup=true
killasgroup=true
stdout_logfile=/dev/stdout
stdout_logfile_maxbytes=0
stderr_logfile=/dev/stderr
stderr_logfile_maxbytes=0
"""
    # Cron starts a Supervisor-owned one-shot, so shutdown also owns any active
    # backup process. Higher priorities stop first: cron, then backup, then Sites.
    cron_command = shlex.join(
        ["/usr/bin/supervisorctl", "-c", str(config_path), "start", "sites-backup"]
    ).replace("%", r"\%")
    cron_config = (
        f'SHELL=/bin/sh\nPATH={PATH}\nMAILTO=""\n'
        f"7 3 * * * root {cron_command} >/proc/1/fd/1 2>/proc/1/fd/2\n"
    )
    private_write(backup_path, json.dumps(config, indent=2) + "\n")
    private_write(config_path, supervisor_config)
    private_write(cron_path, cron_config)
    return config_path


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv and argv[0] == "exec-sites":
        original = json.loads(base64.b64decode(argv[1], validate=True))
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
