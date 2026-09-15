"""Sites image launch contract, with optional real Linux Supervisor coverage."""

import configparser
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / "infra/images/sites-supervisor.py"
ENTRYPOINT = ROOT / "infra/images/sites-entrypoint"


def load_helper():
    spec = importlib.util.spec_from_file_location("sites_supervisor", HELPER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class EntrypointTests(unittest.TestCase):
    def test_default_and_nonserve_keep_exec_contract(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for name in ("setpriv", "python3", "mountpoint", "chown", "chmod"):
                stub = directory / name
                stub.write_text(
                    "#!/bin/sh\n"
                    '[ -z "${FINITE_SITES_BORG_SSH_KEY:-}${FINITE_SITES_BORG_KNOWN_HOSTS:-}${FINITE_SITES_BORG_PASSPHRASE:-}" ] || exit 99\n'
                    'printf "%s\\n" "$(basename "$0")" "$@" >> "$CALLS"\n'
                    'if [ "$(basename "$0")" = mountpoint ]; then exit "${MOUNT_RESULT:-0}"; fi\n'
                )
                stub.chmod(0o755)
            cases = [
                (["--help"], "1", "setpriv"),
                (["serve", "--data", "/var/lib/finite-sites"], "0", "setpriv"),
                (["serve", "--data", "/var/lib/finite-sites"], "true", "setpriv"),
                (["serve", "--data", "/var/lib/finite-sites"], "1", "python3"),
            ]
            for args, enabled, launcher in cases:
                with self.subTest(args=args, enabled=enabled):
                    calls = directory / "calls"
                    calls.unlink(missing_ok=True)
                    env = dict(
                        os.environ,
                        PATH=f"{directory}:/usr/bin:/bin",
                        CALLS=str(calls),
                        FINITE_SITES_BACKUP_ENABLED=enabled,
                        FINITE_SITES_BORG_SSH_KEY="synthetic-key",
                        FINITE_SITES_BORG_KNOWN_HOSTS="synthetic-host",
                        FINITE_SITES_BORG_PASSPHRASE="synthetic-passphrase",
                    )
                    result = subprocess.run(
                        ["sh", str(ENTRYPOINT), *args],
                        env=env,
                        capture_output=True,
                        text=True,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    invoked = calls.read_text().splitlines()
                    self.assertIn(launcher, invoked)
                    self.assertEqual(invoked[-len(args) :], args)
                    if launcher == "setpriv":
                        self.assertNotIn("python3", invoked)
                        for flag in (
                            "--reuid=65532",
                            "--regid=65532",
                            "--clear-groups",
                            "--no-new-privs",
                            "finitesitesd",
                        ):
                            self.assertIn(flag, invoked)
                    if args[0] != "serve":
                        self.assertNotIn("mountpoint", invoked)
            env["MOUNT_RESULT"] = "1"
            calls.unlink()
            result = subprocess.run(
                ["sh", str(ENTRYPOINT), "serve"],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("setpriv", calls.read_text())
            self.assertNotIn("python3", calls.read_text())


class ConfigurationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.credentials = self.directory / "credentials"
        self.credentials.mkdir(mode=0o700)
        for name in ("id_ed25519", "known_hosts", "borg-passphrase"):
            (self.credentials / name).write_text("synthetic-unused")
        self.run = self.directory / "run"
        self.run.mkdir()
        self.cron = self.directory / "sites-backup.cron"
        self.env = {
            "FINITE_SITES_BACKUP_REPOSITORY": "backup@example.test:sites",
            "FINITE_SITES_BACKUP_CREDENTIALS_DIR": str(self.credentials),
            "SYNTHETIC_TOKEN": "must-not-be-in-config",
        }
        self.module = load_helper()

    def configure(self, args=None):
        # Production requires root; local configuration tests use the current owner.
        with patch.object(self.module, "ROOT_UID", os.getuid()):
            return self.module.configure(
                args or ["serve", "--data", "/var/lib/finite-sites", "--mailer", "dev"],
                self.env,
                run_dir=self.run,
                cron_path=self.cron,
            )

    def test_private_config_and_clean_scheduled_backup(self):
        path = self.configure()
        config = json.loads((self.run / "sites-backup.json").read_text())
        self.assertEqual(
            config,
            {
                "data": "/var/lib/finite-sites",
                "root": "/var/backups/finite-sites",
                "repository": "backup@example.test:sites",
                "remote_path": "borg12",
                "credentials_dir": str(self.credentials),
                "control": ["supervisorctl", "-c", str(path)],
                "service": "sites",
            },
        )
        for file in (path, self.run / "sites-backup.json", self.cron):
            self.assertEqual(stat.S_IMODE(file.stat().st_mode), 0o600)
            self.assertNotIn("must-not-be-in-config", file.read_text())
        conf = configparser.ConfigParser(interpolation=None)
        conf.read(path)
        self.assertEqual(conf["unix_http_server"]["chmod"], "0600")
        self.assertNotIn("inet_http_server", conf)
        sites, backup, cron = [
            conf[f"program:{name}"] for name in ("sites", "sites-backup", "cron")
        ]
        self.assertLess(int(sites["priority"]), int(backup["priority"]))
        self.assertLess(int(backup["priority"]), int(cron["priority"]))
        self.assertEqual(sites["stopsignal"], "INT")
        self.assertLess(int(sites["stopwaitsecs"]), 45)
        for program in (sites, backup, cron):
            self.assertEqual(program["stopasgroup"], "true")
            self.assertEqual(program["killasgroup"], "true")
            self.assertEqual(program["stdout_logfile"], "/dev/stdout")
            self.assertEqual(program["stderr_logfile"], "/dev/stderr")
        self.assertEqual(backup["autostart"], "false")
        self.assertEqual(backup["autorestart"], "false")
        self.assertEqual(backup["startsecs"], "0")
        self.assertIn("/usr/bin/env -i ", cron["command"])
        self.assertIn("TZ=UTC", cron["command"])
        self.assertIn("/usr/bin/env -i ", backup["command"])
        self.assertIn("/usr/local/bin/sites-backup run --config ", backup["command"])
        self.assertIn("7 3 * * * root ", self.cron.read_text())
        self.assertIn("start sites-backup", self.cron.read_text())

    def test_arguments_survive_supervisor_and_privilege_drop(self):
        args = [
            "serve",
            "--data",
            "/var/lib/finite-sites",
            "--mail-from",
            "A 'quoted' \"name\" ; 100% %(ENV_SECRET)s $HOME `id`\nnext",
            "--empty",
            "",
        ]
        path = self.configure(args)
        conf = configparser.ConfigParser()
        conf.read(path)
        command = shlex.split(conf["program:sites"]["command"])
        self.assertEqual(
            command[:3],
            ["/usr/bin/python3", "/usr/local/bin/sites-supervisor.py", "exec-sites"],
        )
        with patch.object(self.module.os, "execvp") as execute:
            self.module.main(command[2:])
        execute.assert_called_once_with(
            "setpriv",
            [
                "setpriv",
                "--reuid=65532",
                "--regid=65532",
                "--clear-groups",
                "--no-new-privs",
                "finitesitesd",
                *args,
            ],
        )

    def test_invalid_backup_settings_fail_before_writing_configuration(self):
        for name, value in (
            ("FINITE_SITES_BACKUP_REPOSITORY", ""),
            ("FINITE_SITES_BACKUP_CREDENTIALS_DIR", "/does-not-exist"),
        ):
            with self.subTest(name=name), patch.dict(self.env, {name: value}):
                with self.assertRaises(ValueError):
                    self.configure()
                self.assertFalse((self.run / "sites-backup.json").exists())
        self.credentials.chmod(0o755)
        with self.assertRaises(ValueError):
            self.configure()
        self.credentials.chmod(0o700)
        linked = self.directory / "linked"
        linked.symlink_to(self.credentials)
        self.env["FINITE_SITES_BACKUP_CREDENTIALS_DIR"] = str(linked)
        with self.assertRaises(ValueError):
            self.configure()

    def test_data_is_required_and_ambiguous_data_fails_closed(self):
        for args in (
            ["serve"],
            ["serve", "--data"],
            ["serve", "--data", "/a", "--data", "/b"],
        ):
            with self.subTest(args=args), self.assertRaises(ValueError):
                self.configure(args)

    def test_credential_contents_are_never_read(self):
        secret = self.credentials / "unreadable"
        secret.write_text("synthetic-private-content")
        secret.chmod(0)
        self.configure()


@unittest.skipUnless(
    os.environ.get("SITES_SUPERVISOR_INTEGRATION") == "1"
    and sys.platform == "linux"
    and os.geteuid() == 0
    and shutil.which("supervisord")
    and shutil.which("supervisorctl"),
    "requires explicit opt-in in a disposable root Linux container with Supervisor and cron",
)
class RealSupervisorTests(unittest.TestCase):
    def test_control_privileges_argv_environment_and_ordered_shutdown(self):
        with tempfile.TemporaryDirectory(prefix="sites-supervisor-") as temporary:
            directory = Path(temporary)
            directory.chmod(0o755)
            run = directory / "run"
            run.mkdir()
            credentials = directory / "credentials"
            credentials.mkdir(mode=0o700)
            for name in ("id_ed25519", "known_hosts", "borg-passphrase"):
                (credentials / name).write_text("synthetic-unused")
            events = directory / "events"
            events.mkdir(mode=0o777)
            events.chmod(0o777)
            log = events / "order"
            log.touch(mode=0o666)
            log.chmod(0o666)
            fixture = directory / "finitesitesd"
            fixture.write_text(f"""#!/usr/bin/python3
import json, os, signal, socket, subprocess, sys, time
from pathlib import Path
events = Path({str(events)!r})
def stopped(signum, frame):
    with (events / "order").open("a") as stream:
        stream.write("sites-stop:" + str(signum) + "\\n")
    sys.exit(0)
signal.signal(signal.SIGINT, stopped)
child_code = "import signal,sys,time; from pathlib import Path; " + \
    "signal.signal(signal.SIGINT, lambda *_: (Path(" + repr(str(events / "child-stopped")) + \
    ").write_text('SIGINT'), sys.exit(0))); time.sleep(300)"
child = subprocess.Popen([sys.executable, "-c", child_code])
client = socket.socket(socket.AF_UNIX)
try:
    client.connect({str(run / "sites-supervisor.sock")!r})
    access = "allowed"
except PermissionError:
    access = "denied"
finally:
    client.close()
(events / "sites.json").write_text(json.dumps({{
    "argv": sys.argv[1:], "uid": os.getuid(), "gid": os.getgid(),
    "groups": os.getgroups(), "socket": access,
    "status": Path("/proc/self/status").read_text(),
}}))
print("sites fixture stdout", flush=True)
print("sites fixture stderr", file=sys.stderr, flush=True)
while True:
    time.sleep(0.05)
""")
            fixture.chmod(0o755)
            backup = directory / "backup.py"
            backup.write_text(f"""
import json, os, signal, sys, time
from pathlib import Path
events = Path({str(events)!r})
def stopped(signum, frame):
    time.sleep(0.2)
    with (events / "order").open("a") as stream:
        stream.write("backup-stop\\n")
    sys.exit(0)
signal.signal(signal.SIGTERM, stopped)
(events / "backup.json").write_text(json.dumps(dict(os.environ)))
while True:
    time.sleep(0.05)
""")
            args = [
                "serve",
                "--data",
                "/var/lib/finite-sites",
                "--mailer",
                "dev",
                "--mail-from",
                "A 'quoted' \"name\" ; 100% %(ENV_SECRET)s\nnext",
                "--empty",
                "",
            ]
            module = load_helper()
            path = module.configure(
                args,
                {
                    "FINITE_SITES_BACKUP_REPOSITORY": "backup@example.test:sites",
                    "FINITE_SITES_BACKUP_CREDENTIALS_DIR": str(credentials),
                },
                run_dir=run,
                cron_path=directory / "cron",
            )
            content = path.read_text().replace(
                "/usr/local/bin/sites-supervisor.py", str(HELPER)
            )
            content = content.replace(
                "/usr/local/bin/sites-backup run --config",
                f"{sys.executable} {backup} run --config",
            )
            path.write_text(content)
            output = directory / "output"
            cron_fixture = Path("/etc/cron.d") / directory.name
            self.addCleanup(cron_fixture.unlink, missing_ok=True)
            with output.open("a") as stream:
                process = subprocess.Popen(
                    ["supervisord", "-n", "-c", str(path)],
                    env=dict(
                        os.environ,
                        PATH=f"{directory}:" + os.environ["PATH"],
                        SYNTHETIC_TOKEN="must-not-reach-cron",
                    ),
                    stdout=stream,
                    stderr=stream,
                )
                try:

                    def wait_for(predicate, timeout=15):
                        deadline = time.monotonic() + timeout
                        while time.monotonic() < deadline:
                            if predicate():
                                return
                            self.assertIsNone(process.poll(), output.read_text())
                            time.sleep(0.05)
                        self.fail("Supervisor fixture timed out: " + output.read_text())

                    def control(*command):
                        result = subprocess.run(
                            ["supervisorctl", "-c", str(path), *command],
                            text=True,
                            capture_output=True,
                            timeout=15,
                        )
                        self.assertEqual(
                            result.returncode, 0, result.stdout + result.stderr
                        )
                        return result.stdout

                    wait_for(lambda: (events / "sites.json").exists())
                    receipt = json.loads((events / "sites.json").read_text())
                    self.assertEqual(receipt["argv"], args)
                    self.assertEqual((receipt["uid"], receipt["gid"]), (65532, 65532))
                    self.assertEqual(receipt["groups"], [])
                    self.assertEqual(receipt["socket"], "denied")
                    self.assertIn("NoNewPrivs:\t1", receipt["status"])
                    sock = run / "sites-supervisor.sock"
                    self.assertEqual(sock.stat().st_uid, 0)
                    self.assertEqual(stat.S_IMODE(sock.stat().st_mode), 0o600)
                    wait_for(
                        lambda: (
                            "success: cron entered RUNNING state" in output.read_text()
                        )
                    )
                    cron_pid = control("pid", "cron").strip()
                    cron_env = Path(f"/proc/{cron_pid}/environ").read_bytes()
                    self.assertNotIn(b"SYNTHETIC_TOKEN", cron_env)
                    self.assertIn(b"TZ=UTC\0", cron_env)
                    control("stop", "sites")
                    self.assertIn("sites-stop:2", log.read_text())
                    wait_for(lambda: (events / "child-stopped").exists())
                    self.assertEqual((events / "child-stopped").read_text(), "SIGINT")
                    (events / "child-stopped").unlink()
                    (events / "sites.json").unlink()
                    control("start", "sites")
                    wait_for(lambda: (events / "sites.json").exists())
                    log.write_text("")
                    # Exercise the generated job with Debian cron, advancing only
                    # the fixture schedule to every minute to avoid a daily wait.
                    cron_fixture.write_text(
                        (directory / "cron")
                        .read_text()
                        .replace("7 3 * * * root", "* * * * * root")
                    )
                    cron_fixture.chmod(0o600)
                    wait_for(lambda: (events / "backup.json").exists(), timeout=75)
                    self.assertNotIn(
                        "SYNTHETIC_TOKEN",
                        json.loads((events / "backup.json").read_text()),
                    )
                    process.send_signal(signal.SIGINT)
                    self.assertEqual(process.wait(timeout=20), 0)
                    self.assertEqual(
                        log.read_text().splitlines(), ["backup-stop", "sites-stop:2"]
                    )
                    self.assertTrue((events / "child-stopped").exists())
                    logs = output.read_text()
                    self.assertLess(
                        logs.index("stopped: cron"), logs.index("stopped: sites-backup")
                    )
                    self.assertLess(
                        logs.index("stopped: sites-backup"),
                        logs.rindex("stopped: sites ("),
                    )
                    self.assertIn("sites fixture stdout", logs)
                    self.assertIn("sites fixture stderr", logs)
                finally:
                    if process.poll() is None:
                        process.send_signal(signal.SIGTERM)
                        process.wait(timeout=30)


if __name__ == "__main__":
    unittest.main()
