"""Owned, bounded runner for one synthetic Kata guest-agent lifecycle arm.

Usage: kata_lifecycle_lab.py <arm.json> <helper-binary> <new-output-dir>

The arm names an immutable lab image ID (a Kata guest rootfs running its own
systemd and kata-agent on a unix socket), the expected helper and agent
SHA-256, and named expected outcomes (see kata_lifecycle_verdict.py). The
helper is built from scripts/kata-lifecycle-lab.

Ownership: the output directory and the container name must not exist. The
container carries a fresh run-generation label. Cleanup is registered only
after `docker create` returns an ID whose name and labels are confirmed; it
removes that exact ID after re-checking its labels, then audits that neither
the ID nor any container with the run label remains. Nothing is ever removed
by name. A create with an uncertain remote result retains its persisted nonce
and reconciliation obligation. An exact labelled ID can resolve ownership;
empty point-in-time inventories cannot resolve an invisible pending create.

Bounds: no new experiment phase is admitted after the whole-arm budget.
Diagnostics, release, failed-create reconciliation and final teardown have
separate finite budgets. Reconciliation and teardown each receive one cleanup
allocation; neither absolute deadline is reset on a subsequent call. Local CLI
termination/drain also has finite observation bounds; it never proves that
accepted engine or agent work was rolled back. Cleanup audits exact owned IDs.

Exit: 0 PASS, 1 FAIL, 2 INCONCLUSIVE (including refused or failed setup),
3 owned resources not cleanly removed. Diagnostics stay in the output dir.
Synthetic state only; this never targets a Runner host or an Agent Runtime.
"""

from __future__ import annotations

import hashlib
import json
import re
import secrets
import signal
import subprocess
import sys
import threading
import time
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

try:
    from . import kata_lifecycle_verdict as verdict_module
except ImportError:
    import kata_lifecycle_verdict as verdict_module

LABEL_RUN = 'computer.finite.lab.run'
LABEL_OWNER = 'computer.finite.lab.owner'
OWNER = 'kata-lifecycle-lab'
HELPER = '/usr/local/bin/finlab'
AGENT_SOCKET = '/run/fin175/agent.sock'
IMAGE_ID = re.compile(r'sha256:[0-9a-f]{64}')
CONTAINER_ID = re.compile(r'[0-9a-f]{64}')
BOOT = 'mount -o remount,rw /sys/fs/cgroup && exec /lib/systemd/systemd --unit=kata-containers.target'
DEFAULT_BOUNDS = verdict_module.DEFAULT_BOUNDS


class Refused(Exception):
    """A precondition failed before any resource was created."""


class Interrupted(BaseException):
    """SIGTERM or SIGINT; cleanup still runs."""


@dataclass
class _CliInterrupts:
    signum: int | None = None
    ignore: bool = False

    def check(self) -> None:
        if self.signum is not None:
            raise Interrupted(self.signum)


_active_cli_interrupts: list[_CliInterrupts] = []


def handle_interrupt(signum: int, _frame: Any) -> None:
    """Defer configured signals while an owned CLI is being cleaned up."""
    if _active_cli_interrupts:
        if not any(frame.ignore for frame in _active_cli_interrupts):
            _active_cli_interrupts[-1].signum = signum
    else:
        raise Interrupted(signum)


@contextmanager
def _cli_interrupts():
    """Protect ownership transitions; final teardown keeps its ignore policy."""
    interrupts = _CliInterrupts()
    main = threading.current_thread() is threading.main_thread()
    if main:
        _active_cli_interrupts.append(interrupts)
    try:
        yield interrupts
    finally:
        if main:
            _active_cli_interrupts.pop()
            if interrupts.signum is not None and _active_cli_interrupts:
                _active_cli_interrupts[-1].signum = interrupts.signum
        interrupts.check()


def _check_interrupts() -> None:
    if not any(frame.ignore for frame in _active_cli_interrupts):
        for frame in _active_cli_interrupts:
            frame.check()


def _poll_cli(operation: Callable[..., Any], deadline: float, interrupts: _CliInterrupts) -> Any:
    while True:
        interrupts.check()
        try:
            return operation(timeout=min(0.1, max(0, deadline - time.monotonic())))
        except subprocess.TimeoutExpired:
            if time.monotonic() >= deadline:
                raise


class _OwnedPopen(subprocess.Popen):
    """Retain the local child handle even when its constructor raises."""

    def __init__(self, *args, **kwargs) -> None:
        try:
            super().__init__(*args, **kwargs)
        except BaseException:
            if getattr(self, 'pid', None) is not None:
                _cleanup_cli(self)
            raise


@dataclass
class Result:
    code: int | None
    stdout: str
    stderr: str
    timed_out: bool = False


class Engine:
    """Owned Docker CLI; bounded local cleanup does not cancel remote work."""

    def __init__(self, binary: str = 'docker') -> None:
        self.binary = binary

    def run(self, args: list[str], timeout: float) -> Result:
        if timeout <= 0:
            raise Refused('expired CLI submission budget')
        with _cli_interrupts() as interrupts:
            proc = None
            try:
                proc = _OwnedPopen([self.binary, *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                try:
                    stdout, stderr = _poll_cli(proc.communicate, time.monotonic() + timeout, interrupts)
                    result = Result(proc.returncode, stdout, stderr)
                except subprocess.TimeoutExpired as expired:
                    proc.kill()
                    try:
                        stdout, stderr = proc.communicate(timeout=1)
                    except subprocess.TimeoutExpired:
                        stdout, stderr = _text(expired.stdout), _text(expired.stderr) + '\nlocal CLI drain unresolved'
                    result = Result(proc.poll(), stdout, stderr, True)
            finally:
                cleaned = proc is None or _cleanup_cli(proc)
            if not cleaned:
                result.timed_out = True
                result.stderr += '\nlocal CLI cleanup unresolved'
            return result

    def stream(self, args: list[str], timeout: float, on_line: Callable[[str], None]) -> Result:
        if timeout <= 0:
            raise Refused('expired CLI submission budget')
        deadline = time.monotonic() + timeout
        with _cli_interrupts() as interrupts:
            proc = None
            readers: list[threading.Thread] = []
            errors: list[str] = []
            try:
                proc = _OwnedPopen([self.binary, *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                interrupts.check()
                reader_failed = threading.Event()

                def read_lines() -> None:
                    try:
                        for line in proc.stdout:
                            on_line(line)
                    except Exception as error:  # noqa: BLE001 - callback failure is not successful evidence
                        reader_failed.set()
                        errors.append(f'local CLI reader failed ({type(error).__name__})')

                reader = threading.Thread(target=read_lines, daemon=True)
                readers.append(reader)
                drain = threading.Thread(target=lambda: errors.extend(proc.stderr), daemon=True)
                readers.append(drain)
                reader.start()
                drain.start()
                try:
                    code = _poll_cli(proc.wait, deadline, interrupts)
                    for thread in readers:
                        while thread.is_alive() and time.monotonic() < deadline:
                            interrupts.check()
                            thread.join(min(0.1, max(0, deadline - time.monotonic())))
                    result = Result(code, '', '', reader.is_alive() or drain.is_alive() or reader_failed.is_set())
                except subprocess.TimeoutExpired:
                    result = Result(None, '', '', True)
            finally:
                cleaned = proc is None or _cleanup_cli(proc, tuple(readers))
            if not cleaned:
                result.timed_out = True
                errors.append('local CLI cleanup unresolved')
            result.stderr = ''.join(errors)
            return result


def _cleanup_cli(proc: subprocess.Popen, readers: tuple[threading.Thread, ...] = ()) -> bool:
    """Reap only this local CLI and close pipes within one two-second budget."""
    deadline = time.monotonic() + 2
    close_errors: list[OSError] = []

    def close_pipes() -> None:
        for pipe in (proc.stdout, proc.stderr):
            if pipe is not None:
                try:
                    pipe.close()
                except OSError as error:
                    close_errors.append(error)

    if proc.poll() is None:
        proc.kill()
    try:
        proc.wait(timeout=max(0, deadline - time.monotonic()))
    except subprocess.TimeoutExpired:
        pass
    for reader in readers:
        if reader.ident is not None:
            reader.join(max(0, deadline - time.monotonic()))
    # An inherited pipe can outlive the CLI. Closing a TextIO wrapper while
    # a reader holds its lock can block; retain a bounded unresolved result.
    if any(reader.is_alive() for reader in readers):
        closer = threading.Thread(target=close_pipes, daemon=True)
        closer.start()
        closer.join(max(0, deadline - time.monotonic()))
        closed = not closer.is_alive()
    else:
        close_pipes()
        closed = True
    return proc.returncode is not None and closed and not close_errors and not any(r.is_alive() for r in readers)


def _text(value: Any) -> str:
    if isinstance(value, bytes):
        return value.decode(errors='replace')
    return value or ''


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1 << 20), b''):
            digest.update(block)
    return digest.hexdigest()


def missing_container(result: Result) -> bool:
    return not result.timed_out and result.code not in (0, None) and 'no such' in result.stderr.lower()


class LabRun:
    def __init__(self, arm: dict[str, Any], helper: Path, out: Path, engine: Engine,
                 sleep: Callable[[float], None] = time.sleep) -> None:
        self.arm, self.helper, self.out, self.engine, self.sleep = arm, helper, out, engine, sleep
        self.bounds = {**DEFAULT_BOUNDS, **arm.get('bounds', {})}
        verdict_module.validate_arm(arm)
        self.run_id = secrets.token_hex(16)
        self.name = 'finlab-' + self.run_id
        self.cid = secrets.token_hex(32)
        self.owned: str | None = None
        self.agent_pid: int | None = None
        self.records: list[str] = []
        self.lock = threading.Lock()
        self.steps: list[dict[str, Any]] = []
        self.deadline = time.monotonic() + self.bounds['total_s']
        self.phase_deadlines: dict[str, float] = {}
        self.state: dict[str, Any] = {'schema': verdict_module.RUN_SCHEMA, 'run': self.run_id, 'arm': arm.get('name'),
                                      'kind': arm.get('kind'), 'synthetic_cid': self.cid, 'steps': self.steps,
                                      'container': {'name': self.name}, 'runner_status': 'aborted',
                                      'creation': {'nonce': self.run_id, 'outcome': 'never-attempted',
                                                   'reconciliation_required': False}}

    # -- bookkeeping -------------------------------------------------------
    def remaining(self, bound: float, phase: str = 'experiment') -> float:
        if phase == 'experiment':
            _check_interrupts()
            deadline = self.deadline
        else:
            allocation = 'cleanup' if phase == 'reconciliation' else phase
            deadline = self.phase_deadlines.setdefault(phase, time.monotonic() + self.bounds[allocation + '_s'])
        left = deadline - time.monotonic()
        if left <= 0:
            self.state.setdefault('exhausted_budgets', []).append(phase)
            raise Refused(phase + ' budget exhausted')
        return min(bound, left)

    def pause(self, seconds: float, phase: str = 'experiment') -> None:
        if seconds == 0:
            self.remaining(1, phase)
            return
        left = self.remaining(seconds, phase)
        self.sleep(left)
        if phase == 'experiment':
            _check_interrupts()
        if left < seconds:
            raise Refused(phase + ' budget exhausted during settle/readiness')
        self.remaining(1, phase)

    def step(self, name: str, result: Result | None, ok: bool, detail: str = '') -> bool:
        entry: dict[str, Any] = {'name': name, 'status': 'ok' if ok else 'failed', 'detail': detail}
        if result is not None:
            entry['exit'] = result.code
            if result.timed_out:
                entry['status'] = 'timeout'
            if not ok and result.stderr:
                entry['stderr_tail'] = result.stderr[-400:]
        self.steps.append(entry)
        return entry['status'] == 'ok'

    def require(self, name: str, result: Result, ok: bool | None = None, detail: str = '') -> Result:
        good = (result.code == 0 and not result.timed_out) if ok is None else ok
        if not self.step(name, result, good, detail):
            raise Refused(f'{name} failed')
        return result

    def add_records(self, text: str) -> None:
        with self.lock:
            self.records.extend(line for line in text.splitlines() if line.strip())

    def exec_(self, args: list[str], bound: float | None = None, phase: str = 'experiment') -> Result:
        return self.engine.run(['exec', self.owned, *args], self.remaining(bound or self.bounds['exec_s'], phase))

    # -- ownership ---------------------------------------------------------
    def labels_match(self, container_id: str, phase: str = 'experiment') -> tuple[bool, Result]:
        if not CONTAINER_ID.fullmatch(container_id):
            return False, Result(2, '', 'invalid container ID')
        fmt = '{{.Id}}|{{.Name}}|{{index .Config.Labels "%s"}}|{{index .Config.Labels "%s"}}' % (LABEL_RUN, LABEL_OWNER)
        result = self.engine.run(['container', 'inspect', '--format', fmt, container_id], self.remaining(self.bounds['exec_s'], phase))
        want = f'{container_id}|/{self.name}|{self.run_id}|{OWNER}'
        return result.code == 0 and not result.timed_out and result.stdout.strip() == want, result

    def label_matches(self, phase: str = 'experiment') -> list[str] | None:
        result = self.engine.run(['ps', '-aq', '--no-trunc', '--filter', f'label={LABEL_RUN}={self.run_id}'], self.remaining(self.bounds['exec_s'], phase))
        if result.code != 0 or result.timed_out:
            return None
        found = result.stdout.split()
        return found if all(CONTAINER_ID.fullmatch(cid) for cid in found) else None

    def preflight(self) -> None:
        self.remaining(self.bounds['exec_s'])
        inputs = self.arm['inputs']
        image, platform = inputs.get('image_id', ''), inputs.get('platform', '')
        if not IMAGE_ID.fullmatch(image) or platform not in {'linux/arm64', 'linux/amd64'}:
            raise Refused('arm inputs must name an immutable sha256 image ID and platform')
        digest = sha256_file(self.helper)
        self.state['inputs'] = {'image_id': image, 'platform': platform, 'helper_sha256': digest}
        self.require('helper-identity', Result(0, '', ''), digest == inputs.get('helper_sha256'),
                     f'helper sha256 {digest}')
        found = self.engine.run(['image', 'inspect', '--format', '{{.Id}} {{.Os}}/{{.Architecture}}', image], self.remaining(self.bounds['exec_s']))
        self.require('image-identity', found, found.code == 0 and found.stdout.strip() == f'{image} {platform}')
        name = self.engine.run(['container', 'inspect', '--format', '{{.Id}}', self.name], self.remaining(self.bounds['exec_s']))
        self.require('name-free', name, missing_container(name), 'container name must not exist')
        self.steps[-1]['missing_confirmed'] = True
        self.require('label-free', Result(0, '', ''), self.label_matches() == [], 'run label must be unused')

    def create(self) -> None:
        timeout = self.remaining(self.bounds['create_s'])
        platform = self.arm['inputs']['platform']
        args = ['create', '--name', self.name, '--label', f'{LABEL_RUN}={self.run_id}', '--label', f'{LABEL_OWNER}={OWNER}',
                '--platform', platform, '--cap-add=ALL', '--security-opt', 'seccomp=unconfined',
                '--security-opt', 'apparmor=unconfined', '--security-opt', 'systempaths=unconfined',
                '--cgroupns=private', '--network', 'none', '--device', '/dev/fuse', '--entrypoint', '/bin/sh',
                self.arm['inputs']['image_id'], '-c', BOOT]
        # Persist before submission. Interruption or loss of the CLI result is
        # uncertain remote work; an empty inventory cannot settle this nonce.
        self.state['creation'].update(outcome='uncertain', reconciliation_required=True)
        self.write()
        created = self.engine.run(args, timeout)
        candidate = created.stdout.strip()
        if created.code == 0 and not created.timed_out and CONTAINER_ID.fullmatch(candidate):
            matched, _ = self.labels_match(candidate)
            if matched:
                self.owned = candidate
                self.state['container'].update(id=candidate, ownership_confirmed=True)
                self.state['creation'].update(outcome='confirmed', reconciliation_required=False)
                self.step('create', created, True)
                return
        # Create failed, timed out or returned an unconfirmed ID: adopt only a
        # container that carries this run's label, after confirming it exactly.
        if not created.timed_out and created.code not in (0, None) and 'Conflict.' in created.stderr:
            self.state['creation'].update(outcome='rejected', reconciliation_required=False)
        self.reconcile_create()
        self.require('create', created, False, 'create failed or ownership unconfirmed')

    def reconcile_create(self) -> None:
        matches = self.label_matches('reconciliation')
        self.state['creation']['last_label_observation'] = matches
        if matches is not None and len(matches) == 1:
            matched, _ = self.labels_match(matches[0], 'reconciliation')
            if matched:
                self.owned = matches[0]
                self.state['container'].update(id=self.owned, ownership_confirmed=True, adopted_after_failed_create=True)
                self.state['creation'].update(outcome='confirmed', reconciliation_required=False)

    def teardown(self) -> None:
        if self.state['creation']['reconciliation_required']:
            try:
                self.reconcile_create()
            except Refused as error:
                # An exhausted reconciliation allocation is still unresolved.
                # It cannot consume final exact-ID removal/audit admission.
                self.state['creation']['reconciliation_error'] = str(error)
        self.remaining(self.bounds['cleanup_s'], 'cleanup')
        cleanup: dict[str, Any] = {'attempted': self.owned is not None, 'guard': 'not-needed', 'removed': False}
        self.state['cleanup'] = cleanup
        if self.owned is not None:
            matched, inspect = self.labels_match(self.owned, 'cleanup')
            if matched:
                cleanup['guard'] = 'ok'
                removed = self.engine.run(['rm', '-f', self.owned], self.remaining(self.bounds['cleanup_s'], 'cleanup'))
                cleanup['rm_exit'] = removed.code
                cleanup['removed'] = removed.code == 0 and not removed.timed_out
            elif missing_container(inspect):
                cleanup['guard'] = 'ok'
                cleanup['removed'] = True
                cleanup['note'] = 'container already absent'
            else:
                cleanup['guard'] = 'ownership-mismatch'
        audit: dict[str, Any] = {'label_matches': self.label_matches('cleanup')}
        if self.owned is not None:
            audit['inspect_absent'] = missing_container(
                self.engine.run(['container', 'inspect', '--format', '{{.Id}}', self.owned], self.remaining(self.bounds['exec_s'], 'cleanup')))
        else:
            audit['inspect_absent'] = True
            cleanup['removed'] = audit['label_matches'] == []
            cleanup['guard'] = 'ok' if audit['label_matches'] == [] else 'unowned-label-match'
        cleanup['audit'] = audit
        if self.state['creation']['reconciliation_required']:
            cleanup.update(guard='create-unresolved', removed=False, reconciliation_required=True)
            audit['inspect_absent'] = None
            cleanup['note'] = 'empty point-in-time inventories do not fence a pending remote create'

    def cleanup_clean(self) -> bool:
        c = self.state.get('cleanup', {})
        return (c.get('guard') in {'ok', 'not-needed'} and c.get('removed') is True
                and not self.state['creation']['reconciliation_required']
                and c['audit'].get('inspect_absent') is True and c['audit'].get('label_matches') == [])

    # -- arm ----------------------------------------------------------------
    def wait_ready(self) -> None:
        deadline = time.monotonic() + self.bounds['ready_s']
        probe = Result(None, '', 'agent socket readiness budget exhausted', True)
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                self.require('ready-agent-socket', probe, False, 'agent socket never appeared')
            probe = self.exec_(['test', '-S', AGENT_SOCKET], min(10, left))
            if probe.code == 0 and not probe.timed_out and time.monotonic() <= deadline:
                break
            self.pause(min(0.5, max(0, deadline - time.monotonic())))
        self.step('ready-agent-socket', probe, True)
        system = self.exec_(['systemctl', 'is-system-running'], 15)
        state = system.stdout.strip()
        self.require('ready-systemd', system, state in {'running', 'degraded'} and not system.timed_out, f'state {state}')

    def helper_exec(self, name: str, args: list[str], bound: float | None = None) -> Result:
        result = self.exec_([HELPER, *args, '-run', self.run_id], bound)
        self.add_records(result.stdout)
        return self.require(name, result)

    def install(self) -> None:
        copied = self.engine.run(['cp', str(self.helper), f'{self.owned}:{HELPER}'], self.remaining(self.bounds['exec_s']))
        self.require('install-helper', copied)
        digest = self.exec_(['sha256sum', HELPER])
        self.require('helper-in-container', digest, digest.code == 0 and digest.stdout.split()[:1] == [self.state['inputs']['helper_sha256']])
        identity = self.helper_exec('identity', ['identity', '-sock', AGENT_SOCKET])
        found = [verdict_module.parse_json(line) for line in identity.stdout.splitlines() if line.strip()]
        try:
            verdict_module.need(len(found) == 1, 'identity record count')
            verdict_module.validate_identity(found[0], self.arm, self.state)
        except verdict_module.Inconclusive as error:
            self.require('agent-identity', identity, False, str(error))
        self.agent_pid = found[0].get('agent_pid')
        if self.arm['kind'] == 'agent-stop':
            self.require('agent-identity', identity, True, 'strict serving identity validated before prepare')
        self.helper_exec('prepare', ['prepare'])
        unit = 'finlab-fused-' + self.run_id[:12]
        hold = str(self.arm.get('hold_max_s', 90)) + 's'
        fused = self.exec_(['systemd-run', '--quiet', f'--unit={unit}', f'--property=RuntimeMaxSec={self.bounds["total_s"]}s',
                            HELPER, 'fused', '-run', self.run_id, '-hold-max', hold], 15)
        self.require('start-fused', fused)
        deadline = time.monotonic() + 15
        mounted = Result(None, '', 'FUSE readiness budget exhausted', True)
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                self.require('ready-fuse', mounted, False, 'FUSE mount never appeared')
            mounted = self.exec_(['grep', '-q', ' /run/finlab/mnt fuse', '/proc/mounts'], min(5, left))
            if mounted.code == 0 and not mounted.timed_out and time.monotonic() <= deadline:
                break
            self.pause(min(0.2, max(0, deadline - time.monotonic())))
        self.step('ready-fuse', mounted, True)

    def release(self) -> None:
        try:
            result = self.exec_([HELPER, 'release', '-run', self.run_id], self.bounds['release_s'], 'release')
        except Refused as error:
            self.step('release', Result(None, '', str(error), True), False)
            return
        self.add_records(result.stdout)
        self.step('release', result, result.code == 0 and not result.timed_out)

    def client_args(self) -> list[str]:
        arm = self.arm
        live = verdict_module.THRESHOLDS['liveness_span_s'] + 2
        return ['client', '-cid', self.cid, '-cgroups', arm['cgroups'], f"-reader={'true' if arm['reader'] else 'false'}",
                '-init-on-term', arm.get('init_on_term', 'exit'), '-signal', str(arm.get('signal', 15)),
                '-agent-pid', str(self.agent_pid or 0), '-max', f"{arm.get('client_max_s', 120)}s",
                '-window', f"{arm.get('window_s', 20)}s", '-min-observe', f'{live}s']

    def run_inner(self) -> None:
        timer: threading.Timer | None = None
        released = threading.Event()
        release_lock = threading.Lock()

        def release_once() -> None:
            with release_lock:
                if not released.is_set():
                    released.set()
                    self.release()

        def on_line(line: str) -> None:
            nonlocal timer
            self.add_records(line)
            try:
                record = verdict_module.parse_json(line)
            except (ValueError, verdict_module.Inconclusive):
                return
            if record.get('kind') == 'signal-sent' and self.arm.get('reader') and timer is None:
                timer = threading.Timer(float(self.arm.get('release_after_s', verdict_module.DEFAULT_RELEASE_AFTER_S)), release_once)
                timer.daemon = True
                timer.start()

        if self.arm['kind'] == 'agent-stop':
            args = [HELPER, *self.client_args(), '-run', self.run_id]
            bound = self.arm.get('client_max_s', 120) + self.bounds['client_margin_s']
        else:
            args = [HELPER, 'order', '-run', self.run_id]
            bound = self.bounds.get('order_s', 120)
        try:
            result = self.engine.stream(['exec', self.owned, *args], self.remaining(bound), on_line)
        finally:
            if timer is not None:
                timer.cancel()
            # Release and drain are separately bounded even after interruption
            # or whole-arm expiration. There is no retry policy here.
            if self.arm.get('reader') or self.arm['kind'] == 'systemd-freezer-order':
                release_once()
            if timer is not None:
                timer.join(self.remaining(self.bounds['release_s'], 'release'))
                if timer.is_alive():
                    self.step('release-join', Result(None, '', '', True), False, 'release helper still running')
        self.require('inner', result)

    def collect(self) -> None:
        """Collects records and diagnostics; runs after a failed arm too."""
        diagnostics = self.out / 'diagnostics'
        diagnostics.mkdir()
        for path in ('/run/finlab/log/fused.jsonl', '/run/finlab/log/ctr.jsonl', '/run/finlab/log/order-readers.jsonl'):
            got = self.exec_(['sh', '-c', f'[ ! -e {path} ] || cat {path}'], 15, 'diagnostics')
            self.step('collect ' + path.rsplit('/', 1)[1], got, got.code == 0 and not got.timed_out)
            self.add_records(got.stdout)
        journal = self.exec_(['journalctl', '-o', 'short-iso-precise', '--no-pager', '-u', 'kata-agent'], 20, 'diagnostics')
        self.step('collect journal', journal, journal.code == 0 and not journal.timed_out)
        (diagnostics / 'kata-agent-journal.txt').write_text(journal.stdout + journal.stderr)

    def final_observation(self) -> None:
        self.pause(self.bounds['settle_s'])
        seen: list[int] = []
        for line in self.records:
            try:
                record = verdict_module.parse_json(line)
            except (ValueError, verdict_module.Inconclusive):
                continue
            if record.get('kind') == 'client-end':
                seen = record.get('seen_pids', [])
        self.helper_exec('final-observation', ['observe', '-cid', self.cid, '-cgroups', self.arm['cgroups'],
                                               '-pids', ','.join(map(str, seen)), '-agent-pid', str(self.agent_pid or 0),
                                               '-label', 'final'])

    def execute(self) -> int:
        # Establish cancellation protection before any resource is submitted;
        # reuse this frame when finalization starts, without an acquisition gap.
        with _cli_interrupts() as interrupts:
            try:
                self.preflight()
                self.create()
                self.require('start', self.engine.run(['start', self.owned], self.remaining(self.bounds['create_s'])))
                self.wait_ready()
                self.install()
                failure: Refused | None = None
                try:
                    self.run_inner()
                    if self.arm['kind'] == 'agent-stop':
                        self.final_observation()
                except Refused as reason:
                    failure = reason
                self.collect()
                if failure is not None:
                    raise failure
                self.state['runner_status'] = 'complete'
            except Refused as reason:
                self.state['refused'] = str(reason)
            except Interrupted:
                self.state['runner_status'] = 'interrupted'
            except Exception as error:  # noqa: BLE001 - a runner defect is INCONCLUSIVE, never exit 1 (FAIL)
                self.state['internal_error'] = type(error).__name__
            finally:
                interrupts.ignore = True
                if interrupts.signum is not None:
                    self.state['runner_status'] = 'interrupted'
                    interrupts.signum = None
                try:
                    if self.owned is not None and not (self.out / 'diagnostics').exists():
                        try:
                            self.collect()
                        except Exception as error:  # noqa: BLE001 - diagnostics cannot suppress cleanup
                            self.state['diagnostics_error'] = str(error)
                    self.teardown()
                except Exception as error:  # noqa: BLE001 - preserve unresolved cleanup
                    self.state.setdefault('cleanup', {}).update(guard='cleanup-unresolved', removed=False,
                                                               error=str(error))
                finally:
                    self.write()
            if not self.cleanup_clean():
                return 3
            try:
                result = verdict_module.classify(self.out)
            except Exception as error:  # noqa: BLE001 - a classifier defect must never read as PASS or FAIL
                result = {'schema': verdict_module.VERDICT_SCHEMA, 'verdict': 'INCONCLUSIVE',
                          'reason': f'classifier error ({type(error).__name__})'}
            (self.out / 'verdict.json').write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
            return verdict_module.EXIT[result['verdict']]

    def write(self) -> None:
        (self.out / 'run.json').write_text(json.dumps(self.state, indent=2, sort_keys=True) + '\n')
        (self.out / 'records.jsonl').write_text(''.join(line.rstrip('\n') + '\n' for line in self.records))


def main(argv: list[str], engine: Engine | None = None) -> int:
    if len(argv) != 4:
        print(__doc__.splitlines()[2], file=sys.stderr)
        return 2
    arm_path, helper, out = Path(argv[1]), Path(argv[2]), Path(argv[3])
    try:
        arm = verdict_module.load_json(arm_path)
        verdict_module.validate_arm(arm)
    except (OSError, ValueError, verdict_module.Inconclusive) as error:
        print(f'refused: arm invalid: {error}', file=sys.stderr)
        return 2
    try:
        out.mkdir()
    except OSError as error:
        print(f'refused: output directory must be new: {error}', file=sys.stderr)
        return 2
    (out / 'arm.json').write_text(json.dumps(arm, indent=2, sort_keys=True) + '\n')

    previous = {s: signal.signal(s, handle_interrupt) for s in (signal.SIGTERM, signal.SIGINT)}
    try:
        code = LabRun(arm, helper, out, engine or Engine()).execute()
    finally:
        for s, handler in previous.items():
            signal.signal(s, handler)
    print(json.dumps({'out': str(out), 'exit': code}))
    return code


if __name__ == '__main__':
    sys.exit(main(sys.argv))
