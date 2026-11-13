"""Exercise the actual guard script against stateful firewall/sysctl fixtures."""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[2] / "infra/scripts/mitigate-cve-2026-98432"
GUARD = '-A PREROUTING -p tcp -m u32 --u32 "0x0>>0x18&0xf=0x6:0xf" -m comment --comment finite-cve-2026-98432 -j DROP'
ACCEPT = "-A PREROUTING -s 192.0.2.1/32 -j ACCEPT"
OTHER = "-A PREROUTING -p udp -j ACCEPT"
SHIM = r'''
import json, os, sys
from pathlib import Path
p = Path(os.environ['GUARD_STATE'])
s = json.loads(p.read_text())
a = sys.argv[1:]
command = Path(sys.argv[0]).name
s['events'].append([command, *a])
code = 0
output = ''
if command == 'iptables':
    assert a[:4] == ['-w', '10', '-t', 'raw'], a
    if a[4:] == ['-S', 'PREROUTING', '1']:
        if s['rules']:
            output = s['rules'][0]
        else:
            code = 1
    else:
        assert a[4:7] == ['-I', 'PREROUTING', '1'], a
        assert a[7:] == ['-p','tcp','-m','u32','--u32','0>>24&0xF=6:15',
                         '-m','comment','--comment','finite-cve-2026-98432','-j','DROP'], a
        if s.get('fail_insert'):
            code = 1
        else:
            s['rules'].insert(0, os.environ['GUARD_CANONICAL'])
elif a == ['-n', 'net.mptcp.enabled']:
    output = s['mptcp']
else:
    assert command == 'sysctl' and a == ['-w', 'net.mptcp.enabled=0'], a
    if s.get('fail_sysctl'):
        code = 1
    else:
        s['mptcp'] = '0'
p.write_text(json.dumps(s))
print(output)
sys.exit(code)
'''


class GuardScriptTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.state_path = self.root / "state.json"
        shim = self.root / "shim"
        shim.write_text(f"#!{sys.executable}\n" + SHIM)
        shim.chmod(0o755)
        for command in ("iptables", "sysctl"):
            (self.root / command).symlink_to(shim)
        self.environment = {
            **os.environ, "PATH": f"{self.root}:{os.environ['PATH']}",
            "GUARD_STATE": str(self.state_path), "GUARD_CANONICAL": GUARD,
        }

    def state(self, rules=(), **extra):
        self.state_path.write_text(json.dumps({
            "rules": list(rules), "mptcp": "1", "events": [], **extra,
        }))

    def run_script(self, argument="--apply", success=True):
        result = subprocess.run(
            ["bash", str(SCRIPT), argument], env=self.environment,
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode == 0, success, result.stderr)
        return json.loads(self.state_path.read_text())

    def test_clean_and_repeated_apply_preserve_unrelated_rules(self):
        self.state([OTHER])
        self.run_script()
        state = self.run_script()
        self.assertEqual(state['rules'], [GUARD, OTHER])
        self.assertEqual(sum('-I' in event for event in state['events']), 1)
        self.run_script('--check')

    def test_guard_behind_accept_is_rejected_then_repaired_without_a_gap(self):
        self.state([ACCEPT, GUARD, OTHER], mptcp='0')
        self.run_script('--check', success=False)
        state = self.run_script()
        # The previous guard stays in place while the new first rule is added.
        # No delete/flush or rule-number deletion can affect concurrent rules.
        self.assertEqual(state['rules'], [GUARD, ACCEPT, GUARD, OTHER])
        self.run_script('--check')
        self.assertEqual(self.run_script()['rules'], state['rules'])

    def test_failed_insert_does_not_disable_mptcp_or_remove_existing_rules(self):
        self.state([ACCEPT, GUARD], fail_insert=True)
        state = self.run_script(success=False)
        self.assertEqual(state['rules'], [ACCEPT, GUARD])
        self.assertEqual(state['mptcp'], '1')

    def test_failed_sysctl_keeps_packet_barrier_and_check_fails(self):
        self.state([OTHER], fail_sysctl=True)
        state = self.run_script(success=False)
        self.assertEqual(state['rules'], [GUARD, OTHER])
        self.run_script('--check', success=False)

    def test_invalid_argument_changes_nothing(self):
        self.state([OTHER])
        state = self.run_script('--invalid', success=False)
        self.assertEqual(state['events'], [])
