from __future__ import annotations

import contextlib
import io
import json
import re
import secrets
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts import kata_lifecycle_lab as lab
from scripts.tests import kata_lifecycle_fixtures as fx

FOREIGN_ID = 'f' * 64


class FakeDocker:
    """In-memory docker CLI with injectable faults. Records every call."""

    def __init__(self, scenario: str = 'idle', faults: set[str] | None = None) -> None:
        self.scenario = scenario
        self.faults = faults or set()
        self.images = {fx.IMAGE_ID: 'linux/arm64'}
        self.containers: dict[str, dict] = {}
        self.calls: list[list[str]] = []
        self.records: fx.Records | None = None
        self.on_stream = None

    def add_foreign(self, name: str) -> None:
        self.containers[FOREIGN_ID] = {'name': name, 'labels': {'other': 'thread'}}

    def find(self, target: str) -> str | None:
        for cid, c in self.containers.items():
            if target in (cid, c['name']):
                return cid
        return None

    def missing(self, target: str) -> lab.Result:
        return lab.Result(1, '', f'Error response from daemon: No such container: {target}\n')

    def build(self, run: str, cid: str) -> None:
        if self.records is None:
            self.records = fx.stop_records(run, cid, self.scenario)

    def lines(self, source: str) -> str:
        assert self.records is not None
        return ''.join(line + '\n' for line in self.records.lines(source))

    def run(self, args: list[str], timeout: float) -> lab.Result:
        self.calls.append(args)
        cmd = args[0]
        if cmd == 'image':
            image = args[-1]
            if image not in self.images or 'image-mismatch' in self.faults:
                return lab.Result(1, '', 'No such image')
            return lab.Result(0, f'{image} {self.images[image]}\n', '')
        if cmd == 'container':
            cid = self.find(args[-1])
            if cid is None:
                return self.missing(args[-1])
            c = self.containers[cid]
            if args[3] == '{{.Id}}':
                return lab.Result(0, cid + '\n', '')
            labels = c['labels']
            return lab.Result(0, f"{cid}|/{c['name']}|{labels.get(lab.LABEL_RUN, '')}|{labels.get(lab.LABEL_OWNER, '')}\n", '')
        if cmd == 'ps':
            if 'ps-blind' in self.faults:
                return lab.Result(0, '', '')
            key, value = args[-1].removeprefix('label=').split('=', 1)
            return lab.Result(0, ''.join(cid + '\n' for cid, c in self.containers.items() if c['labels'].get(key) == value), '')
        if cmd == 'create':
            name = args[args.index('--name') + 1]
            labels = dict(a.split('=', 1) for a in args[1:] if '=' in a and a.startswith('computer.finite.lab.'))
            if 'create-race' in self.faults:
                self.add_foreign(name)
            if self.find(name) is not None:
                return lab.Result(125, '', f'Conflict. The container name "/{name}" is already in use\n')
            if 'create-fails' in self.faults:
                return lab.Result(125, '', 'create failed\n')
            cid = secrets.token_hex(32)
            self.containers[cid] = {'name': name, 'labels': labels}
            if 'create-timeout' in self.faults:
                return lab.Result(None, '', '', True)
            return lab.Result(0, cid + '\n', '')
        if cmd == 'start':
            return lab.Result(1, '', 'start failed\n') if 'start-fails' in self.faults else lab.Result(0, '', '')
        if cmd == 'cp':
            return lab.Result(0, '', '')
        if cmd == 'rm':
            target = args[-1]
            assert re.fullmatch('[0-9a-f]{64}', target), f'rm by non-ID {target}'
            if 'rm-fails' in self.faults:
                return lab.Result(1, '', 'removal failed\n')
            if 'rm-lies' not in self.faults:
                self.containers.pop(target, None)
            return lab.Result(0, target + '\n', '')
        if cmd == 'exec':
            return self.exec_(args[1], args[2:])
        raise AssertionError(f'unexpected docker call {args}')

    def exec_(self, cid: str, argv: list[str]) -> lab.Result:
        assert cid in self.containers, 'exec into a container that does not exist'
        if argv[:2] == ['test', '-S']:
            return lab.Result(1, '', '') if 'socket-never' in self.faults else lab.Result(0, '', '')
        if argv[0] == 'systemctl':
            return lab.Result(0, 'running\n', '')
        if argv[0] == 'sha256sum':
            return lab.Result(0, f'{fx.HELPER_SHA}  {lab.HELPER}\n', '')
        if argv[0] == 'systemd-run' or argv[0] == 'grep' or argv[0] == 'journalctl':
            return lab.Result(0, '', '')
        if argv[0] == 'sh':
            source = {'fused.jsonl': 'fused'}.get(argv[-1].rsplit('/', 1)[-1])
            return lab.Result(0, self.lines(source) if source and self.records is not None else '', '')
        assert argv[0] == lab.HELPER, argv
        run = argv[argv.index('-run') + 1]
        sub = argv[1]
        if sub == 'identity':
            if 'identity-garbage' in self.faults:
                return lab.Result(0, 'not json\n', '')
            rec = fx.Records(run)
            fx.identity(rec, agent_sha='e' * 64 if 'agent-mismatch' in self.faults else fx.AGENT_SHA)
            return lab.Result(0, ''.join(line + '\n' for line in rec.lines()), '')
        if sub == 'prepare':
            rec = fx.Records(run)
            rec.add('prepare', 'prepared', 0.5, rootfs='/var/lib/finlab/rootfs', rootfs_helper_sha256=fx.HELPER_SHA)
            return lab.Result(0, '\n'.join(rec.lines()) + '\n', '')
        if sub == 'release':
            return lab.Result(None, '', '', True) if 'release-stall' in self.faults else lab.Result(0, self.lines('release'), '')
        if sub == 'observe':
            if 'final-missing' in self.faults:
                return lab.Result(0, '', '')
            return lab.Result(0, self.lines('observe'), '')
        raise AssertionError(argv)

    def stream(self, args: list[str], timeout: float, on_line) -> lab.Result:
        self.calls.append(args)
        argv = args[2:]
        run = argv[argv.index('-run') + 1]
        cid = argv[argv.index('-cid') + 1]
        self.build(run, cid)
        if self.on_stream:
            self.on_stream(self)
        if 'client-exit-10' in self.faults:
            on_line(self.lines('client').splitlines()[0])
            return lab.Result(10, '', 'setup failed\n')
        if 'client-stall' in self.faults:
            return lab.Result(None, '', '', True)
        for line in self.lines('client').splitlines():
            on_line(line)
        return lab.Result(0, '', '')


class LabTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.helper = Path(self.tmp.name) / 'finlab'
        self.helper.write_bytes(b'helper')
        self.helper_sha = lab.sha256_file(self.helper)
        fx_sha = fx.HELPER_SHA
        self.addCleanup(setattr, fx, 'HELPER_SHA', fx_sha)
        fx.HELPER_SHA = self.helper_sha

    def go(self, docker: FakeDocker, scenario: str = 'idle', name: str = 'out', **arm_overrides) -> tuple[int, Path]:
        arm = fx.stop_arm(scenario, **arm_overrides)
        arm['inputs']['helper_sha256'] = self.helper_sha
        arm['bounds'] = {'ready_s': 0.2, 'settle_s': 0}
        arm_path = Path(self.tmp.name) / f'{name}-arm.json'
        arm_path.write_text(json.dumps(arm))
        out = Path(self.tmp.name) / name
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            code = lab.main(['lab', str(arm_path), str(self.helper), str(out)], engine=docker)
        return code, out

    def state(self, out: Path) -> dict:
        return json.loads((out / 'run.json').read_text())

    def removals(self, docker: FakeDocker) -> list[str]:
        return [c[-1] for c in docker.calls if c[0] == 'rm']

    def assertNoLabRemains(self, docker: FakeDocker) -> None:
        self.assertFalse([c for c in docker.containers.values() if lab.LABEL_RUN in c['labels']])

    def test_positive_control_passes_and_removes_only_its_container(self) -> None:
        docker = FakeDocker()
        code, out = self.go(docker)
        verdict = json.loads((out / 'verdict.json').read_text())
        self.assertEqual((code, verdict['verdict']), (0, 'PASS'), verdict)
        state = self.state(out)
        self.assertEqual(self.removals(docker), [state['container']['id']])
        self.assertEqual(state['cleanup']['audit'], {'inspect_absent': True, 'label_matches': []})
        self.assertNoLabRemains(docker)

    def test_expected_negative_held_request_is_released_by_the_runner(self) -> None:
        docker = FakeDocker('held-starved')
        code, out = self.go(docker, 'held-starved', release_after_s=0)
        self.assertEqual(code, 0, (out / 'verdict.json').read_text())
        self.assertIn('release', [s['name'] for s in self.state(out)['steps']])

    def test_output_reuse_is_refused_before_any_engine_call(self) -> None:
        docker = FakeDocker()
        (Path(self.tmp.name) / 'out').mkdir()
        code, _ = self.go(docker)
        self.assertEqual((code, docker.calls), (2, []))

    def test_launch_name_collision_never_touches_the_other_container(self) -> None:
        docker = FakeDocker()
        original = lab.LabRun.__init__

        def collide(run_self, *args, **kwargs):
            original(run_self, *args, **kwargs)
            docker.add_foreign(run_self.name)

        with mock.patch.object(lab.LabRun, '__init__', collide):
            code, out = self.go(docker)
        self.assertEqual(code, 2)
        self.assertIn(FOREIGN_ID, docker.containers)
        self.assertEqual(self.removals(docker), [])
        self.assertFalse([c for c in docker.calls if c[0] == 'create'])
        self.assertIn('name-free failed', self.state(out)['refused'])

    def test_create_race_never_removes_by_name(self) -> None:
        docker = FakeDocker(faults={'create-race'})
        code, out = self.go(docker)
        self.assertEqual(code, 2)
        self.assertIn(FOREIGN_ID, docker.containers)
        self.assertEqual(self.removals(docker), [])

    def test_create_timeout_adopts_only_the_labelled_container(self) -> None:
        docker = FakeDocker(faults={'create-timeout'})
        code, out = self.go(docker)
        state = self.state(out)
        self.assertEqual(code, 2)
        self.assertTrue(state['container']['adopted_after_failed_create'])
        self.assertEqual(self.removals(docker), [state['container']['id']])
        self.assertNoLabRemains(docker)

    def test_setup_and_readiness_failures_clean_up(self) -> None:
        for fault, step in (('start-fails', 'start'), ('socket-never', 'ready-agent-socket'),
                            ('agent-mismatch', 'agent-identity')):
            with self.subTest(fault=fault):
                docker = FakeDocker(faults={fault})
                code, out = self.go(docker, name=fault)
                state = self.state(out)
                self.assertEqual(code, 2)
                self.assertIn(step, [s['name'] for s in state['steps'] if s['status'] != 'ok'])
                self.assertEqual(self.removals(docker), [state['container']['id']])
                self.assertNoLabRemains(docker)

    def test_malformed_helper_output_is_inconclusive_not_fail(self) -> None:
        docker = FakeDocker(faults={'identity-garbage'})
        code, out = self.go(docker)
        state = self.state(out)
        self.assertEqual((code, state['internal_error'], state['runner_status']), (2, 'JSONDecodeError', 'aborted'))
        self.assertEqual(self.removals(docker), [state['container']['id']])

    def test_inner_client_exit_is_propagated(self) -> None:
        docker = FakeDocker(faults={'client-exit-10'})
        code, out = self.go(docker)
        inner = [s for s in self.state(out)['steps'] if s['name'] == 'inner'][0]
        self.assertEqual((code, inner['status'], inner['exit']), (2, 'failed', 10))
        verdict = json.loads((out / 'verdict.json').read_text())
        self.assertIn('inner failed (exit 10)', verdict['reason'])

    def test_wrong_result_fails(self) -> None:
        docker = FakeDocker('held-starved')
        code, out = self.go(docker, 'held-starved', release_after_s=0,
                            expect=dict(fx.STOP_EXPECT['held-starved'], same_connection='responsive'))
        self.assertEqual(code, 1)

    def test_helper_and_release_stalls_are_bounded_and_inconclusive(self) -> None:
        for fault, step in (('client-stall', 'inner'), ('release-stall', 'release')):
            with self.subTest(fault=fault):
                docker = FakeDocker('held-starved', faults={fault})
                code, out = self.go(docker, 'held-starved', name=fault, release_after_s=0)
                statuses = {s['name']: s['status'] for s in self.state(out)['steps']}
                self.assertEqual((code, statuses[step]), (2, 'timeout'))
                self.assertNoLabRemains(docker)

    def test_missing_final_state_is_inconclusive(self) -> None:
        docker = FakeDocker(faults={'final-missing'})
        code, out = self.go(docker)
        self.assertEqual(code, 2)
        self.assertIn('final observation missing', json.loads((out / 'verdict.json').read_text())['reason'])

    def test_cleanup_failures_exit_3_and_keep_diagnostics(self) -> None:
        # ps-blind: the label filter misses the leaked container, so only the
        # by-ID inspect audit can see it.
        for faults in ({'rm-fails'}, {'rm-lies'}, {'rm-lies', 'ps-blind'}):
            with self.subTest(faults=sorted(faults)):
                docker = FakeDocker(faults=set(faults))
                fault = '-'.join(sorted(faults))
                code, out = self.go(docker, name=fault)
                state = self.state(out)
                self.assertEqual(code, 3)
                self.assertIn(state['container']['id'], docker.containers)
                self.assertTrue((out / 'records.jsonl').exists())
                self.assertFalse((out / 'verdict.json').exists())

    def test_ownership_guard_blocks_removal_of_a_relabelled_container(self) -> None:
        docker = FakeDocker()

        def relabel(d: FakeDocker) -> None:
            for c in d.containers.values():
                c['labels'][lab.LABEL_RUN] = '0' * 32

        docker.on_stream = relabel
        code, out = self.go(docker)
        self.assertEqual(code, 3)
        self.assertEqual(self.removals(docker), [])
        self.assertEqual(self.state(out)['cleanup']['guard'], 'ownership-mismatch')

    def test_interrupt_still_cleans_up(self) -> None:
        docker = FakeDocker()

        def interrupt(d: FakeDocker) -> None:
            raise lab.Interrupted(15)

        docker.on_stream = interrupt
        code, out = self.go(docker)
        state = self.state(out)
        self.assertEqual((code, state['runner_status']), (2, 'interrupted'))
        self.assertEqual(self.removals(docker), [state['container']['id']])

    def test_preflight_identity_refusals_create_nothing(self) -> None:
        docker = FakeDocker(faults={'image-mismatch'})
        code, _ = self.go(docker, name='image')
        self.assertEqual(code, 2)
        self.assertFalse([c for c in docker.calls if c[0] == 'create'])
        docker = FakeDocker()
        self.helper.write_bytes(b'tampered')
        code, _ = self.go(docker, name='helper')
        self.assertEqual(code, 2)
        self.assertEqual(docker.calls[-1][0], 'ps')
        self.assertFalse([c for c in docker.calls if c[0] in {'create', 'image'}])

    def test_wrong_generation_record_is_inconclusive(self) -> None:
        docker = FakeDocker()

        def foreign(d: FakeDocker) -> None:
            d.records.items[2]['run'] = '9' * 32

        docker.on_stream = foreign
        code, out = self.go(docker)
        self.assertEqual(code, 2)
        self.assertIn('wrong-generation', json.loads((out / 'verdict.json').read_text())['reason'])


if __name__ == '__main__':
    unittest.main()
