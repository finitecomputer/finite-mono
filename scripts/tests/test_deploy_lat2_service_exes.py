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


class ActivationEntryPointTests(unittest.TestCase):
    """Run --activate, including its real remote body, with host I/O shims.

    The SSH shim maps absolute host filesystem roots into a temporary tree.
    It does not replace control flow, functions, gates, or the failure handler.
    No network, host service, system profile, or real Nix store is modified.
    """

    def test_stale_process_refuses_success_and_rearms_timers(self):
        self.run_activation(stale=True)

    def test_matching_process_reports_deployed(self):
        self.run_activation(stale=False)

    def run_activation(self, *, stale):
        import sys
        from scripts.tests.test_lat2_closure_artifact import (
            VALID_MANIFEST, write_valid_artifact, write_shim,
        )
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            artifact = root / 'artifact'
            artifact.mkdir()
            write_valid_artifact(artifact)
            shims = root / 'bin'
            shims.mkdir()
            store_root = str(root / 'store') + '/'
            system = Path(VALID_MANIFEST['system'].replace('/nix/store/', store_root))
            units = system / 'etc/systemd/system'
            units.mkdir(parents=True)
            (system / 'bin').mkdir()
            (units / 'finite-saas-core.service').write_text(
                '[Service]\nExecStart=' + store_root + 'candidate-core/bin/finite-saas-core\n')
            (units / 'finite-healthcheck.timer').touch()
            marker = root / 'switched'
            log = root / 'calls'
            write_shim(system / 'bin', 'switch-to-configuration',
                       'echo "switch $*" >> "$CALL_LOG"\ntouch "$SWITCH_MARKER"\n')
            for tool in ['git', 'nix', 'nix-store', 'nix-env']:
                write_shim(shims, tool, f'echo "{tool} $*" >> "$CALL_LOG"\n')
            write_shim(shims, 'systemctl', '''
echo "systemctl $*" >> "$CALL_LOG"
case "$1" in
  --failed) exit 0;;
  cat) exit 1;;
  show) echo 123;;
  is-active) [[ "${!#}" != finite-saas-runner.service ]];;
  stop|start) [[ "${!#}" == *.timer ]];;
  *) exit 90;;
esac
''')
            write_shim(shims, 'readlink', '''
case "${!#}" in
  /run/current-system|/nix/var/nix/profiles/system)
    if [[ -e "$SWITCH_MARKER" ]]; then echo "$TEST_SYSTEM"; else echo "$TEST_OLD_SYSTEM"; fi;;
  /proc/*/exe) echo "$TEST_EXE";;
  *) echo "${!#}";;
esac
''')
            ssh = shims / 'ssh'
            ssh.write_text('#!' + sys.executable + '\n' + '''import os,sys,subprocess
args=sys.argv[1:]
if 'dry-activate' in args:
    print('would restart the following units: finite-saas-core.service')
    sys.exit(0)
body=sys.stdin.read()
if 'LAT2' not in body and 'verify_running_executable' not in body and 'previous_system=' not in body:
    sys.exit(0) # host-secret preflight is outside this process-identity fixture
store=os.environ['TEST_STORE_ROOT']
body=body.replace('/nix/store/',store)
body=body.replace('systemd_units_dir=/etc/systemd/system', 'systemd_units_dir='+os.environ['TEST_UNITS'])
remote_args=[a.replace('/nix/store/',store) for a in args[args.index('--')+1:]]
sys.exit(subprocess.run(['bash','-s','--',*remote_args],input=body,text=True).returncode)
''')
            ssh.chmod(0o755)
            env = dict(os.environ, PATH=str(shims)+os.pathsep+os.environ['PATH'],
                       CALL_LOG=str(log), SWITCH_MARKER=str(marker),
                       TEST_SYSTEM=str(system), TEST_OLD_SYSTEM=str(root/'previous'),
                       TEST_UNITS=str(units), TEST_STORE_ROOT=store_root,
                       TEST_EXE=store_root+('old-core' if stale else 'candidate-core')+'/bin/finite-saas-core')
            result = subprocess.run([str(ROOT/'scripts/deploy-lat2-closure-cache'), '--activate', str(artifact)],
                                    cwd=ROOT, env=env, text=True, capture_output=True)
            calls = log.read_text().splitlines()
            self.assertIn('switch switch', calls, result.stderr)
            self.assertIn('systemctl stop finite-healthcheck.timer', calls)
            self.assertIn('systemctl start finite-healthcheck.timer', calls)
            self.assertFalse(any(line.startswith('systemctl restart') for line in calls))
            self.assertEqual(sum(line.startswith('nix-env ') for line in calls), 1)
            if stale:
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn('==> DEPLOYED', result.stdout)
                self.assertIn('executable mismatch', result.stderr)
                self.assertIn('ACTIVATION FAILED', result.stderr)
                self.assertIn('NO automatic revert', result.stderr)
            else:
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn('==> DEPLOYED', result.stdout)


if __name__ == '__main__':
    unittest.main()
