from __future__ import annotations

import unittest

from scripts import kata_freeze_model as model

Scenario = model.Scenario


def ordered(trace: list[str], *needles: str) -> bool:
    """True when each needle prefixes some later trace label, in order."""
    position = 0
    for needle in needles:
        while position < len(trace) and not trace[position].startswith(needle):
            position += 1
        if position == len(trace):
            return False
        position += 1
    return True


class FreezeModelTests(unittest.TestCase):
    def test_rejected_prototype_late_freeze_counterexample(self) -> None:
        # Strict expected negative: the test fails if the explorer finds no late freeze.
        for stuck in (True, False):
            with self.subTest(stuck=stuck):
                sc = Scenario(stuck=stuck)
                result = model.explore('detached-timeout-thaw', sc)
                self.assertIn('late-op', result['violations'])
                self.assertIn('stranded-freeze', result['violations'])
                trace = result['violations']['late-op']
                self.assertTrue(ordered(trace, 'H: local timeout, ThawUnit on a new connection', 'systemd: process',
                                        'H: thaw returned, discard freeze, reply error', 'systemd: process'), trace)
                self.assertTrue(trace[-1].endswith(' freeze'), trace)
                end = model.replay('detached-timeout-thaw', sc, trace)
                self.assertEqual((end['terminal'], end['host'], end['kf']), ('aborted', 'error', 1))

    def test_named_schedules_reproduce_both_late_freeze_shapes(self) -> None:
        # Timeout before submit: the thaw completes before the helper even sends.
        before_submit = ['H: spawn_blocking freeze', 'deadline', 'H: local timeout, ThawUnit on a new connection',
                         'dbus: deliver m1 from c1', 'systemd: process m1 thaw',
                         'H: thaw returned, discard freeze, reply error', 'F: zbus connect', 'F: send FreezeUnit',
                         'dbus: deliver m2 from c2', 'systemd: process m2 freeze', 'held request answered',
                         'kernel: cgroup frozen', 'systemd: cgroup.events', 'quiescent']
        # Queued freeze overtaken: submitted first, delivered after the new connection's thaw.
        overtaken = ['H: spawn_blocking freeze', 'F: zbus connect', 'F: send FreezeUnit', 'deadline',
                     'H: local timeout, ThawUnit on a new connection', 'dbus: deliver m2 from c2',
                     'systemd: process m2 thaw', 'H: thaw returned, discard freeze, reply error',
                     'dbus: deliver m1 from c1', 'systemd: process m1 freeze', 'held request answered',
                     'kernel: cgroup frozen', 'systemd: cgroup.events', 'quiescent']
        for name, schedule in (('before-submit', before_submit), ('overtaken', overtaken)):
            with self.subTest(name=name):
                end = model.replay('detached-timeout-thaw', Scenario(), schedule)
                self.assertEqual((end['terminal'], end['kf'], end['kz'], end['unit']), ('aborted', 1, 1, 'FROZEN'))
                self.assertEqual(model.quiescent_violations('detached-timeout-thaw', end, Scenario()), ('stranded-freeze',))

    def test_rejected_prototype_freezes_a_successor(self) -> None:
        result = model.explore('detached-timeout-thaw', Scenario(reuse=True))
        trace = result['violations']['successor-touched']
        self.assertTrue(ordered(trace, 'H: thaw returned', 'admit successor', 'systemd: process'), trace)

    def test_handler_scoped_drop_after_freeze_strands_the_cgroup(self) -> None:
        # Kata 3.29.0 and the block_in_place demonstration share this ordering.
        result = model.explore('handler-scoped', Scenario(lock_waiter=True))
        trace = result['violations']['stranded-freeze']
        self.assertIn('deadline', trace)
        self.assertTrue(ordered(trace, 'H: send FreezeUnit', 'H: freeze call returned',
                                'H: dropped at sandbox-lock await', 'quiescent'), trace)
        self.assertEqual(model.replay('handler-scoped', Scenario(lock_waiter=True), trace)['kf'], 1)
        self.assertEqual(model.explore('handler-scoped', Scenario())['violations'], {})

    def test_enumeration_failure_and_thaw_error(self) -> None:
        failed = model.explore('handler-scoped', Scenario(getpids_fail=True))['violations']['stranded-freeze']
        self.assertTrue(ordered(failed, 'H: get_pids failed, return before thaw'), failed)
        self.assertIn('dishonest-success', model.explore('handler-scoped', Scenario(thaw_error=True))['violations'])
        owner = model.explore('owner-reconciled', Scenario(getpids_fail=True))
        self.assertEqual(owner['violations'], {})
        self.assertIn('terminal=aborted host=error kf=0 owner=done successor=False', owner['outcomes'])
        unresolved = model.explore('owner-reconciled', Scenario(thaw_error=True, reuse=True))
        self.assertEqual(unresolved['violations'], {})
        self.assertTrue(any('owner=unresolved successor=False' in o and 'terminal=None' in o for o in unresolved['outcomes']))
        self.assertFalse(any('owner=unresolved' in o and 'successor=True' in o for o in unresolved['outcomes']))

    def test_same_connection_thaw_depends_on_fifo(self) -> None:
        self.assertEqual(model.explore('owner-ordered-thaw', Scenario())['violations'], {})
        unordered = model.explore('owner-ordered-thaw', Scenario(fifo=False))
        self.assertIn('late-op', unordered['violations'])
        self.assertIn('false-terminal', model.explore('owner-ordered-thaw', Scenario(thaw_error=True))['violations'])

    def test_reconciled_owner_has_no_violation_in_any_scenario(self) -> None:
        explored = 0
        for sc in model.matrix():
            result = model.explore('owner-reconciled', sc)
            explored += result['states']
            self.assertEqual(result['violations'], {}, sc)
        self.assertGreater(explored, 1000)

    def test_reconciled_owner_never_submits_after_its_deadline(self) -> None:
        result = model.explore('owner-reconciled', Scenario())
        self.assertIn('terminal=aborted-before-submit host=error kf=0 owner=done successor=False', result['outcomes'])

    def test_stalled_systemd_leaves_the_owner_unresolved_and_fenced(self) -> None:
        result = model.explore('owner-reconciled', Scenario(systemd_alive=False, reuse=True))
        self.assertEqual(result['violations'], {})
        for outcome in result['outcomes']:
            # Either nothing was ever submitted, or the owner is retained and the successor fenced.
            self.assertTrue('aborted-before-submit' in outcome or
                            ('terminal=None' in outcome and 'successor=False' in outcome), outcome)
        self.assertTrue(any('terminal=None' in o for o in result['outcomes']))

    def test_exploration_is_deterministic(self) -> None:
        sc = Scenario(lock_waiter=True, reuse=True, fifo=False)
        self.assertEqual(model.explore('detached-timeout-thaw', sc), model.explore('detached-timeout-thaw', sc))


if __name__ == '__main__':
    unittest.main()
