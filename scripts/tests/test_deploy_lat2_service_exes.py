"""Exercise the actual lat2 activation gate with a stale running process."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class RunningExecutableTests(unittest.TestCase):
    def verify(self, *, stale=False, no_pid=False):
        source = (ROOT / 'scripts/deploy-lat2-closure-cache').read_text()
        block = source.split('# BEGIN RUNNING_EXECUTABLE_VERIFICATION\n', 1)[1].split('# END RUNNING_EXECUTABLE_VERIFICATION', 1)[0]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            units = root / 'etc/systemd/system'
            units.mkdir(parents=True)
            (units / 'finite-saas-core.service').write_text('[Service]\nExecStart=/nix/store/candidate-core/bin/finite-saas-core\n')
            for name, body in {
                'systemctl': 'printf "%s\\n" "$TEST_PID"',
                'readlink': 'case "$2" in /proc/*) printf "%s\\n" "$TEST_EXE";; *) printf "%s\\n" "$2";; esac',
            }.items():
                p = root / name
                p.write_text('#!/bin/sh\n' + body + '\n')
                p.chmod(0o755)
            env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ['PATH'],
                       TEST_PID='0' if no_pid else '123',
                       TEST_EXE='/nix/store/old-core/bin/finite-saas-core' if stale else '/nix/store/candidate-core/bin/finite-saas-core')
            return subprocess.run(['bash', '-c', 'set -euo pipefail\nsystem="$1"\nactivation_failed() { echo "$2" >&2; exit "$1"; }\n' + block, 'verify', str(root)], env=env, text=True, capture_output=True)

    def test_candidate_process_passes(self):
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('verified finite-saas-core.service', result.stdout)

    def test_active_old_process_fails(self):
        result = self.verify(stale=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('executable mismatch', result.stderr)
        self.assertIn('inspect before an explicitly scoped restart', result.stderr)

    def test_absent_process_fails(self):
        self.assertNotEqual(self.verify(no_pid=True).returncode, 0)


if __name__ == '__main__':
    unittest.main()
