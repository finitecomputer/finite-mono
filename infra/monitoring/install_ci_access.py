#!/usr/bin/env python3
"""Operator-only installation of restricted dashboard CI access on both hosts."""

import argparse
import json
from pathlib import Path
import shlex
import subprocess


# Runs through the operator's existing SSH access, never through the CI key.
# Key bytes travel on stdin and are never logged or committed.
INSTALL = r"""
import fcntl, json, os, pathlib, pwd, shutil, sys, tempfile
p = json.load(sys.stdin)
assert os.geteuid() == 0
with open('/run/lock/finite-monitoring-dashboards.lock', 'w') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    account = pwd.getpwnam(p['user'])
    sshdir = pathlib.Path(account.pw_dir) / '.ssh'
    assert not sshdir.is_symlink()
    sshdir.mkdir(mode=0o700, exist_ok=True)
    os.chown(sshdir, account.pw_uid, account.pw_gid)
    auth = sshdir / 'authorized_keys'
    assert not auth.is_symlink()
    root = pathlib.Path('/var/lib/finite-monitoring-ci')
    assert not root.is_symlink()
    root.mkdir(mode=0o755, exist_ok=True)
    (root / 'scripts').mkdir(mode=0o755, exist_ok=True)
    assert not (root / 'scripts').is_symlink()
    backup_root = pathlib.Path('/var/backups')
    assert not backup_root.is_symlink()
    backup_root.mkdir(mode=0o755, exist_ok=True)
    backup = pathlib.Path(tempfile.mkdtemp(prefix='finite-monitoring-ci.', dir=backup_root))
    previous = auth.read_text() if auth.exists() else ''
    (backup / 'authorized_keys').write_text(previous)
    (backup / 'authorized_keys.state').write_text('present' if auth.exists() else 'absent')
    def replace(path, content, mode, uid=0, gid=0):
        assert not path.is_symlink()
        if path.exists():
            shutil.copy2(path, backup / path.name)
        fd, name = tempfile.mkstemp(prefix='.ci-', dir=path.parent)
        try:
            with os.fdopen(fd, 'w') as out:
                out.write(content)
                out.flush()
                os.fsync(out.fileno())
                os.fchmod(out.fileno(), mode)
                os.fchown(out.fileno(), uid, gid)
            os.replace(name, path)
        finally:
            pathlib.Path(name).unlink(missing_ok=True)
    # Install code before granting key access. Retain unrelated operator keys.
    for name, content in p['files'].items():
        path = root / name
        assert path.parent in (root, root / 'scripts')
        replace(path, content, 0o755 if name == 'ci_dispatch' else 0o644)
    lines = [line for line in previous.splitlines() if not line.endswith(' finite-monitoring-ci')]
    lines.append(p['authorized_key'])
    replace(auth, '\n'.join(lines) + '\n', 0o600, account.pw_uid, account.pw_gid)
    print('Restricted CI access installed; previous files: ' + str(backup))
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("public_key", type=Path)
    options = parser.parse_args()
    subprocess.run(
        ["ssh-keygen", "-lf", str(options.public_key)],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    fields = options.public_key.read_text().strip().split()
    if len(fields) != 3 or fields[0] != "ssh-ed25519":
        raise SystemExit("expected an Ed25519 public key")
    root = Path(__file__).resolve().parents[2]
    monitoring = root / "infra/monitoring"
    targets = [
        (
            "ubuntu@152.236.5.27",
            "ubuntu",
            "/var/lib/finite-monitoring-ci/ci_dispatch",
            {
                name: (monitoring / name).read_text()
                for name in ("ci_dispatch", "deploy_dashboards.py")
            },
        ),
        (
            "root@64.34.80.19",
            "root",
            "/run/current-system/sw/bin/python3 /var/lib/finite-monitoring-ci/scripts/finite-status --json",
            {
                "scripts/" + name: (root / "scripts" / name).read_text()
                for name in ("finite-status", "finite_status.py")
            },
        ),
    ]
    for target, user, command, files in targets:
        line = (
            f'restrict,command="{command}" {fields[0]} {fields[1]} finite-monitoring-ci'
        )
        payload = {"user": user, "authorized_key": line, "files": files}
        subprocess.run(
            [
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=yes",
                target,
                "sudo -n python3 -c " + shlex.quote(INSTALL),
            ],
            input=json.dumps(payload),
            text=True,
            check=True,
        )


if __name__ == "__main__":
    main()
