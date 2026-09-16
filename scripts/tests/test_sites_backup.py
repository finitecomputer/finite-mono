import contextlib
import fcntl
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "infra/scripts/sites-backup"


def load_backup():
    loader = importlib.machinery.SourceFileLoader("sites_backup", str(SCRIPT))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


class SitesBackupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.data = self.root / "data"
        self.data.mkdir()
        with contextlib.closing(sqlite3.connect(self.data / "registry.db")) as db:
            db.execute("create table evidence (value text)")
            db.execute("insert into evidence values ('original')")
            db.commit()
        (self.data / "cookie-secret").write_text(secrets.token_hex(32))
        (self.data / "blobs").mkdir()
        (self.data / "blobs/index.html").write_text("original site")
        self.git = self.data / "git/projects/source.git"
        self.git.parent.mkdir(parents=True)
        subprocess.run(
            ["git", "init", "--bare", str(self.git)], check=True, capture_output=True
        )

    def run_cli(self, *args, env=None):
        return subprocess.run(
            ["python3", str(SCRIPT), *map(str, args)],
            capture_output=True,
            text=True,
            timeout=60,
            env=env,
        )

    def require_ok(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        return result

    def test_offline_snapshot_restores_full_tree_without_changing_source(self):
        snapshot = self.root / "snapshot"
        source_bytes = (self.data / "registry.db").read_bytes()
        self.require_ok(
            self.run_cli(
                "snapshot", "--offline", "--data", self.data, "--target", snapshot
            )
        )
        self.assertEqual((self.data / "registry.db").read_bytes(), source_bytes)
        self.assertFalse((snapshot / "finite-sites/registry.db-wal").exists())
        self.assertTrue((snapshot / "manifest.sha256").is_file())
        target = self.root / "restored"
        self.require_ok(
            self.run_cli("restore", "--snapshot", snapshot, "--target", target)
        )
        self.assertEqual((target / "blobs/index.html").read_text(), "original site")
        self.assertEqual(
            (target / "cookie-secret").read_bytes(),
            (self.data / "cookie-secret").read_bytes(),
        )
        self.assertTrue((target / "git/projects/source.git/HEAD").is_file())
        with contextlib.closing(sqlite3.connect(target / "registry.db")) as db:
            self.assertEqual(
                db.execute("select value from evidence").fetchone(), ("original",)
            )
        self.assertNotEqual(
            self.run_cli(
                "restore", "--snapshot", snapshot, "--target", target
            ).returncode,
            0,
        )

    def job_config(self):
        credentials = self.root / "credentials"
        credentials.mkdir(mode=0o700)
        self.passphrase = secrets.token_hex(32)
        for name, value in (
            ("borg-passphrase", self.passphrase),
            ("id_ed25519", "synthetic-local-test"),
            ("known_hosts", "synthetic-local-test"),
        ):
            path = credentials / name
            path.write_text(value)
            path.chmod(0o600)
        repository = self.root / "borg"
        self.borg_env = {
            k: v for k, v in os.environ.items() if not k.startswith("BORG_")
        }
        self.borg_env.update(
            BORG_REPO=str(repository),
            BORG_PASSPHRASE=self.passphrase,
            BORG_BASE_DIR=str(self.root / "borg-client"),
        )
        subprocess.run(
            ["borg", "init", "--encryption=repokey-blake2"],
            env=self.borg_env,
            check=True,
            capture_output=True,
        )
        self.state = self.root / "service-state"
        self.state.write_text("RUNNING")
        self.calls = self.root / "control-calls"
        control = self.root / "control"
        control.write_text(
            "#!/usr/bin/env python3\nimport pathlib,sys\np=pathlib.Path(__file__).parent\ns=p/'service-state'\na=sys.argv[1]\nwith (p/'control-calls').open('a') as f: f.write(a+'\\n')\nif a=='status': print('sites '+s.read_text())\nelif a=='stop': s.write_text('STOPPED')\nelif a=='start': s.write_text('RUNNING')\n"
        )
        control.chmod(0o700)
        self.backups = self.root / "backups"
        config = self.root / "backup.json"
        config.write_text(
            json.dumps(
                {
                    "data": str(self.data),
                    "root": str(self.backups),
                    "repository": str(repository),
                    "remote_path": "borg12",
                    "credentials_dir": str(credentials),
                    "control": [str(control)],
                    "service": "sites",
                }
            )
        )
        config.chmod(0o600)
        return config

    def test_job_captures_fresh_state_resumes_service_and_archives_with_native_borg(
        self,
    ):
        config = self.job_config()
        self.require_ok(self.run_cli("run", "--config", config))
        status = json.loads((self.backups / "status.json").read_text())
        self.assertEqual(status["status"], "ok")
        self.assertEqual(self.state.read_text(), "RUNNING")
        calls = self.calls.read_text().splitlines()
        self.assertLess(calls.index("stop"), calls.index("start"))
        extracted = self.root / "extracted"
        extracted.mkdir()
        subprocess.run(
            ["borg", "extract", "::" + status["archive"]],
            cwd=extracted,
            env=self.borg_env,
            check=True,
            capture_output=True,
        )
        target = self.root / "restored"
        self.require_ok(
            self.run_cli(
                "restore", "--snapshot", extracted / "snapshot", "--target", target
            )
        )
        self.assertEqual((target / "blobs/index.html").read_text(), "original site")
        self.assertTrue(status["snapshot_at"] <= status["uploaded_at"])

    def test_capture_failure_restarts_sites_and_does_not_archive_stale_data(self):
        config = self.job_config()
        self.require_ok(self.run_cli("run", "--config", config))
        previous = json.loads((self.backups / "status.json").read_text())
        (self.data / "registry.db").write_bytes(b"corrupted source")
        self.assertNotEqual(self.run_cli("run", "--config", config).returncode, 0)
        failed = json.loads((self.backups / "status.json").read_text())
        self.assertEqual(failed["status"], "failed")
        self.assertEqual(failed["archive"], previous["archive"])
        self.assertEqual(failed["uploaded_at"], previous["uploaded_at"])
        self.assertEqual(self.state.read_text(), "RUNNING")
        archives = json.loads(
            subprocess.check_output(["borg", "list", "--json"], env=self.borg_env)
        )["archives"]
        self.assertEqual(len(archives), 1)

    def test_hashing_and_verification_run_only_after_restart_without_capture_alarm(
        self,
    ):
        config = self.job_config()
        module = load_backup()
        digest = module.hashlib.file_digest
        observed = []

        def checked_digest(stream, algorithm):
            self.assertEqual(self.state.read_text(), "RUNNING")
            self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0, 0))
            if str(stream.name).endswith("/blobs/index.html"):
                observed.append(stream.name)
            return digest(stream, algorithm)

        with patch.object(module.hashlib, "file_digest", checked_digest):
            module.run_job(config)
        # One manifest pass and one verification pass, both after restart.
        self.assertEqual(len(observed), 2)
        self.assertEqual(
            json.loads((self.backups / "status.json").read_text())["status"], "ok"
        )

    def test_capture_deadline_interrupts_python_and_subprocess_then_restarts(self):
        config = self.job_config()
        module = load_backup()
        original_command = module.command

        def slow_copy(args, **kwargs):
            if args[0] == "rsync":
                args = [sys.executable, "-c", "import time; time.sleep(30)"]
            return original_command(args, **kwargs)

        for operation in ("inventory", "rsync"):
            with self.subTest(operation=operation):
                delayed = (
                    patch.object(
                        module, "inventory", side_effect=lambda _: time.sleep(30)
                    )
                    if operation == "inventory"
                    else patch.object(module, "command", side_effect=slow_copy)
                )
                started = time.monotonic()
                with patch.object(module, "CAPTURE_TIMEOUT", 1), delayed:
                    with self.assertRaises(TimeoutError):
                        module.run_job(config)
                self.assertLess(time.monotonic() - started, 10)
                self.assertEqual(self.state.read_text(), "RUNNING")
                self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0, 0))
                receipt = json.loads((self.backups / "status.json").read_text())
                self.assertEqual(receipt["status"], "failed")
                self.assertNotIn("uploaded_at", receipt)
                self.assertEqual(list(self.backups.glob("capture-*")), [])
        archives = json.loads(
            subprocess.check_output(["borg", "list", "--json"], env=self.borg_env)
        )["archives"]
        self.assertEqual(archives, [])

    def test_verification_failure_after_restart_does_not_upload(self):
        config = self.job_config()
        module = load_backup()
        verify = module.verify

        def corrupt_snapshot(stage):
            self.assertEqual(self.state.read_text(), "RUNNING")
            (stage / "finite-sites/registry.db").write_bytes(b"corrupt copy")
            return verify(stage)

        with patch.object(module, "verify", side_effect=corrupt_snapshot):
            with self.assertRaises(ValueError):
                module.run_job(config)
        self.assertEqual(self.state.read_text(), "RUNNING")
        receipt = json.loads((self.backups / "status.json").read_text())
        self.assertEqual(receipt["status"], "failed")
        self.assertNotIn("uploaded_at", receipt)

    def test_upload_failure_happens_after_service_resumes_and_is_not_success(self):
        config = self.job_config()
        shim = self.root / "bin"
        shim.mkdir()
        wrapper = shim / "borg"
        wrapper.write_text(
            "#!/usr/bin/env python3\nimport os,pathlib,sys\n"
            f"state=pathlib.Path({str(self.state)!r})\n"
            "if sys.argv[1]=='create':\n"
            " state.with_name('upload-observed-state').write_text(state.read_text())\n"
            " sys.exit(1)\n"
            f"os.execv({shutil.which('borg')!r}, ['borg', *sys.argv[1:]])\n"
        )
        wrapper.chmod(0o700)
        env = {**os.environ, "PATH": str(shim) + os.pathsep + os.environ["PATH"]}
        self.assertNotEqual(
            self.run_cli("run", "--config", config, env=env).returncode, 0
        )
        self.assertEqual((self.root / "upload-observed-state").read_text(), "RUNNING")
        state = json.loads((self.backups / "status.json").read_text())
        self.assertEqual(state["status"], "failed")
        self.assertNotIn("uploaded_at", state)

    def test_failed_restart_prevents_upload(self):
        config = self.job_config()
        control = self.root / "control"
        control.write_text(
            control.read_text().replace("s.write_text('RUNNING')", "sys.exit(1)")
        )
        self.assertNotEqual(self.run_cli("run", "--config", config).returncode, 0)
        state = json.loads((self.backups / "status.json").read_text())
        self.assertEqual(state["status"], "failed")
        self.assertNotIn("uploaded_at", state)
        self.assertEqual(self.state.read_text(), "STOPPED")

    def test_corrupt_snapshot_and_symlink_target_fail_without_mutation(self):
        snapshot = self.root / "snapshot"
        self.require_ok(
            self.run_cli(
                "snapshot", "--offline", "--data", self.data, "--target", snapshot
            )
        )
        (snapshot / "finite-sites/blobs/index.html").write_text("tampered")
        target = self.root / "restored"
        self.assertNotEqual(
            self.run_cli(
                "restore", "--snapshot", snapshot, "--target", target
            ).returncode,
            0,
        )
        self.assertFalse(target.exists())
        target.symlink_to(self.data, target_is_directory=True)
        self.assertNotEqual(
            self.run_cli(
                "restore", "--snapshot", snapshot, "--target", target
            ).returncode,
            0,
        )
        self.assertEqual((self.data / "blobs/index.html").read_text(), "original site")

    def test_snapshot_uses_legacy_checksum_and_symlink_inventory_conventions(self):
        (self.data / "spaced name\\with\nnewline\r").write_text("retained")
        (self.data / "outside-link").symlink_to("/not-copied")
        snapshot = self.root / "snapshot"
        self.require_ok(
            self.run_cli(
                "snapshot", "--offline", "--data", self.data, "--target", snapshot
            )
        )
        subprocess.run(
            ["sha256sum", "--check", "manifest.sha256"],
            cwd=snapshot,
            check=True,
            capture_output=True,
        )
        target = self.root / "restored"
        self.require_ok(
            self.run_cli("restore", "--snapshot", snapshot, "--target", target)
        )
        self.assertEqual(os.readlink(target / "outside-link"), "/not-copied")

    def test_wrong_borg_passphrase_does_not_stop_sites(self):
        config = self.job_config()
        (self.root / "credentials/borg-passphrase").write_text(secrets.token_hex(32))
        self.assertNotEqual(self.run_cli("run", "--config", config).returncode, 0)
        self.assertFalse(self.calls.exists())
        self.assertEqual(self.state.read_text(), "RUNNING")

    def test_overlapping_job_is_refused_before_touching_service_or_status(self):
        config = self.job_config()
        self.backups.mkdir(mode=0o700)
        with (self.backups / "job.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertNotEqual(self.run_cli("run", "--config", config).returncode, 0)
        self.assertFalse(self.calls.exists())
        self.assertFalse((self.backups / "status.json").exists())

    def test_snapshot_refuses_symlink_registry_and_capture_needs_offline_assertion(
        self,
    ):
        target = self.root / "snapshot"
        self.assertNotEqual(
            self.run_cli(
                "snapshot", "--data", self.data, "--target", target
            ).returncode,
            0,
        )
        (self.data / "registry.db").rename(self.root / "other.db")
        (self.data / "registry.db").symlink_to(self.root / "other.db")
        self.assertNotEqual(
            self.run_cli(
                "snapshot", "--offline", "--data", self.data, "--target", target
            ).returncode,
            0,
        )
        self.assertFalse(target.exists())

    def test_interrupted_stop_waits_for_supervisor_then_restarts_sites(self):
        config = self.job_config()
        marker = self.root / "stopping"
        daemon = self.root / "daemon.py"
        daemon.write_text(
            "import pathlib, signal, sys, time\n"
            "def stop(*_):\n"
            f"    pathlib.Path({str(marker)!r}).touch()\n"
            "    time.sleep(3)\n    sys.exit(0)\n"
            "signal.signal(signal.SIGINT, stop)\n"
            "while True: time.sleep(.05)\n"
        )
        supervisor_config = self.root / "supervisor.conf"
        supervisor_config.write_text(f"""[unix_http_server]
file={self.root}/supervisor.sock
[supervisord]
nodaemon=true
logfile={self.root}/supervisor.log
pidfile={self.root}/supervisor.pid
childlogdir={self.root}
[rpcinterface:supervisor]
supervisor.rpcinterface_factory=supervisor.rpcinterface:make_main_rpcinterface
[supervisorctl]
serverurl=unix://{self.root}/supervisor.sock
[program:sites]
command={sys.executable} {daemon}
stopsignal=INT
stopwaitsecs=5
startsecs=0
autorestart=true
""")
        control = ["supervisorctl", "-c", str(supervisor_config)]
        settings = json.loads(config.read_text())
        settings["control"] = control
        config.write_text(json.dumps(settings))
        supervisor = subprocess.Popen(
            ["supervisord", "-n", "-c", str(supervisor_config)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        worker = None
        try:
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                result = subprocess.run(
                    [*control, "status", "sites"],
                    capture_output=True,
                    timeout=5,
                )
                if b"RUNNING" in result.stdout:
                    break
                time.sleep(0.05)
            self.assertIn(b"RUNNING", result.stdout)
            worker = subprocess.Popen(
                [sys.executable, str(SCRIPT), "run", "--config", str(config)],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            deadline = time.monotonic() + 10
            while not marker.exists() and time.monotonic() < deadline:
                self.assertIsNone(worker.poll())
                time.sleep(0.05)
            self.assertTrue(marker.exists())
            worker.send_signal(signal.SIGTERM)
            worker.communicate(timeout=15)
            self.assertNotEqual(worker.returncode, 0)
            result = subprocess.run(
                [*control, "status", "sites"],
                capture_output=True,
                timeout=5,
            )
            self.assertIn(b"RUNNING", result.stdout)
            self.assertEqual(
                json.loads((self.backups / "status.json").read_text())["status"],
                "failed",
            )
        finally:
            if worker is not None:
                if worker.poll() is None:
                    worker.kill()
                worker.communicate(timeout=5)
            supervisor.terminate()
            supervisor.wait(timeout=15)

    def test_interrupted_capture_restarts_sites_and_reports_failure(self):
        config = self.job_config()
        shim = self.root / "bin"
        shim.mkdir()
        marker = self.root / "capture-started"
        rsync = shim / "rsync"
        rsync.write_text(
            "#!/usr/bin/env python3\nimport pathlib, time\n"
            f"pathlib.Path({str(marker)!r}).touch()\ntime.sleep(60)\n"
        )
        rsync.chmod(0o755)
        environment = dict(os.environ, PATH=str(shim) + os.pathsep + os.environ["PATH"])
        process = subprocess.Popen(
            ["python3", str(SCRIPT), "run", "--config", str(config)],
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            deadline = time.monotonic() + 15
            while not marker.exists() and time.monotonic() < deadline:
                self.assertIsNone(process.poll())
                time.sleep(0.05)
            self.assertTrue(marker.exists())
            self.assertEqual(self.state.read_text(), "STOPPED")
            process.send_signal(signal.SIGTERM)
            process.communicate(timeout=10)
            self.assertNotEqual(process.returncode, 0)
            self.assertEqual(self.state.read_text(), "RUNNING")
            state = json.loads((self.backups / "status.json").read_text())
            self.assertEqual(state["status"], "failed")
            self.assertNotIn("uploaded_at", state)
        finally:
            if process.poll() is None:
                process.kill()
            process.communicate(timeout=5)


if __name__ == "__main__":
    unittest.main()
