from __future__ import annotations

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from scripts import kata_lifecycle_verdict as verdict
from scripts.tests import kata_lifecycle_fixtures as fx

RUN = '1' * 32
CID = '2' * 64


class VerdictTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.count = 0

    def classify(self, arm: dict, lines: list[str], **state: object) -> dict:
        self.count += 1
        directory = fx.write_run(Path(self.tmp.name) / str(self.count), arm, fx.run_state(RUN, arm, CID, **state), lines)
        return verdict.classify(directory)

    def stop(self, scenario: str, **arm_overrides: object) -> tuple[dict, list[str]]:
        return fx.stop_arm(scenario, **arm_overrides), fx.stop_records(RUN, CID, scenario).lines()

    def assertVerdict(self, result: dict, expected: str, reason: str | None = None) -> None:
        self.assertEqual(result['verdict'], expected, result)
        if reason is not None:
            self.assertIn(reason, result.get('reason', ''))

    def test_positive_control_passes_with_lifecycle_success(self) -> None:
        result = self.classify(*self.stop('idle'))
        self.assertVerdict(result, 'PASS')
        self.assertEqual(result['claims'], {'health_liveness': True, 'lab_stop_completion': True, 'signal_ok': True,
                                            'stop_completed_before_release': None, 'correct_after_release': True,
                                            'lifecycle_success': True})

    def test_expected_negative_match_is_not_lifecycle_success(self) -> None:
        result = self.classify(*self.stop('held-starved'))
        self.assertVerdict(result, 'PASS')
        self.assertFalse(result['claims']['health_liveness'])
        self.assertTrue(result['claims']['lab_stop_completion'])
        self.assertFalse(result['claims']['lifecycle_success'])

    def test_liveness_without_stop_completion_is_distinguished(self) -> None:
        result = self.classify(*self.stop('held-liveness-only'))
        self.assertVerdict(result, 'PASS')
        self.assertEqual(result['observed']['same_connection'], 'responsive')
        self.assertEqual(result['observed']['stop_before_release'], 'incomplete')
        # Fast signal and live Checks do not mean the stop finished: init was
        # reaped only after the held request was answered.
        self.assertTrue(result['claims']['health_liveness'])
        self.assertTrue(result['claims']['correct_after_release'])
        self.assertFalse(result['claims']['stop_completed_before_release'])
        self.assertFalse(result['claims']['lifecycle_success'])

    def test_worker_handoff_shape_is_not_lifecycle_success(self) -> None:
        result = self.classify(*self.stop('stranded-frozen'))
        self.assertVerdict(result, 'PASS')
        self.assertTrue(result['claims']['health_liveness'])
        self.assertFalse(result['claims']['lab_stop_completion'])
        self.assertFalse(result['claims']['lifecycle_success'])

    def test_expectation_mismatch_fails(self) -> None:
        arm, lines = self.stop('held-starved')
        arm['expect']['same_connection'] = 'responsive'
        result = self.classify(arm, lines)
        self.assertVerdict(result, 'FAIL')
        self.assertEqual(result['mismatches'], {'same_connection': {'expected': 'responsive', 'observed': 'starved'}})

    def test_candidate_arm_cannot_accept_liveness_only(self) -> None:
        arm, lines = self.stop('stranded-frozen', purpose='candidate-qualification')
        self.assertVerdict(self.classify(arm, lines), 'INCONCLUSIVE', 'must expect lifecycle success')

    def test_expected_negative_arm_must_predict_a_negative(self) -> None:
        arm, lines = self.stop('idle', purpose='expected-negative')
        self.assertVerdict(self.classify(arm, lines), 'INCONCLUSIVE', 'predicts no negative outcome')

    def test_debug_log_level_arm_is_refused(self) -> None:
        arm, lines = self.stop('idle')
        arm['identity']['agent_log_level'] = 'debug'
        self.assertVerdict(self.classify(arm, lines), 'INCONCLUSIVE', 'default agent log level')

    def mutated(self, scenario: str, mutate) -> dict:
        arm = fx.stop_arm(scenario)
        rec = fx.stop_records(RUN, CID, scenario)
        mutate(rec)
        return self.classify(arm, [json.dumps(r) for r in rec.items])

    def test_evidence_integrity_failures_are_inconclusive(self) -> None:
        def kind(name):
            return lambda rec: [r for r in rec.items if r['kind'] == name]

        cases = {
            'wrong-generation': ('idle', lambda rec: rec.items[3].update(run='9' * 32)),
            'sequence gap': ('idle', lambda rec: rec.items.pop(4)),
            'serving agent executable mismatch': ('idle', lambda rec: kind('identity')(rec)[0].update(agent_exe_sha256='e' * 64)),
            'not the default': ('idle', lambda rec: kind('identity')(rec)[0].update(agent_log_level_env_set=True, agent_log_level_env='debug')),
            'final observation missing': ('idle', lambda rec: rec.drop(lambda r: r['kind'] == 'observation')),
            'safety bound': ('held-starved', lambda rec: kind('read-released')(rec)[0].update(by='bound')),
            'not held before the signal': ('held-starved', lambda rec: kind('read-received')(rec)[0].update(t=fx.stamp(10.5))),
            'no check evidence': ('idle', lambda rec: rec.drop(lambda r: r['kind'] in {'check-started', 'check'}
                                                               and r['conn'] == 'A' and r['sent_t'] > fx.stamp(5))),
            'setup or precondition failed': ('idle', lambda rec: rec.add('client', 'setup-error', 30.0, step='create-container')),
            'unit FreezerState disagrees': ('idle', lambda rec: kind('observation')(rec)[0]['unit'].update(FreezerState='frozen')),
            'signal result missing': ('idle', lambda rec: rec.drop(lambda r: r['kind'] == 'signal-returned')),
        }
        for reason, (scenario, mutate) in cases.items():
            with self.subTest(reason=reason):
                result = self.mutated(scenario, mutate)
                self.assertVerdict(result, 'INCONCLUSIVE')
                self.assertIn(reason, result['reason'])

    # A begun Check that never answered was still waiting at client end, so its
    # elapsed wait is known: at or above starved_ms it is starvation, below it
    # the evidence is censored. Absent attempts are missing evidence. In 'idle',
    # A's in-window Checks start at 13 s and 18 s and the client ends at 24 s.
    @staticmethod
    def unanswered_A(rec) -> None:
        rec.drop(lambda r: r['kind'] == 'check' and r['conn'] == 'A' and r['sent_t'] > fx.stamp(5))

    def test_unanswered_check_with_long_known_wait_fails(self) -> None:
        result = self.mutated('idle', self.unanswered_A)
        self.assertVerdict(result, 'FAIL')
        self.assertEqual(result['mismatches']['same_connection'], {'expected': 'responsive', 'observed': 'starved'})
        self.assertFalse(result['claims']['health_liveness'])
        self.assertFalse(result['claims']['lifecycle_success'])

    def test_short_censored_or_missing_attempt_evidence_is_inconclusive(self) -> None:
        rec = fx.stop_records(RUN, CID, 'idle')
        self.unanswered_A(rec)
        short = fx.stop_arm('idle', thresholds={'starved_ms': 12000.0})  # Known waits: 11 s and 6 s.
        self.assertVerdict(self.classify(short, rec.lines()), 'INCONCLUSIVE', 'censored')
        unattempted = self.mutated('idle', lambda records: records.drop(
            lambda r: r['kind'] == 'check-started' and r['conn'] == 'A' and r['sent_t'] > fx.stamp(5)))
        self.assertVerdict(unattempted, 'INCONCLUSIVE', 'without attempted request')

    def test_wrong_typed_fields_never_crash_or_pass(self) -> None:
        # Each field of each record replaced by null must yield a verdict, never
        # an exception; required fields must not PASS.
        base = fx.stop_records(RUN, CID, 'held-starved')
        arm = fx.stop_arm('held-starved')
        reserved = {'schema', 'run', 'source', 'seq', 't', 'kind'}
        required = {'kernel_agent_params': 'identity', 'pending': 'client-end', 'cgroup_state': 'observation'}
        for index, record in enumerate(base.items):
            for field in sorted(set(record) - reserved):
                lines = [json.dumps(r) for r in base.items]
                broken = dict(record, **{field: None})
                lines[index] = json.dumps(broken)
                result = self.classify(arm, lines)
                self.assertIn(result['verdict'], {'PASS', 'FAIL', 'INCONCLUSIVE'})
                if required.get(field) == record['kind']:
                    self.assertEqual(result['verdict'], 'INCONCLUSIVE', (record['kind'], field))

    def test_malformed_and_empty_records_are_inconclusive(self) -> None:
        arm, lines = self.stop('idle')
        self.assertVerdict(self.classify(arm, lines[:5] + ['{not json'] + lines[5:]), 'INCONCLUSIVE', 'malformed record')
        self.assertVerdict(self.classify(arm, []), 'INCONCLUSIVE', 'zero records')

    def test_runner_failures_propagate_as_inconclusive(self) -> None:
        arm, lines = self.stop('idle')
        steps = [{'name': 'inner', 'status': 'failed', 'exit': 10}]
        self.assertVerdict(self.classify(arm, lines, steps=steps), 'INCONCLUSIVE', 'inner failed (exit 10)')
        leaked = {'attempted': True, 'guard': 'ok', 'removed': False, 'audit': {'inspect_absent': False, 'label_matches': ['d' * 64]}}
        result = self.classify(arm, lines, cleanup=leaked)
        self.assertVerdict(result, 'INCONCLUSIVE', 'not cleanly removed')
        self.assertEqual(result['teardown'], 'leaked-or-failed')
        self.assertVerdict(self.classify(arm, lines, runner_status='interrupted'), 'INCONCLUSIVE', 'runner status')

    def test_missing_files_are_inconclusive(self) -> None:
        directory = Path(self.tmp.name) / 'empty'
        directory.mkdir()
        self.assertVerdict(verdict.classify(directory), 'INCONCLUSIVE', 'missing arm.json')

    def test_order_primitives_pass_and_fence_would_fail(self) -> None:
        arm = fx.order_arm()
        self.assertVerdict(self.classify(arm, fx.order_records(RUN).lines()), 'PASS')
        result = self.classify(arm, fx.order_records(RUN, 'fenced').lines())
        self.assertVerdict(result, 'FAIL')
        self.assertEqual(result['observed']['cases']['thaw-before-freeze'], 'fenced')
        self.assertVerdict(self.classify(arm, fx.order_records(RUN, 'missing-step').lines()), 'INCONCLUSIVE', 'missing step')

    def test_cli_exit_codes(self) -> None:
        arm, lines = self.stop('idle')
        directory = fx.write_run(Path(self.tmp.name) / 'cli', arm, fx.run_state(RUN, arm, CID), lines)
        with contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(verdict.main(['x', str(directory)]), 0)
            (directory / 'records.jsonl').write_text('')
            self.assertEqual(verdict.main(['x', str(directory)]), 2)
        self.assertIn('"zero records"', out.getvalue())


if __name__ == '__main__':
    unittest.main()
