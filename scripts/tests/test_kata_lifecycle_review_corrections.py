"""R1/R2 oracle and R4 fake-runner regressions; no real engine/helper runs."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from scripts import kata_lifecycle_lab as lab
from scripts import kata_lifecycle_verdict as verdict
from scripts.tests import kata_lifecycle_fixtures as fx
from scripts.tests.test_kata_lifecycle_lab import FakeDocker

RUN, CID = '1' * 32, '2' * 64


class OracleCases(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.count = 0

    def case(self, mutate=lambda a, r: None, scenario='idle', order=False):
        arm = fx.order_arm() if order else fx.stop_arm(scenario)
        records = fx.order_records(RUN) if order else fx.stop_records(RUN, CID, scenario)
        mutate(arm, records)
        self.count += 1
        path = Path(self.tmp.name) / str(self.count)
        fx.write_run(path, arm, fx.run_state(RUN, arm, CID), records.lines())
        return verdict.classify(path)

    @staticmethod
    def emitted(records):
        records.items.sort(key=lambda r: r['t'])
        records.drop(lambda r: False)

    @staticmethod
    def target(records):
        return next(r for r in records.items if r['kind'] == 'check' and r['conn'] == 'A' and r['sent_t'] == fx.stamp(18))

    def censor(self, arm, records, latency=5999, kind='check-abandoned', outcome='cancelled', remove=False):
        target = self.target(records)
        target.update(kind=kind, outcome=outcome, latency_ms=latency, t=fx.stamp(18 + latency / 1000), deadline_ms=30000)
        target.pop('health_status')
        target.pop('received_t', None)
        if remove:
            records.items.remove(target)
        self.emitted(records)


class ReviewOracle(OracleCases):
    def test_R1_latest_slow_cancel_is_failure_despite_final_health(self):
        result = self.case(self.censor)
        self.assertEqual(result['verdict'], 'FAIL', result)
        self.assertEqual(result['observed']['same_connection'], 'starved')
        self.assertTrue(result['observed']['final_health'])
        self.assertFalse(result['claims']['health_liveness'])
        self.assertFalse(result['claims']['lifecycle_success'])

    def test_R1_pending_censored_and_abandoned_preserve_elapsed_wait(self):
        for kind, outcome in [('check-abandoned', 'cancelled'), ('check-pending', 'pending'), ('check-censored', 'censored')]:
            result = self.case(lambda a, r: self.censor(a, r, kind=kind, outcome=outcome))
            self.assertEqual(result['verdict'], 'FAIL', result)
            self.assertEqual(result['observed']['same_connection'], 'starved')
        result = self.case(lambda a, r: self.censor(a, r, remove=True))
        self.assertEqual(result['verdict'], 'FAIL', result)
        self.assertFalse(result['claims']['health_liveness'])

    def test_R1_short_cancel_is_insufficient_not_known_starvation(self):
        result = self.case(lambda a, r: self.censor(a, r, latency=200))
        self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
        self.assertIn('censored', result['reason'])

    def test_R1_send_time_boundary_and_outside_window_controls(self):
        def extra(arm, records, sent, with_result):
            stream = max(r.get('stream', 0) for r in records.items if r.get('conn') == 'A') + 2
            records.add('client', 'check-started', sent, conn='A', request_id=100, sent_t=fx.stamp(sent),
                        cid=CID, operation=RUN + ':stop:' + CID)
            if with_result:
                records.add('client', 'check-abandoned', 23.999, conn='A', request_id=100,
                            sent_t=fx.stamp(sent), method='Check', stream=stream, code=0, submission='submitted',
                            outcome='cancelled', latency_ms=(23.999 - sent) * 1000,
                            cid=CID, operation=RUN + ':stop:' + CID)
            self.emitted(records)
        for with_result in (True, False):
            outside = self.case(lambda a, r: extra(a, r, 20.001, with_result))
            self.assertEqual(outside['verdict'], 'PASS', outside)
            self.assertTrue(outside['claims']['health_liveness'])
            boundary = self.case(lambda a, r: extra(a, r, 20.0, with_result))
            self.assertEqual(boundary['verdict'], 'FAIL', boundary)
            self.assertFalse(boundary['claims']['health_liveness'])

    def test_R1_expected_negative_remains_prediction_match(self):
        result = self.case(scenario='held-starved')
        self.assertEqual(result['verdict'], 'PASS', result)
        self.assertFalse(result['claims']['lifecycle_success'])
        self.assertFalse(result['claims']['health_liveness'])

    def test_R2_each_current_contradiction_rejects_independently(self):
        def reuse(arm, records):
            signal = next(r for r in records.items if r['kind'] == 'signal-returned')
            next(r for r in records.items if r['kind'] == 'wait-returned')['stream'] = signal['stream']
        def late_identity(arm, records):
            next(r for r in records.items if r['kind'] == 'identity')['t'] = fx.stamp(27)
            self.emitted(records)
        def outside_scope(arm, records):
            held = next(r for r in records.items if r['kind'] == 'held-request')
            held['pid'] = 900
            next(r for r in records.items if r['kind'] == 'read-received' and r['unique'] == held['unique'])['pid'] = 900
        for mutation, order, reason in [(reuse, False, 'stream reused'), (late_identity, False, 'preflight'),
                                        (outside_scope, True, 'held PID outside')]:
            result = self.case(mutation, order=order)
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
            self.assertIn(reason, result['reason'])

    def test_R2_producer_style_unique_stream_controls(self):
        records = fx.stop_records(RUN, CID, 'idle')
        calls = [r for r in records.items if r['source'] == 'client' and 'method' in r]
        keys = [(r['conn'], r['stream']) for r in calls]
        self.assertEqual(len(keys), len(set(keys)))
        for conn in ('A', 'B'):
            ordered = sorted((r for r in calls if r['conn'] == conn), key=lambda r: r['sent_t'])
            self.assertEqual([r['stream'] for r in ordered], list(range(1, len(ordered) * 2, 2)))
        self.assertEqual(self.case()['verdict'], 'PASS')
        self.assertEqual(self.case(order=True)['verdict'], 'PASS')

    def test_R2_same_rpc_later_observation_is_consistent_not_a_new_request(self):
        def repeat(arm, records, inconsistent=False):
            wait = next(r for r in records.items if r['kind'] == 'wait-returned')
            wait['received_t'] = wait['t']
            response = copy.deepcopy(wait)
            response.update(kind='late-response', t=fx.stamp(23))
            if inconsistent:
                response['status'] = 7
            records.items.append(response)
            self.emitted(records)
        self.assertEqual(self.case(repeat)['verdict'], 'PASS')
        result = self.case(lambda a, r: repeat(a, r, True))
        self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
        self.assertIn('contradictory', result['reason'])

    def test_R2_local_check_cancel_and_later_genuine_reply_share_one_request(self):
        def later(arm, records):
            original = self.target(records)
            reply = copy.deepcopy(original)
            self.censor(arm, records, latency=100)
            reply.update(kind='late-response', t=fx.stamp(18.25), received_t=fx.stamp(18.2), latency_ms=200)
            records.items.append(reply)
            self.emitted(records)
        result = self.case(later)
        self.assertEqual(result['verdict'], 'PASS', result)
        self.assertTrue(result['claims']['health_liveness'])


class SnapshotChecks(OracleCases):
    """Producer-shaped direct Check calls, independent of wrapper starts/IDs."""

    @staticmethod
    def final_record(records):
        return next(r for r in records.items if r['source'] == 'observe')

    def snapshot(self, records, sent=10.1, latency=0.4, outcome='ok'):
        final = self.final_record(records)
        terminal = copy.deepcopy(final)
        terminal.update(source='client', kind='terminal-observation',
                        started_t=fx.stamp(sent - 0.01), completed_t=fx.stamp(sent + latency / 1000 + 0.01),
                        t=fx.stamp(sent + latency / 1000 + 0.02))
        terminal.pop('label', None)
        # Allocate the snapshot's stream on B in actual send order. No
        # check-started or request_id is emitted by this direct conn.call.
        earlier = [r['stream'] for r in records.items if r.get('conn') == 'B' and 'stream' in r
                   and r.get('sent_t', '') < fx.stamp(sent)]
        terminal['check'].update(conn='B', stream=max(earlier, default=-1) + 2,
                                 sent_t=fx.stamp(sent), latency_ms=latency, outcome=outcome)
        for r in records.items:
            if r.get('conn') == 'B' and r.get('sent_t', '') >= fx.stamp(sent) and 'stream' in r:
                r['stream'] += 2
        if outcome == 'ok':
            terminal['check']['received_t'] = fx.stamp(sent + latency / 1000)
        else:
            terminal['check'].pop('received_t', None)
            terminal['check'].pop('health_status', None)
        end = next(r for r in records.items if r['kind'] == 'client-end')
        if terminal['t'] >= end['t']:
            end['t'] = fx.stamp(sent + latency / 1000 + 0.1)
            final.update(started_t=fx.stamp(sent + latency / 1000 + 1),
                         completed_t=fx.stamp(sent + latency / 1000 + 1.5), t=fx.stamp(sent + latency / 1000 + 2))
            final['check'].update(sent_t=fx.stamp(sent + latency / 1000 + 1.4),
                                  received_t=fx.stamp(sent + latency / 1000 + 1.4004))
        records.items.append(terminal)
        self.emitted(records)
        return terminal

    def test_V3_1_direct_snapshot_prompt_and_known_slow(self):
        prompt = self.case(lambda a, r: self.snapshot(r))
        self.assertEqual(prompt['verdict'], 'PASS', prompt)
        self.assertTrue(prompt['claims']['lifecycle_success'])
        slow = self.case(lambda a, r: self.snapshot(r, latency=4000))
        self.assertEqual(slow['verdict'], 'FAIL', slow)
        self.assertEqual(slow['observed']['independent_connection'], 'starved')
        self.assertTrue(slow['observed']['final_health'])
        self.assertFalse(slow['claims']['health_liveness'])
        self.assertFalse(slow['claims']['lifecycle_success'])
        def predicted(arm, records):
            arm.update(purpose='expected-negative')
            arm['expect']['independent_connection'] = 'starved'
            self.snapshot(records, latency=4000)
        negative = self.case(predicted)
        self.assertEqual(negative['verdict'], 'PASS', negative)
        self.assertFalse(negative['claims']['lifecycle_success'])

    def test_V3_1_direct_censored_short_is_insufficient_long_is_starved(self):
        for outcome in ('cancelled', 'pending', 'censored'):
            with self.subTest(outcome=outcome):
                short = self.case(lambda a, r: self.snapshot(r, latency=200, outcome=outcome))
                self.assertEqual(short['verdict'], 'INCONCLUSIVE', short)
                self.assertIn('censored', short['reason'])
                long = self.case(lambda a, r: self.snapshot(r, latency=4000, outcome=outcome))
                self.assertEqual(long['verdict'], 'FAIL', long)
                self.assertFalse(long['claims']['health_liveness'])

    def test_V3_1_actual_local_return_without_send_metadata_stays_unknown(self):
        # callWithMeta's local cancellation/write/admission-error returns
        # omit sentAt. snapshot() does not supply the wrapper's override.
        for latency in (200, 4000):
            def missing(arm, records):
                terminal = self.snapshot(records, latency=latency, outcome='cancelled')
                terminal['check'].pop('sent_t')
            result = self.case(missing)
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)

    def test_V3_1_nested_send_interval_is_inclusive_with_outside_control(self):
        for sent, expected in [(20.0, 'FAIL'), (20.001, 'PASS')]:
            result = self.case(lambda a, r: self.snapshot(r, sent=sent, latency=4000))
            self.assertEqual(result['verdict'], expected, result)
            self.assertEqual(result['claims']['health_liveness'], expected == 'PASS')
        def at_start(arm, records):
            signal = next(r for r in records.items if r['kind'] == 'signal-returned')
            signal.update(t=fx.stamp(10), received_t=fx.stamp(10), latency_ms=0)
            wait = next(r for r in records.items if r['kind'] == 'wait-returned')
            wait.update(t=fx.stamp(10), received_t=fx.stamp(10), latency_ms=7900)
            terminal = self.snapshot(records, sent=10, latency=4000)
            terminal['started_t'] = fx.stamp(10)
        result = self.case(at_start)
        self.assertEqual(result['verdict'], 'FAIL', result)
        self.assertFalse(result['claims']['health_liveness'])

    def test_V3_1_repeated_snapshot_reply_has_no_wrapper_id_or_new_request(self):
        def repeat(arm, records):
            terminal = self.snapshot(records)
            response = copy.deepcopy(terminal['check'])
            self.assertNotIn('request_id', response)
            records.add('client', 'late-response', 23, **response, cid=CID, operation=RUN + ':stop:' + CID)
            self.emitted(records)
        result = self.case(repeat)
        self.assertEqual(result['verdict'], 'PASS', result)
        self.assertTrue(result['claims']['health_liveness'])

    def test_V3_1_reused_stream_and_conflicting_remote_replies_reject(self):
        def reuse(arm, records):
            self.snapshot(records)['check']['stream'] = 1  # B readiness owns it.
        self.assertEqual(self.case(reuse)['verdict'], 'INCONCLUSIVE')
        def conflicting(arm, records):
            terminal = self.snapshot(records)
            response = copy.deepcopy(terminal['check'])
            response.update(outcome='not-serving', health_status=2)
            records.add('client', 'late-response', 23, **response, cid=CID, operation=RUN + ':stop:' + CID)
            self.emitted(records)
        result = self.case(conflicting)
        self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
        self.assertIn('contradictory', result['reason'])

    def test_V3_1_later_prompt_receipt_does_not_erase_long_local_wait(self):
        def later(arm, records, elapsed):
            terminal = self.snapshot(records, latency=elapsed, outcome='cancelled')
            response = copy.deepcopy(terminal['check'])
            response.update(outcome='ok', health_status=1, latency_ms=0.4, received_t=fx.stamp(10.1004))
            records.add('client', 'late-response', 23, **response, cid=CID, operation=RUN + ':stop:' + CID)
            self.emitted(records)
        for elapsed, expected in [(200, 'PASS'), (4000, 'FAIL')]:
            result = self.case(lambda a, r: later(a, r, elapsed))
            self.assertEqual(result['verdict'], expected, result)
            self.assertEqual(result['claims']['health_liveness'], expected == 'PASS')

    def test_V3_1_local_deadline_survives_later_reply_under_raised_threshold(self):
        # B's 5 s wrapper deadline is answered 5.5 s after send. With starved_ms
        # at 6 s only the retained local deadline, not the wait, is starvation.
        def late_reply(arm, records, local):
            arm['thresholds'] = {'starved_ms': 6000.0}
            check = next(r for r in records.items if r['kind'] == 'check' and r['conn'] == 'B' and r['sent_t'] == fx.stamp(15))
            reply = copy.deepcopy(check)
            check.update(outcome=local, latency_ms=5000.0, deadline_ms=5000.0, t=fx.stamp(20))
            check.pop('health_status')
            check.pop('received_t')
            reply.update(kind='late-response', latency_ms=5500.0, received_t=fx.stamp(20.5), t=fx.stamp(20.5))
            records.items.append(reply)
            self.emitted(records)
        deadline = self.case(lambda a, r: late_reply(a, r, 'deadline'))
        self.assertEqual(deadline['observed']['independent_connection'], 'starved', deadline)
        self.assertEqual(deadline['observed']['late_responses'], 1)
        self.assertFalse(deadline['claims']['health_liveness'])
        cancelled = self.case(lambda a, r: late_reply(a, r, 'cancelled'))
        self.assertEqual(cancelled['observed']['independent_connection'], 'degraded', cancelled)

    def test_V3_1_request_id_less_late_check_only_from_snapshot_connection_B(self):
        # Only snapshot() calls Check without a request ID: on B, or on observe
        # with no late callback. Fable P3a/P3b: no in-window A wrapper Check.
        def direct_late(arm, records, conn=None):
            records.drop(lambda r: r.get('conn') == 'A' and r['kind'] in {'check-started', 'check'}
                         and r['sent_t'] >= fx.stamp(10))
            if conn:
                stream = max(r['stream'] for r in records.items if r.get('conn') == 'A' and 'stream' in r) + 2
                records.add('client', 'late-response', 12.0004, conn=conn, method='Check', stream=stream, code=0,
                            submission='submitted', outcome='ok', health_status=1, latency_ms=0.4,
                            sent_t=fx.stamp(12), received_t=fx.stamp(12.0004), cid=CID, operation=RUN + ':stop:' + CID)
                self.emitted(records)
        absent = self.case(direct_late)
        self.assertEqual(absent['verdict'], 'INCONCLUSIVE', absent)
        self.assertIn('no check evidence', absent['reason'])
        for conn, reason in [('A', 'outside snapshot connection B'), ('observe', 'connection missing')]:
            result = self.case(lambda a, r: direct_late(a, r, conn))
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
            self.assertIn(reason, result['reason'])

    # N4: a request-ID-less late Check on B can only report snapshot()'s own
    # RPC, so it must name a client terminal-observation Check.
    def direct_reply(self, records, stream, sent, received=None, t=None, **fields):
        received = sent + 0.0004 if received is None else received
        reply = records.add('client', 'late-response', received if t is None else t, conn='B', method='Check',
                            stream=stream, code=0, submission='submitted', outcome='ok', health_status=1,
                            latency_ms=round((received - sent) * 1000, 6), sent_t=fx.stamp(sent),
                            received_t=fx.stamp(received), cid=CID, operation=RUN + ':stop:' + CID)
        reply.update(fields)
        self.emitted(records)
        return reply

    @staticmethod
    def absent_B(records):
        records.drop(lambda r: r.get('conn') == 'B' and r['kind'] in {'check-started', 'check'}
                     and r['sent_t'] >= fx.stamp(10))

    def test_N4_saved_orphan_B_reply_without_terminal_snapshot_rejects(self):
        # Opus V4 saved pair: B's in-window wrapper Checks removed, then one
        # request-ID-less late Check on B. Only the observe snapshot exists.
        absent = self.case(lambda a, r: self.absent_B(r))
        self.assertEqual(absent['verdict'], 'INCONCLUSIVE', absent)
        self.assertIn('no check evidence', absent['reason'])
        def orphan(arm, records):
            self.absent_B(records)
            self.direct_reply(records, stream=3, sent=12.0)
        result = self.case(orphan)
        self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
        self.assertIn('without matching terminal snapshot Check', result['reason'])
        self.assertNotIn('claims', result)

    def test_N4_observe_snapshot_check_is_not_a_B_reply_origin(self):
        # observe has its own connection and no late callback. Its Check
        # identity, copied onto B after the liveness window, names no origin.
        def copied(arm, records):
            final = self.final_record(records)['check']
            self.direct_reply(records, stream=final['stream'], sent=25.4)
        result = self.case(copied)
        self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
        self.assertIn('without matching terminal snapshot Check', result['reason'])

    def test_N4_reply_must_name_its_terminal_snapshot_check(self):
        # As in the saved pair, B's later wrapper Checks are absent; the terminal
        # snapshot's own Check (stream 3, sent 10.1 s) is B's in-window evidence.
        def variant(arm, records, reply, snapshot=None, change=lambda check, terminal: None):
            self.absent_B(records)
            terminal = self.snapshot(records, **(snapshot or {}))
            change(terminal['check'], terminal)
            self.direct_reply(records, **reply)
        genuine = dict(stream=3, sent=10.1, t=23)
        not_sent = dict(latency=200, outcome='cancelled')
        for name, args, reason in [
                ('genuine reply', (genuine,), None),
                ('uncertain local submission', (genuine, dict(latency=200, outcome='transport-error'),
                                                lambda c, t: c.update(submission='uncertain')), None),
                ('terminal with live tasks', (genuine, None, lambda c, t: t['tasks'].update({'290': 'S:futex_wait'})), None),
                ('unrelated B call', (dict(stream=5, sent=12.0),), 'without matching terminal snapshot Check'),
                ('same send, other stream', (dict(stream=5, sent=10.1),), 'without matching terminal snapshot Check'),
                ('same stream, other send', (dict(stream=3, sent=10.15),), 'contradicts terminal snapshot Check'),
                ('terminal Check has a wrapper ID', (genuine, None, lambda c, t: c.update(request_id=99)),
                 'contradicts terminal snapshot Check'),
                ('terminal Check never sent', (genuine, not_sent, lambda c, t: c.update(submission='not-sent')),
                 'contradicts terminal snapshot Check'),
                ('reply without a stream', (dict(stream=0, sent=10.1, submission='not-sent', outcome='cancelled'),),
                 'origin identity malformed')]:
            with self.subTest(name):
                result = self.case(lambda a, r: variant(a, r, *args))
                if reason is None:
                    self.assertEqual(result['verdict'], 'PASS', result)
                    self.assertEqual(result['observed']['late_responses'], 1)
                else:
                    self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
                    self.assertIn(reason, result['reason'])
                    self.assertNotIn('claims', result)

    def test_N4_orphan_rejected_even_when_B_is_otherwise_healthy(self):
        # Every B wrapper Check stays prompt and the orphan follows them, after
        # the liveness window. It is rejected, not ignored.
        def orphan(arm, records, terminal):
            if terminal:
                self.snapshot(records)
            stream = max(r['stream'] for r in records.items if r.get('conn') == 'B' and 'stream' in r) + 2
            self.direct_reply(records, stream=stream, sent=23.0)
        for terminal in (False, True):
            with self.subTest(terminal=terminal):
                result = self.case(lambda a, r: orphan(a, r, terminal))
                self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
                self.assertIn('without matching terminal snapshot Check', result['reason'])

    def test_N4_reply_callback_may_precede_its_terminal_record(self):
        # abandon() can report a reply that arrived before cancellation won,
        # before snapshot() returns its record. The local outcome stays
        # cancelled; association does not depend on emission order.
        def reply(arm, records, at):
            terminal = self.snapshot(records, latency=300, outcome='cancelled')
            late = self.direct_reply(records, stream=terminal['check']['stream'], sent=10.1, received=10.3, t=at)
            self.assertEqual(late['t'] < terminal['t'], at < 23)
        for at in (10.35, 23.0):
            with self.subTest(emitted=at):
                result = self.case(lambda a, r: reply(a, r, at))
                self.assertEqual(result['verdict'], 'PASS', result)
                self.assertEqual(result['observed']['late_responses'], 1)

    def test_V3_1_missing_abandoned_and_malformed_nested_shapes_fail_closed(self):
        def bad(arm, records, mutation):
            terminal = self.snapshot(records)
            mutation(terminal)
        for mutation in [lambda t: t.pop('check'), lambda t: t.update(check=None), lambda t: t.update(check=[]),
                         lambda t: t['check'].pop('stream'), lambda t: t['check'].pop('sent_t'),
                         lambda t: t['check'].pop('latency_ms'), lambda t: t['check'].update(latency_ms=True),
                         lambda t: t['check'].update(outcome='abandoned'), lambda t: t['check'].pop('received_t')]:
            result = self.case(lambda a, r: bad(a, r, mutation))
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)

    def test_V3_2_final_check_interval_must_fit_snapshot(self):
        def mutate(arm, records, sent, received):
            health = self.final_record(records)['check']
            health.update(sent_t=fx.stamp(sent), received_t=fx.stamp(received), latency_ms=(received - sent) * 1000)
        for sent, received in [(1.4, 1.4004), (24.9, 24.9004), (25.6, 25.6004), (25.4, 25.6), (25.4, 25.3)]:
            result = self.case(lambda a, r: mutate(a, r, sent, received))
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
        self.assertEqual(self.case()['verdict'], 'PASS')

    def test_V3_2_snapshot_connection_origin_contradictions_reject(self):
        for conn in ('A', 'B', 'unknown'):
            result = self.case(lambda a, r: self.final_record(r)['check'].update(conn=conn))
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
            self.assertIn('supplying connection', result['reason'])
        for conn in ('A', 'observe', 'unknown'):
            result = self.case(lambda a, r: self.snapshot(r)['check'].update(conn=conn))
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
            self.assertIn('supplying connection', result['reason'])

    def test_V3_2_separate_late_evidence_allowed_but_new_enclosure_cannot_reuse_rpc(self):
        def copied(arm, records):
            terminal = self.snapshot(records)
            later = copy.deepcopy(terminal)
            later.update(started_t=fx.stamp(21), completed_t=fx.stamp(21.5), t=fx.stamp(21.6))
            records.items.append(later)
            self.emitted(records)
        result = self.case(copied)
        self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
        self.assertIn('outside enclosure', result['reason'])

    def test_V3_2_before_release_proof_requires_its_own_fresh_check(self):
        def before(arm, records, stale=False):
            arm.update(purpose='candidate-qualification')
            arm['expect'].update(init='exited-promptly', stop_before_release='completed')
            wait = next(r for r in records.items if r['kind'] == 'wait-returned')
            wait.update(t=fx.stamp(10.05), received_t=fx.stamp(10.05), latency_ms=7950)
            terminal = self.snapshot(records)
            if stale:
                terminal['check'].update(sent_t=fx.stamp(1.4), received_t=fx.stamp(1.4004))
            self.emitted(records)
        fresh = self.case(before, scenario='held-liveness-only')
        self.assertEqual(fresh['verdict'], 'PASS', fresh)
        self.assertTrue(fresh['claims']['stop_completed_before_release'])
        stale = self.case(lambda a, r: before(a, r, True), scenario='held-liveness-only')
        self.assertEqual(stale['verdict'], 'INCONCLUSIVE', stale)

    def test_V3_2_final_check_pending_missing_and_enclosure_fields_reject(self):
        for field in ('check', 'started_t', 'completed_t'):
            result = self.case(lambda a, r: self.final_record(r).pop(field))
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
        for outcome in ('pending', 'cancelled', 'censored'):
            def incomplete(arm, records):
                health = self.final_record(records)['check']
                health.update(outcome=outcome)
                health.pop('received_t')
                health.pop('health_status')
            result = self.case(incomplete)
            self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)

    def test_V3_2_outside_window_fresh_final_health_can_be_slow(self):
        def slow_final(arm, records):
            final = self.final_record(records)
            final.update(completed_t=fx.stamp(29.5), t=fx.stamp(30))
            final['check'].update(latency_ms=4000, received_t=fx.stamp(29.4))
        result = self.case(slow_final)
        self.assertEqual(result['verdict'], 'PASS', result)
        self.assertTrue(result['claims']['lifecycle_success'])


class ReviewCleanup(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.helper = Path(self.tmp.name) / 'helper'
        self.helper.write_bytes(b'identity-only; never executed')
        prior = fx.HELPER_SHA
        fx.HELPER_SHA = lab.sha256_file(self.helper)
        self.addCleanup(setattr, fx, 'HELPER_SHA', prior)
        self.now = [0.0]
        self.count = 0

    def run_case(self, faults=(), diagnostic_s=0.6, diagnostics_budget=10, unresolved=False, removal_timeout=False):
        now = self.now
        bounds = []
        class Fake(FakeDocker):
            def run(engine, args, timeout):
                bounds.append((args, timeout, now[0]))
                if unresolved and args[0] == 'create':
                    engine.calls.append(args)
                    return lab.Result(None, '', '', True)
                if removal_timeout and args[0] == 'rm':
                    engine.calls.append(args)
                    now[0] += timeout
                    return lab.Result(None, '', '', True)
                result = super().run(args, timeout)
                if args[0] == 'exec' and args[2] in {'sh', 'journalctl'}:
                    if diagnostic_s > timeout:
                        now[0] += timeout
                        return lab.Result(None, '', '', True)
                    now[0] += diagnostic_s
                return result
        engine = Fake(faults=set(faults))
        arm = fx.stop_arm('idle', bounds={'settle_s': 0, 'cleanup_s': 1, 'diagnostics_s': diagnostics_budget})
        self.count += 1
        out = Path(self.tmp.name) / str(self.count)
        out.mkdir()
        (out / 'arm.json').write_text(json.dumps(arm))
        with mock.patch.object(lab.time, 'monotonic', side_effect=lambda: now[0]):
            run = lab.LabRun(arm, self.helper, out, engine)
            status = run.execute()
        self.assertTrue(all(0 < bound <= 60 for _, bound, _ in bounds))
        return status, run, engine

    def test_R4_confirmed_timeout_gets_fresh_finite_teardown_after_long_diagnostics(self):
        status, run, engine = self.run_case({'create-timeout'})
        self.assertEqual(status, 2, run.state)
        self.assertEqual(run.state['runner_status'], 'aborted')
        self.assertIn('create failed', run.state['refused'])
        self.assertTrue(run.cleanup_clean())
        self.assertEqual([c for c in engine.calls if c[0] == 'rm'], [['rm', '-f', run.owned]])
        self.assertFalse(engine.containers)
        self.assertEqual(run.phase_deadlines['reconciliation'], 1)
        self.assertAlmostEqual(run.phase_deadlines['cleanup'], 3.4)

    def test_R4_exhausted_diagnostics_still_allows_exact_id_removal(self):
        status, run, engine = self.run_case({'create-timeout'}, diagnostics_budget=0.5)
        self.assertEqual(status, 2, run.state)
        self.assertTrue(run.cleanup_clean())
        self.assertIn('diagnostics_error', run.state)
        self.assertEqual(len([c for c in engine.calls if c[0] == 'rm']), 1)

    def test_R4_unresolved_create_keeps_nonce_and_never_certifies_clean(self):
        status, run, engine = self.run_case(unresolved=True)
        self.assertEqual(status, 3, run.state)
        self.assertTrue(run.state['creation']['reconciliation_required'])
        self.assertEqual(run.state['creation']['nonce'], run.run_id)
        self.assertFalse(run.cleanup_clean())
        self.assertIsNone(run.state['cleanup']['audit']['inspect_absent'])
        self.assertFalse([c for c in engine.calls if c[0] == 'rm'])

    def test_R4_removal_failure_and_timeout_remain_unresolved(self):
        for timeout in (False, True):
            status, run, engine = self.run_case({'create-timeout'} | (set() if timeout else {'rm-fails'}), removal_timeout=timeout)
            self.assertEqual(status, 3, run.state)
            self.assertFalse(run.cleanup_clean())
            self.assertFalse(run.state['cleanup']['removed'])
            self.assertIn(run.owned, engine.containers)
            self.assertEqual(len([c for c in engine.calls if c[0] == 'rm']), 1)

    def test_R4_success_and_refusal_noop_controls(self):
        status, run, engine = self.run_case(diagnostic_s=0)
        self.assertEqual(status, 0, run.state)
        self.assertTrue(run.cleanup_clean())
        status, run, engine = self.run_case({'image-mismatch'}, diagnostic_s=0)
        self.assertEqual(status, 2, run.state)
        self.assertTrue(run.cleanup_clean())
        self.assertFalse([c for c in engine.calls if c[0] in {'create', 'rm'}])

    def test_R4_reconciliation_deadline_is_never_reset(self):
        status, run, engine = self.run_case(unresolved=True)
        self.assertEqual(status, 3)
        original = run.phase_deadlines['reconciliation']
        self.now[0] = original + 1
        calls = len(engine.calls)
        with mock.patch.object(lab.time, 'monotonic', side_effect=lambda: self.now[0]):
            with self.assertRaisesRegex(lab.Refused, 'reconciliation budget exhausted'):
                run.reconcile_create()
        self.assertEqual(run.phase_deadlines['reconciliation'], original)
        self.assertEqual(len(engine.calls), calls)
        self.assertTrue(run.state['creation']['reconciliation_required'])


if __name__ == '__main__':
    unittest.main()
