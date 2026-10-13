"""Independent adverse cases from Astra F1–F6, plus the relevant controls.

Fixtures are synthetic v2 evidence. Old receipts are never rewritten.
"""
from __future__ import annotations

import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts import kata_lifecycle_lab as lab
from scripts import kata_lifecycle_verdict as verdict
from scripts.tests import kata_lifecycle_fixtures as fx
from scripts.tests import test_kata_lifecycle_lab as lab_tests
from scripts.tests.test_kata_lifecycle_lab import FakeDocker

RUN, CID = '1' * 32, '2' * 64


class OracleCorrections(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.count = 0

    def case(self, mutate=lambda a, s, r: None, scenario='idle') -> dict:
        arm = fx.stop_arm(scenario)
        state = fx.run_state(RUN, arm, CID)
        records = fx.stop_records(RUN, CID, scenario)
        mutate(arm, state, records)
        self.count += 1
        out = fx.write_run(Path(self.tmp.name) / str(self.count), arm, state, records.lines())
        return verdict.classify(out)

    @staticmethod
    def record(records: fx.Records, kind: str) -> dict:
        return next(r for r in records.items if r['kind'] == kind)

    def test_controls_preserve_purpose_specific_claims(self) -> None:
        for scenario, success in [('idle', True), ('held-starved', False), ('held-liveness-only', False), ('stranded-frozen', False)]:
            with self.subTest(scenario=scenario):
                result = self.case(scenario=scenario)
                self.assertEqual(result['verdict'], 'PASS', result)
                self.assertEqual(result['claims']['lifecycle_success'], success)

    def test_F1_both_success_purposes_reject_release_dependent_stop(self) -> None:
        for purpose in ('candidate-qualification', 'positive-control'):
            result = self.case(lambda a, s, r: a.update(purpose=purpose), 'held-liveness-only')
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
            self.assertIn('stop_before_release', result['reason'])

    def test_F2_live_pid_and_descendant_population_fail(self) -> None:
        for mutate in (
            lambda final: final['tasks'].update({'300': 'D:__fuse_simple_request'}),
            lambda final: final['cgroup_state'].update(populated='1'),
            lambda final: final['cgroup_state'].update(populated='1', all_procs=[301], descendants={
                'child': {'freeze': '0', 'frozen': '0', 'populated': '1', 'procs': [301]}}),
        ):
            result = self.case(lambda a, s, r: mutate(self.record(r, 'observation')))
            self.assertEqual(result['verdict'], 'FAIL', result)
            self.assertFalse(result['claims']['lab_stop_completion'])
            self.assertFalse(result['claims']['lifecycle_success'])

    def test_F2_failed_final_health_fails(self) -> None:
        def mutate(a, s, r):
            self.record(r, 'observation')['check']['outcome'] = 'deadline'
        result = self.case(mutate)
        self.assertEqual(result['verdict'], 'FAIL', result)
        self.assertFalse(result['claims']['lifecycle_success'])

    def test_F2_unknown_or_stale_observations_are_inconclusive(self) -> None:
        changes = [
            lambda f: f.update(t=fx.stamp(1)),
            lambda f: f.update(started_t=fx.stamp(10.01)),
            lambda f: f['tasks'].update({'300': 'unknown:permission denied'}),
            lambda f: f.update(tasks={}),
            lambda f: f['cgroup_state'].update(error='I/O error'),
            lambda f: f['cgroup_state'].pop('descendants'),
            lambda f: f.update(agent_exe_sha256='e' * 64),
            lambda f: f.update(agent_exe_error='permission denied'),
            lambda f: f.update(cgroup_state={'cgroup': 'absent', 'path': f['cgroup_state']['path']},
                               unit={'FreezerState': 'frozen', 'ActiveState': 'inactive'}),
        ]
        for change in changes:
            result = self.case(lambda a, s, r: change(self.record(r, 'observation')))
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)

    def before_release(self, arm, state, records, snapshot=True) -> None:
        arm.update(purpose='candidate-qualification')
        arm['expect'].update(init='exited-promptly', stop_before_release='completed')
        wait = self.record(records, 'wait-returned')
        wait.update(t=fx.stamp(10.05), received_t=fx.stamp(10.05), latency_ms=7950.0)
        if snapshot:
            terminal = copy.deepcopy(self.record(records, 'observation'))
            for key in ('schema', 'run', 'source', 'seq', 't', 'kind'):
                terminal.pop(key)
            terminal.update(started_t=fx.stamp(10.06), completed_t=fx.stamp(10.07))
            terminal['check'].update(conn='B', stream=3, sent_t=fx.stamp(10.069), received_t=fx.stamp(10.0694))
            for r in records.items:
                if r.get('conn') == 'B' and r.get('method') == 'Check' and r['sent_t'] > fx.stamp(10.069):
                    r['stream'] += 2
            records.add('client', 'terminal-observation', 10.08, **terminal)
        records.items.sort(key=lambda r: r['t'])
        records.drop(lambda r: False)

    def test_F2_wait_reply_alone_cannot_prove_before_release(self) -> None:
        result = self.case(lambda a, s, r: self.before_release(a, s, r, snapshot=False), 'held-liveness-only')
        self.assertEqual(result['verdict'], 'FAIL', result)
        self.assertFalse(result['claims']['stop_completed_before_release'])
        result = self.case(self.before_release, 'held-liveness-only')
        self.assertEqual(result['verdict'], 'PASS', result)
        self.assertTrue(result['claims']['lifecycle_success'])

    def test_F3_required_fields_and_operation_binding(self) -> None:
        changes = [
            lambda a, s, r: s.update(steps=[]),
            lambda a, s, r: s['steps'][0].update(exit=9),
            lambda a, s, r: s['inputs'].update(image_id='sha256:' + 'f' * 64),
            lambda a, s, r: self.record(r, 'signal-sent').update(signal=9),
            lambda a, s, r: self.record(r, 'signal-sent').update(exec_id=CID),
            lambda a, s, r: self.record(r, 'identity').update(agent_pid=None, agent_pids=[None]),
            lambda a, s, r: self.record(r, 'identity').update(agent_pid=1, agent_pids=[True]),
            lambda a, s, r: self.record(r, 'wait-returned').pop('status'),
            lambda a, s, r: self.record(r, 'wait-returned').update(status=True),
            lambda a, s, r: self.record(r, 'wait-returned').update(exec_id='3' * 64),
            lambda a, s, r: self.record(r, 'signal-returned').update(submission='uncertain'),
            lambda a, s, r: self.record(r, 'signal-returned').update(operation='other-stop'),
            lambda a, s, r: self.record(r, 'check').update(latency_ms=float('nan')),
            lambda a, s, r: self.record(r, 'check').update(health_status=0),
            lambda a, s, r: self.record(r, 'check').update(conn='unqualified-third-connection'),
            lambda a, s, r: s['steps'].pop(),
            lambda a, s, r: r.drop(lambda record: record['kind'] == 'prepared'),
            lambda a, s, r: next(record for record in r.items if record.get('step') == 'start-container').update(t=fx.stamp(0.1)),
        ]
        for change in changes:
            result = self.case(change)
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)

    def test_F3_zero_hold_and_unknown_check_are_not_negative_proof(self) -> None:
        def zero(a, s, r):
            for record in r.items:
                if record['kind'] in {'held-request', 'read-received', 'read-released'}:
                    record['unique'] = 0
                    if 'pid' in record:
                        record['pid'] = 0
        def unknown(a, s, r):
            for record in r.items:
                if record['kind'] == 'check' and record['sent_t'] >= fx.stamp(10):
                    record['outcome'] = 'invalid-starvation-token'
        for change in (zero, unknown):
            self.assertEqual(self.case(change, 'held-starved')['verdict'], 'INCONCLUSIVE')

    def test_F3_order_requires_daemon_correlation_and_case_order(self) -> None:
        arm = fx.order_arm()
        for mutation in ('missing-daemon', 'wrong-unique', 'premature-release', 'stale-command'):
            records = fx.order_records(RUN)
            if mutation == 'missing-daemon':
                records.drop(lambda r: r['source'] == 'fused' and r['kind'] == 'read-received')
            elif mutation == 'wrong-unique':
                self.record(records, 'read-received')['unique'] = 0
            elif mutation == 'premature-release':
                self.record(records, 'read-released')['t'] = fx.stamp(2.4)
            else:
                next(r for r in records.items if r.get('step') == 'thaw-returned')['t'] = fx.stamp(1)
            self.count += 1
            out = fx.write_run(Path(self.tmp.name) / str(self.count), arm, fx.run_state(RUN, arm, CID), records.lines())
            self.assertEqual(verdict.classify(out)['verdict'], 'INCONCLUSIVE')

    def test_legacy_schema_is_not_silently_promoted(self) -> None:
        result = self.case(lambda a, s, r: s.update(schema='finite.kata-lifecycle-run.v1'))
        self.assertEqual(result['verdict'], 'INCONCLUSIVE')
        self.assertEqual(result['reason'], 'run.json schema')


class LateCreate(FakeDocker):
    def run(self, args, timeout):
        if args[0] == 'create':
            self.calls.append(args)
            self.delayed = {'name': args[args.index('--name') + 1], 'labels': dict(a.split('=', 1) for a in args if a.startswith('computer.finite.lab.'))}
            return lab.Result(None, '', '', True)
        return super().run(args, timeout)


class WrongIdentity(FakeDocker):
    def __init__(self, changes):
        super().__init__()
        self.changes = changes

    def exec_(self, cid, argv):
        result = super().exec_(cid, argv)
        if argv[:2] == [lab.HELPER, 'identity']:
            record = json.loads(result.stdout)
            record.update(self.changes)
            return lab.Result(0, json.dumps(record) + '\n', '')
        return result


class RunnerCorrections(unittest.TestCase):
    setUp = lab_tests.LabTests.setUp
    go = lab_tests.LabTests.go
    state = lab_tests.LabTests.state
    removals = lab_tests.LabTests.removals

    def test_F4_delayed_invisible_create_retains_nonce_and_exits_3(self) -> None:
        engine = LateCreate()
        code, out = self.go(engine)
        state = self.state(out)
        engine.containers['e' * 64] = engine.delayed
        self.assertEqual(code, 3)
        self.assertEqual(state['creation']['nonce'], state['run'])
        self.assertTrue(state['creation']['reconciliation_required'])
        self.assertEqual(state['cleanup']['guard'], 'create-unresolved')
        self.assertFalse(state['cleanup']['removed'])
        self.assertIsNone(state['cleanup']['audit']['inspect_absent'])
        self.assertFalse((out / 'verdict.json').exists())
        self.assertEqual(self.removals(engine), [])

    def test_F4_interruption_during_create_does_not_certify_absence(self) -> None:
        class InterruptCreate(FakeDocker):
            def run(self, args, timeout):
                if args[0] == 'create':
                    self.calls.append(args)
                    raise lab.Interrupted(15)
                return super().run(args, timeout)
        code, out = self.go(InterruptCreate())
        self.assertEqual(code, 3)
        self.assertTrue(self.state(out)['creation']['reconciliation_required'])

    def test_F5_expired_before_execute_admits_no_experiment(self) -> None:
        original = lab.LabRun.__init__
        def expired(run, *args, **kwargs):
            original(run, *args, **kwargs)
            run.deadline = 0
        engine = FakeDocker()
        with mock.patch.object(lab.LabRun, '__init__', expired):
            code, out = self.go(engine)
        self.assertEqual(code, 2)
        self.assertFalse([call for call in engine.calls if call[0] in {'image', 'create', 'start', 'cp', 'exec'}])
        self.assertIn('experiment budget exhausted', self.state(out)['refused'])

    def test_F5_expiry_between_phases_still_diagnoses_and_cleans(self) -> None:
        for phase in ('create', 'start', 'identity', 'prepare', 'inner'):
            now = [0.0]
            class Expiring(FakeDocker):
                def run(self, args, timeout):
                    result = super().run(args, timeout)
                    if args[0] == phase or (args[0] == 'exec' and args[2:4] == [lab.HELPER, phase]):
                        now[0] = 301.0
                    return result
                def stream(self, args, timeout, on_line):
                    result = super().stream(args, timeout, on_line)
                    if phase == 'inner':
                        now[0] = 301.0
                    return result
            engine = Expiring()
            with mock.patch.object(lab.time, 'monotonic', side_effect=lambda: now[0]):
                code, out = self.go(engine, name=phase)
            state = self.state(out)
            self.assertEqual(code, 2, state)
            self.assertTrue(state['cleanup']['removed'], state)
            self.assertEqual(len(self.removals(engine)), 1)
            if phase in {'create', 'start', 'identity', 'prepare'}:
                self.assertFalse([c for c in engine.calls if c[0] == 'exec' and c[2:4] == [lab.HELPER, 'client']])
            if phase in {'identity', 'prepare', 'inner'}:
                self.assertTrue((out / 'diagnostics').exists())

    def test_F5_invalid_bounds_refused_before_engine_calls(self) -> None:
        for key, value in [('total_s', 0), ('total_s', float('nan')), ('exec_s', -1), ('settle_s', 31), ('ready_s', True)]:
            arm = fx.stop_arm('idle', bounds={key: value})
            with self.assertRaises(verdict.Inconclusive):
                verdict.validate_arm(arm)

    def test_F5_readiness_uses_remaining_phase_budget(self) -> None:
        now = [0.0]
        timeouts = []
        class SlowReady(FakeDocker):
            def run(self, args, timeout):
                result = super().run(args, timeout)
                if args[:1] == ['exec'] and args[2:4] == ['test', '-S']:
                    timeouts.append(timeout)
                    now[0] += timeout
                    return lab.Result(None, '', '', True)
                return result
        engine = SlowReady()
        with mock.patch.object(lab.time, 'monotonic', side_effect=lambda: now[0]):
            code, out = self.go(engine)
        self.assertEqual(code, 2)
        self.assertEqual(timeouts, [0.2])
        self.assertTrue(self.state(out)['cleanup']['removed'])

    def test_timed_out_lookup_is_not_absence_evidence(self) -> None:
        self.assertFalse(lab.missing_container(lab.Result(-9, '', 'No such container', True)))

    def test_F6_identity_rejected_before_prepare_or_client(self) -> None:
        cases = [dict(agent_serves_socket=False), dict(agent_pid=None, agent_pids=[None]), dict(run='e' * 32),
                 dict(schema='finite.kata-lifecycle-record.v1'), dict(agent_log_level_env_set=True, agent_log_level_env='debug'),
                 dict(systemd_version='wrong'), dict(observation_error='permission denied')]
        for index, change in enumerate(cases):
            engine = WrongIdentity(change)
            code, out = self.go(engine, name=str(index))
            self.assertEqual(code, 2)
            self.assertFalse([call for call in engine.calls if call[0] == 'exec' and call[2:4] in [[lab.HELPER, 'prepare'], [lab.HELPER, 'client']]])
            self.assertTrue(self.state(out)['cleanup']['removed'])


if __name__ == '__main__':
    unittest.main()
