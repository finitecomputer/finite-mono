"""Regression cases for the four PR1092 review findings; synthetic state only."""
from __future__ import annotations

import contextlib
import io
import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock

from scripts import kata_lifecycle_lab as lab
from scripts import kata_lifecycle_verdict as verdict
from scripts.tests import kata_lifecycle_fixtures as fx

RUN, CID = '1' * 32, '2' * 64


class OwnedCliTests(unittest.TestCase):
    def interrupted_cli(self, method: str, signum: int) -> None:
        with tempfile.TemporaryDirectory() as directory:
            ready = Path(directory) / 'ready'
            processes: list[subprocess.Popen] = []
            real_popen = lab._OwnedPopen
            stop = threading.Event()
            sent = threading.Event()

            def capture(*args, **kwargs):
                proc = real_popen(*args, **kwargs)
                processes.append(proc)
                return proc

            def send_signal() -> None:
                deadline = time.monotonic() + 4
                while not stop.wait(0.005) and time.monotonic() < deadline:
                    if ready.exists():
                        sent.set()
                        os.kill(os.getpid(), signum)
                        return

            previous = signal.signal(signum, lab.handle_interrupt)
            sender = threading.Thread(target=send_signal, daemon=True)
            sender.start()
            script = 'import pathlib,sys,time; pathlib.Path(sys.argv[1]).write_text("ready"); print("ready",flush=True); time.sleep(30)'
            args = ['-u', '-c', script, str(ready)]
            started = time.monotonic()
            try:
                with mock.patch.object(lab, '_OwnedPopen', side_effect=capture):
                    engine = lab.Engine(sys.executable)
                    with self.assertRaises(lab.Interrupted):
                        if method == 'run':
                            engine.run(args, 5)
                        else:
                            engine.stream(args, 5, lambda line: None)
                self.assertTrue(sent.is_set(), 'The real OS signal must interrupt the owned CLI wait')
                self.assertLess(time.monotonic() - started, 4, 'Exceptional cleanup must be bounded')
                self.assertEqual(len(processes), 1)
                proc = processes[0]
                self.assertIsNotNone(proc.returncode, 'Owned CLI must be reaped before interruption escapes')
                self.assertTrue(proc.stdout.closed)
                self.assertTrue(proc.stderr.closed)
            finally:
                stop.set()
                sender.join(1)
                signal.signal(signum, previous)
                # Red regressions must not leave their real local sleeper behind.
                for proc in processes:
                    if proc.poll() is None:
                        proc.kill()
                    proc.wait(timeout=2)
                    proc.stdout.close()
                    proc.stderr.close()

    def test_run_reaps_cli_after_real_sigint(self) -> None:
        self.interrupted_cli('run', signal.SIGINT)

    def test_run_reaps_cli_after_real_sigterm(self) -> None:
        self.interrupted_cli('run', signal.SIGTERM)

    def test_stream_reaps_cli_after_real_sigint(self) -> None:
        self.interrupted_cli('stream', signal.SIGINT)

    def test_stream_reaps_cli_after_real_sigterm(self) -> None:
        self.interrupted_cli('stream', signal.SIGTERM)

    def test_stream_reaps_cli_when_reader_setup_is_interrupted(self) -> None:
        for signum in (signal.SIGINT, signal.SIGTERM):
            for stage in ('event', 'reader', 'drain'):
                with self.subTest(signal=signum, stage=stage):
                    processes = []
                    popen = lab._OwnedPopen
                    real_event, real_thread = threading.Event, threading.Thread
                    thread_count = 0

                    def capture(*args, **kwargs):
                        proc = popen(*args, **kwargs)
                        processes.append(proc)
                        return proc

                    def event(*args, **kwargs):
                        if stage == 'event':
                            os.kill(os.getpid(), signum)
                        return real_event(*args, **kwargs)

                    def thread(*args, **kwargs):
                        nonlocal thread_count
                        thread_count += 1
                        if thread_count == (1 if stage == 'reader' else 2) and stage != 'event':
                            os.kill(os.getpid(), signum)
                        return real_thread(*args, **kwargs)

                    previous = signal.signal(signum, lab.handle_interrupt)
                    started = time.monotonic()
                    try:
                        with mock.patch.object(lab, '_OwnedPopen', side_effect=capture), \
                                mock.patch.object(lab.threading, 'Event', side_effect=event), \
                                mock.patch.object(lab.threading, 'Thread', side_effect=thread):
                            with self.assertRaises(lab.Interrupted):
                                lab.Engine(sys.executable).stream(['-c', 'import time; time.sleep(30)'], 5, lambda line: None)
                        self.assertLess(time.monotonic() - started, 4, 'Setup cleanup must be bounded')
                        self.assertEqual(len(processes), 1)
                        self.assertIsNotNone(processes[0].returncode, 'Setup interruption must reap the owned CLI')
                        self.assertTrue(processes[0].stdout.closed)
                        self.assertTrue(processes[0].stderr.closed)
                    finally:
                        signal.signal(signum, previous)
                        for proc in processes:
                            if proc.poll() is None:
                                proc.kill()
                            proc.wait(timeout=2)
                            proc.stdout.close()
                            proc.stderr.close()

    def test_constructor_interruption_reaps_the_spawned_cli(self) -> None:
        for method in ('run', 'stream'):
            for signum in (signal.SIGINT, signal.SIGTERM):
                with self.subTest(method=method, signal=signum):
                    processes = []
                    execute_child = subprocess.Popen._execute_child

                    def interrupt_after_spawn(proc, *args, **kwargs):
                        execute_child(proc, *args, **kwargs)
                        processes.append(proc)
                        os.kill(os.getpid(), signum)

                    previous = signal.signal(signum, lab.handle_interrupt)
                    started = time.monotonic()
                    try:
                        with mock.patch.object(subprocess.Popen, '_execute_child', new=interrupt_after_spawn):
                            with self.assertRaises(lab.Interrupted):
                                engine = lab.Engine(sys.executable)
                                args = ['-c', 'import time; time.sleep(30)']
                                if method == 'run':
                                    engine.run(args, 5)
                                else:
                                    engine.stream(args, 5, lambda line: None)
                        self.assertLess(time.monotonic() - started, 4)
                        self.assertEqual(len(processes), 1)
                        self.assertIsNotNone(processes[0].returncode, 'Constructor interruption must reap the spawned CLI')
                        self.assertTrue(processes[0].stdout.closed)
                        self.assertTrue(processes[0].stderr.closed)
                    finally:
                        signal.signal(signum, previous)
                        for proc in processes:
                            if proc.poll() is None:
                                proc.kill()
                            proc.wait(timeout=2)
                            proc.stdout.close()
                            proc.stderr.close()

    def test_repeated_interruption_cannot_preempt_cli_cleanup(self) -> None:
        for method in ('run', 'stream'):
            with self.subTest(method=method):
                processes = []
                popen, cleanup = lab._OwnedPopen, lab._cleanup_cli
                sent = []

                def capture(*args, **kwargs):
                    proc = popen(*args, **kwargs)
                    processes.append(proc)
                    target = 'communicate' if method == 'run' else 'wait'
                    original = getattr(proc, target)

                    def interrupt_wait(*a, **kw):
                        if not sent:
                            sent.append(signal.SIGTERM)
                            os.kill(os.getpid(), signal.SIGTERM)
                        return original(*a, **kw)

                    setattr(proc, target, interrupt_wait)
                    return proc

                def interrupt_cleanup(*args, **kwargs):
                    sent.append(signal.SIGINT)
                    os.kill(os.getpid(), signal.SIGINT)
                    return cleanup(*args, **kwargs)

                previous = {s: signal.signal(s, lab.handle_interrupt) for s in (signal.SIGINT, signal.SIGTERM)}
                started = time.monotonic()
                try:
                    with mock.patch.object(lab, '_OwnedPopen', side_effect=capture), \
                            mock.patch.object(lab, '_cleanup_cli', side_effect=interrupt_cleanup):
                        with self.assertRaises(lab.Interrupted):
                            engine = lab.Engine(sys.executable)
                            args = ['-c', 'import time; time.sleep(30)']
                            if method == 'run':
                                engine.run(args, 5)
                            else:
                                engine.stream(args, 5, lambda line: None)
                    self.assertEqual(sent, [signal.SIGTERM, signal.SIGINT])
                    self.assertLess(time.monotonic() - started, 4)
                    self.assertIsNotNone(processes[0].returncode, 'Repeated interruption must not skip cleanup')
                    self.assertTrue(processes[0].stdout.closed)
                    self.assertTrue(processes[0].stderr.closed)
                    self.assertEqual(lab._active_cli_interrupts, [])
                    for s in previous:
                        self.assertIs(signal.getsignal(s), lab.handle_interrupt)
                finally:
                    for s, handler in previous.items():
                        signal.signal(s, handler)
                    for proc in processes:
                        if proc.poll() is None:
                            proc.kill()
                        proc.wait(timeout=2)
                        proc.stdout.close()
                        proc.stderr.close()

    def test_constructor_exception_reaps_the_spawned_cli(self) -> None:
        for method in ('run', 'stream'):
            with self.subTest(method=method):
                processes = []
                execute_child = subprocess.Popen._execute_child

                def fail_after_spawn(proc, *args, **kwargs):
                    execute_child(proc, *args, **kwargs)
                    processes.append(proc)
                    raise ValueError('synthetic constructor failure after spawn')

                try:
                    with mock.patch.object(subprocess.Popen, '_execute_child', new=fail_after_spawn):
                        with self.assertRaisesRegex(ValueError, 'synthetic constructor'):
                            engine = lab.Engine(sys.executable)
                            args = ['-c', 'import time; time.sleep(30)']
                            if method == 'run':
                                engine.run(args, 5)
                            else:
                                engine.stream(args, 5, lambda line: None)
                    self.assertEqual(len(processes), 1)
                    self.assertIsNotNone(processes[0].returncode, 'Failed construction must reap the spawned CLI')
                    self.assertTrue(processes[0].stdout.closed)
                    self.assertTrue(processes[0].stderr.closed)
                    self.assertEqual(lab._active_cli_interrupts, [])
                finally:
                    for proc in processes:
                        if proc.poll() is None:
                            proc.kill()
                        proc.wait(timeout=2)
                        proc.stdout.close()
                        proc.stderr.close()

    def test_timeout_reaps_real_cli_and_closes_pipes(self) -> None:
        for method in ('run', 'stream'):
            with self.subTest(method=method):
                processes = []
                popen = lab._OwnedPopen

                def capture(*args, **kwargs):
                    proc = popen(*args, **kwargs)
                    processes.append(proc)
                    return proc

                with mock.patch.object(lab, '_OwnedPopen', side_effect=capture):
                    engine = lab.Engine(sys.executable)
                    args = ['-c', 'import time; time.sleep(30)']
                    result = engine.run(args, 0.05) if method == 'run' else engine.stream(args, 0.05, lambda line: None)
                self.assertTrue(result.timed_out)
                self.assertIsNotNone(processes[0].returncode)
                self.assertTrue(processes[0].stdout.closed)
                self.assertTrue(processes[0].stderr.closed)

    def test_non_signal_exception_also_reaps_real_cli(self) -> None:
        for method in ('run', 'stream'):
            with self.subTest(method=method):
                processes = []
                popen = lab._OwnedPopen

                def capture(*args, **kwargs):
                    proc = popen(*args, **kwargs)
                    processes.append(proc)
                    target = 'communicate' if method == 'run' else 'wait'
                    original = getattr(proc, target)
                    calls = 0

                    def fail_once(*a, **kw):
                        nonlocal calls
                        calls += 1
                        if calls == 1:
                            raise ValueError('synthetic owned CLI observation failure')
                        return original(*a, **kw)

                    setattr(proc, target, fail_once)
                    return proc

                try:
                    with mock.patch.object(lab, '_OwnedPopen', side_effect=capture):
                        engine = lab.Engine(sys.executable)
                        args = ['-c', 'import time; time.sleep(30)']
                        with self.assertRaisesRegex(ValueError, 'synthetic owned CLI'):
                            if method == 'run':
                                engine.run(args, 5)
                            else:
                                engine.stream(args, 5, lambda line: None)
                    self.assertIsNotNone(processes[0].returncode)
                    self.assertTrue(processes[0].stdout.closed)
                    self.assertTrue(processes[0].stderr.closed)
                finally:
                    for proc in processes:
                        if proc.poll() is None:
                            proc.kill()
                        proc.wait(timeout=2)
                        proc.stdout.close()
                        proc.stderr.close()

    def test_failed_stream_callback_is_not_successful_evidence(self) -> None:
        def failed(_line):
            raise ValueError('synthetic callback failure')

        result = lab.Engine(sys.executable).stream(['-u', '-c', 'print("record",flush=True)'], 5, failed)
        self.assertTrue(result.timed_out)
        self.assertIn('local CLI reader failed (ValueError)', result.stderr)


class EvidenceReviewTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.count = 0

    def fixture(self, scenario: str = 'idle') -> Path:
        self.count += 1
        arm = fx.stop_arm(scenario)
        return fx.write_run(Path(self.tmp.name) / str(self.count), arm, fx.run_state(RUN, arm, CID),
                            fx.stop_records(RUN, CID, scenario).lines())

    def test_duplicate_keys_in_all_input_files_are_inconclusive(self) -> None:
        cases = [
            ('arm.json', '"purpose": "positive-control"', '"purpose":"expected-negative","purpose":"positive-control"'),
            ('arm.json', '"agent_log_level": "default"', '"agent_log_level":"debug","agent_log_level":"default"'),
            ('run.json', '"runner_status": "complete"', '"runner_status":"interrupted","runner_status":"complete"'),
            ('run.json', '"outcome": "confirmed"', '"outcome":"uncertain","outcome":"confirmed"'),
            ('run.json', '"runner_status": "complete"', '"runner_status":"complete","runner_status":"complete"'),
            ('run.json', '"runner_status": "complete"', '"runner_\\u0073tatus":"interrupted","runner_status":"complete"'),
            ('records.jsonl', '"outcome": "ok"', '"outcome":"transport-error","outcome":"ok"'),
            ('records.jsonl', '"freeze": "0"', '"freeze":"1","freeze":"0"'),
            ('records.jsonl', '"seq": 1', '"seq":0,"seq":1'),
        ]
        for name, original, duplicate in cases:
            with self.subTest(file=name, key=original):
                directory = self.fixture()
                path = directory / name
                text = path.read_text()
                self.assertIn(original, text)
                path.write_text(text.replace(original, duplicate, 1))
                result = verdict.classify(directory)
                self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
                self.assertIn('duplicate JSON key', result['reason'])
                self.assertFalse(result.get('claims', {}).get('lifecycle_success', False))

    def test_duplicate_arm_is_refused_before_any_engine_call(self) -> None:
        directory = self.fixture()
        arm_path = directory / 'arm.json'
        text = arm_path.read_text()
        arm_path.write_text(text.replace('"purpose": "positive-control"', '"purpose":"expected-negative","purpose":"positive-control"', 1))
        engine = mock.Mock()
        out = directory / 'new-output'
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            code = lab.main(['lab', str(arm_path), str(directory / 'unused-helper'), str(out)], engine=engine)
        self.assertEqual(code, 2)
        self.assertFalse(engine.mock_calls)
        self.assertFalse(out.exists())

    def test_either_accepts_only_both_supported_negative_observations(self) -> None:
        for scenario, observed in [('held-starved', 'starved'), ('held-liveness-only', 'responsive')]:
            with self.subTest(scenario=scenario):
                directory = self.fixture(scenario)
                arm_path = directory / 'arm.json'
                arm = json.loads(arm_path.read_text())
                arm['expect']['independent_connection'] = 'either'
                arm_path.write_text(json.dumps(arm))
                result = verdict.classify(directory)
                self.assertEqual(result['verdict'], 'PASS', result)
                self.assertEqual(result['observed']['independent_connection'], observed)
                self.assertFalse(result['claims']['lifecycle_success'])

    def test_either_cannot_accept_missing_connection_evidence(self) -> None:
        directory = self.fixture('held-starved')
        arm = fx.stop_arm('held-starved')
        arm['expect']['independent_connection'] = 'either'
        records = fx.stop_records(RUN, CID, 'held-starved')
        records.drop(lambda r: r.get('conn') == 'B' and r['kind'] in {'check', 'check-started'})
        fx.write_run(directory, arm, fx.run_state(RUN, arm, CID), records.lines())
        result = verdict.classify(directory)
        self.assertEqual(result['verdict'], 'INCONCLUSIVE', result)
        self.assertFalse(result.get('claims', {}).get('lifecycle_success', False))

    def test_either_remains_invalid_for_positive_qualification(self) -> None:
        arm = fx.stop_arm('idle')
        arm['expect']['independent_connection'] = 'either'
        with self.assertRaises(verdict.Inconclusive):
            verdict.validate_arm(arm)

    def test_omitted_release_default_is_used_in_actual_stream_callback(self) -> None:
        arm = fx.stop_arm('held-starved')
        arm.pop('release_after_s')
        verdict.validate_arm(arm)

        class LocalStream(lab.Engine):
            def stream(self, args, timeout, on_line):
                code = 'import json; print(json.dumps({"kind":"signal-sent"}),flush=True); print(json.dumps({"kind":"client-end"}),flush=True)'
                return super().stream(['-u', '-c', code], timeout, on_line)

        runner = lab.LabRun(arm, Path(self.tmp.name) / 'helper', Path(self.tmp.name), LocalStream(sys.executable))
        runner.owned = 'd' * 64
        runner.release = mock.Mock()
        with mock.patch.object(lab.threading, 'Timer') as timer:
            timer.return_value.is_alive.return_value = False
            runner.run_inner()
            self.assertEqual(timer.call_args.args[0], 12)
            timer.return_value.start.assert_called_once()
            timer.return_value.cancel.assert_called_once()
            timer.return_value.join.assert_called_once()
        runner.release.assert_called_once()
        self.assertEqual([json.loads(line)['kind'] for line in runner.records], ['signal-sent', 'client-end'])


if __name__ == '__main__':
    unittest.main()
