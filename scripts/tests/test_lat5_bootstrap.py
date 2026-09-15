#!/usr/bin/env python3
"""Exercise the bootstrap boundary that must pass before the empty-host wipe."""
import base64
import runpy
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

validate_bootstrap = runpy.run_path(
    str(Path(__file__).resolve().parents[1] / 'check-lat5-bootstrap')
)['validate']


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        for directory in ('etc', 'etc/finite', 'etc/ssh'):
            (self.root / directory).mkdir(mode=0o700)
        for name in ('etc/finite/metrics-remote-write.env', 'etc/finite/logs-write.env'):
            path = self.root / name
            path.write_text('synthetic fixture\n')
            path.chmod(0o600)
        self.private = self.root / 'etc/ssh/ssh_host_ed25519_key'
        self.public = self.root / 'etc/ssh/ssh_host_ed25519_key.pub'
        subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(self.private)],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.public.chmod(0o600)
        self.key = self.root / 'etc/finite/wireguard-private-key'
        self.key.write_bytes(base64.b64encode(os.urandom(32)) + b'\n')
        self.key.chmod(0o600)

    def validate(self):
        validate_bootstrap(self.root, expected_uid=os.getuid(), expected_gid=os.getgid())

    def test_complete_private_bootstrap_passes(self):
        self.validate()

    def test_original_four_file_bootstrap_is_refused(self):
        self.key.unlink()
        with self.assertRaisesRegex(SystemExit, 'WireGuard key'):
            self.validate()

    def test_invalid_keys_are_refused_without_echoing_values(self):
        for value in (b'', b'synthetic-invalid-secret', base64.b64encode(bytes(32)),
                      base64.b64encode(os.urandom(31)), base64.b64encode(os.urandom(32)) + b'\n\n'):
            with self.subTest(length=len(value)):
                self.key.write_bytes(value)
                with self.assertRaisesRegex(SystemExit, 'invalid WireGuard private key') as error:
                    self.validate()
                self.assertNotIn('synthetic-invalid-secret', str(error.exception))

    def test_readable_key_and_symlink_are_refused(self):
        self.key.chmod(0o644)
        with self.assertRaisesRegex(SystemExit, 'unexpected path or mode'):
            self.validate()
        self.key.unlink()
        self.key.symlink_to(self.root / 'etc/ssh/ssh_host_ed25519_key')
        with self.assertRaisesRegex(SystemExit, 'not symlinks'):
            self.validate()

    def test_runner_admission_is_refused(self):
        path = self.root / 'etc/finite/runner.env'
        path.write_text('synthetic fixture\n')
        path.chmod(0o600)
        with self.assertRaisesRegex(SystemExit, 'unexpected path or mode'):
            self.validate()

    def test_malformed_private_key_is_refused(self):
        self.private.write_text('synthetic-invalid-private-key')
        with self.assertRaisesRegex(SystemExit, 'invalid or mismatched Ed25519'):
            self.validate()

    def test_mismatched_public_key_is_refused(self):
        other = Path(self.temp.name) / 'other-key'
        subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(other)],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.public.write_bytes(other.with_suffix('.pub').read_bytes())
        other.unlink()
        other.with_suffix('.pub').unlink()
        with self.assertRaisesRegex(SystemExit, 'invalid or mismatched Ed25519'):
            self.validate()

    def test_incorrect_owner_is_refused(self):
        with self.assertRaisesRegex(SystemExit, 'root-owned'):
            validate_bootstrap(self.root, expected_uid=os.getuid() + 1, expected_gid=os.getgid())


if __name__ == '__main__':
    unittest.main()
