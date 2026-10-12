"""Synthetic minimum records for the Kata lifecycle lab tests.

Nothing here is copied from a real run: values are the smallest records the
verdict needs, with offsets in seconds from a fixed origin.
"""

from __future__ import annotations

import copy
import json
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

HELPER_SHA = 'a' * 64
AGENT_SHA = 'b' * 64
IMAGE_ID = 'sha256:' + 'c' * 64
ORIGIN = datetime(2026, 1, 1, tzinfo=timezone.utc)
SCHEMA = 'finite.kata-lifecycle-record.v2'

STOP_EXPECT = {
    'idle': {'held_request': 'absent', 'signal': 'prompt-ok', 'same_connection': 'responsive',
             'independent_connection': 'responsive', 'init': 'exited-promptly',
             'stop_before_release': 'not-applicable', 'final_freezer': 'running', 'final_tasks': 'none'},
    'held-starved': {'held_request': 'accepted-before-signal', 'signal': 'ok-after-release',
                     'same_connection': 'starved', 'independent_connection': 'starved',
                     'init': 'exited-after-release', 'stop_before_release': 'incomplete',
                     'final_freezer': 'running', 'final_tasks': 'none'},
    'held-liveness-only': {'held_request': 'accepted-before-signal', 'signal': 'prompt-ok',
                           'same_connection': 'responsive', 'independent_connection': 'responsive',
                           'init': 'exited-after-release', 'stop_before_release': 'incomplete',
                           'final_freezer': 'running', 'final_tasks': 'none'},
    'stranded-frozen': {'held_request': 'accepted-before-signal', 'signal': 'deadline-exceeded',
                        'same_connection': 'responsive', 'independent_connection': 'responsive',
                        'init': 'not-exited', 'stop_before_release': 'incomplete',
                        'final_freezer': 'frozen', 'final_tasks': 'remaining'},
}
PURPOSE = {'idle': 'positive-control', 'held-starved': 'expected-negative',
           'held-liveness-only': 'expected-negative', 'stranded-frozen': 'expected-negative'}
ORDER_CASES = {'thaw-cancels-accepted-freeze': 'cancelled', 'thaw-before-freeze': 'late-freeze-applied',
               'local-timeout-not-rollback': 'freeze-still-pending'}


def stamp(offset: float) -> str:
    return (ORIGIN + timedelta(seconds=offset)).strftime('%Y-%m-%dT%H:%M:%S.%f') + '000Z'


class Records:
    def __init__(self, run: str) -> None:
        self.run = run
        self.seq: dict[str, int] = {}
        self.items: list[dict[str, Any]] = []
        self.check_id = 0

    def add(self, source: str, kind: str, offset: float, **fields: Any) -> dict[str, Any]:
        self.seq[source] = self.seq.get(source, 0) + 1
        record = {'schema': SCHEMA, 'run': self.run, 'source': source, 'seq': self.seq[source],
                  't': stamp(offset), 'kind': kind, **fields}
        self.items.append(record)
        return record

    def drop(self, predicate) -> None:
        """Removes records as if never emitted, renumbering each source."""
        self.items = [r for r in self.items if not predicate(r)]
        self.seq = {}
        for record in self.items:
            self.seq[record['source']] = record['seq'] = self.seq.get(record['source'], 0) + 1

    def lines(self, source: str | None = None) -> list[str]:
        return [json.dumps(r) for r in self.items if source is None or r['source'] == source]


def stop_arm(scenario: str, **overrides: Any) -> dict[str, Any]:
    reader = scenario != 'idle'
    arm = {'schema': 'finite.kata-lifecycle-arm.v1', 'name': 'fixture-' + scenario, 'kind': 'agent-stop',
           'purpose': PURPOSE[scenario],
           'inputs': {'image_id': IMAGE_ID, 'platform': 'linux/arm64', 'helper_sha256': HELPER_SHA},
           'identity': {'agent_sha256': AGENT_SHA, 'agent_log_level': 'default', 'systemd_version_prefix': 'systemd 255 '},
           'cgroups': 'systemd', 'reader': reader, 'signal': 15, 'init_on_term': 'exit',
           'release_after_s': 12, 'hold_max_s': 60, 'expect': copy.deepcopy(STOP_EXPECT[scenario])}
    arm.update(overrides)
    return arm


def order_arm() -> dict[str, Any]:
    return {'schema': 'finite.kata-lifecycle-arm.v1', 'name': 'fixture-order', 'kind': 'systemd-freezer-order',
            'purpose': 'expected-negative',
            'inputs': {'image_id': IMAGE_ID, 'platform': 'linux/arm64', 'helper_sha256': HELPER_SHA},
            'identity': {'systemd_version_prefix': 'systemd 255 '}, 'expect': {'cases': dict(ORDER_CASES)}}


def identity(rec: Records, agent_sha: str = AGENT_SHA, **extra: Any) -> None:
    fields = {'helper_sha256': HELPER_SHA, 'agent_pids': [75], 'agent_pid': 75, 'agent_exe_sha256': agent_sha,
              'agent_serves_socket': True, 'agent_log_level_env_set': False, 'agent_log_level_env': '',
              'kernel_agent_params': [], 'systemd_version': 'systemd 255 (255.4-1ubuntu8)', 'cgroup2': True,
              'helper_error': '', 'systemd_version_error': '', 'observation_error': '', 'agent_exe_error': '',
              'agent_exe_path': '/usr/bin/kata-agent', 'agent_socket': '/run/fin175/agent.sock',
              'agent_unit': 'synthetic ExecStart=/usr/bin/kata-agent', 'agent_unit_error': '',
              'kernel_release': 'synthetic-kernel', 'nproc': 4, 'agent_tokio_workers': 4}
    fields.update(extra)
    rec.add('identity', 'identity', 0.0, **fields)


def check(rec: Records, conn: str, sent: float, latency_ms: float, outcome: str = 'ok') -> None:
    rec.check_id += 1
    rec.add('client', 'check-started', sent, conn=conn, request_id=rec.check_id, sent_t=stamp(sent))
    rec.add('client', 'check', sent + latency_ms / 1000, conn=conn, sent_t=stamp(sent), latency_ms=latency_ms, outcome=outcome,
            method='Check', request_id=rec.check_id, stream=1, code=0, submission='submitted')
    rec.items[-1]['health_status'] = 1
    if outcome == 'ok':
        rec.items[-1]['received_t'] = rec.items[-1]['t']


def final(cgroups: str, freezer: str, procs: list[int]) -> dict[str, Any]:
    freeze, frozen = {'running': ('0', '0'), 'frozen': ('1', '1'), 'freezing': ('1', '0')}[freezer]
    state = {'cgroup_state': {'cgroup': 'present', 'freeze': freeze, 'frozen': frozen,
                              'populated': '1' if procs else '0', 'procs': procs, 'all_procs': procs, 'descendants': {}},
             'tasks': {'290': 'S:futex_wait', '300': 'D:__fuse_simple_request'} if procs else {'290': 'gone', '300': 'gone'},
             'agent_same_pid': True, 'agent_pids': [75], 'agent_exe_sha256': AGENT_SHA, 'agent_serves_socket': True,
             'check': {'outcome': 'ok', 'method': 'Check', 'conn': 'observe', 'stream': 1, 'code': 0,
                       'submission': 'submitted', 'latency_ms': 0.4, 'health_status': 1}, 'label': 'final',
             'observation_error': '', 'agent_exe_error': '', 'cgroups_mode': cgroups}
    if cgroups == 'systemd':
        state['unit'] = {'ActiveState': 'active' if procs else 'inactive', 'FreezerState': freezer}
    return state


def stop_records(run: str, cid: str, scenario: str, cgroups: str = 'systemd') -> Records:
    """Client, fused and final-observation records for one agent-stop scenario."""
    rec = Records(run)
    identity(rec)
    rec.add('prepare', 'prepared', 0.5, rootfs='/var/lib/finlab/rootfs', rootfs_helper_sha256=HELPER_SHA)
    rec.add('fused', 'mounted', 0.9, mnt='/run/finlab/mnt', hold_max_s=60)
    path = ('system.slice:nerdctl:' if cgroups == 'systemd' else '/finlab/') + cid
    check(rec, 'A', 1.0, 0.3)
    check(rec, 'B', 1.0, 0.1)
    for step in ('create-sandbox', 'create-container', 'start-container'):
        rec.add('client', 'setup-ok', 2.0, step=step, method={'create-sandbox': 'CreateSandbox', 'create-container': 'CreateContainer',
                'start-container': 'StartContainer'}[step], stream=rec.seq.get('client', 0) * 2 + 1, conn='A',
                latency_ms=0.0, code=0, outcome='ok', submission='submitted')
    rec.add('client', 'container-created', 2.0, cid=cid, cgroups_path=path)
    rec.add('client', 'wait-sent', 2.1, exec_id=cid, conn='A')
    if scenario != 'idle':
        rec.add('fused', 'read-received', 3.0, unique=4, pid=300)
        rec.add('client', 'held-request', 3.1, pid=300, unique=4, task='S:request_wait_answer')
    sample = final(cgroups, 'running', [290, 300] if scenario != 'idle' else [290])
    sample.pop('label')
    sample.pop('check')  # The sampler does not issue health RPCs.
    rec.add('client', 'sample', 9.8, **sample)
    rec.add('client', 'signal-sent', 10.0, signal=15, exec_id='')
    if scenario == 'idle':
        rec.add('client', 'signal-returned', 10.02, outcome='ok', latency_ms=20.0)
        rec.add('client', 'wait-returned', 10.05, outcome='ok', status=0)
        for sent in (13.0, 18.0):
            check(rec, 'A', sent, 0.5)
        for sent in range(11, 23):
            check(rec, 'B', float(sent), 0.4)
        end, observed = 24.0, final(cgroups, 'running', [])
    elif scenario == 'held-starved':
        check(rec, 'B', 11.0, 5000.0, 'deadline')
        check(rec, 'A', 13.0, 9000.0)
        rec.add('fused', 'read-released', 22.0, unique=4, by='marker')
        rec.add('client', 'signal-returned', 22.02, outcome='ok', latency_ms=12020.0)
        rec.add('client', 'wait-returned', 22.05, outcome='ok', status=0)
        for sent in range(22, 26):
            check(rec, 'B', float(sent), 0.4)
        end, observed = 26.0, final(cgroups, 'running', [])
    elif scenario == 'held-liveness-only':
        rec.add('client', 'signal-returned', 10.0015, outcome='ok', latency_ms=1.5)
        for sent in (13.0, 18.0):
            check(rec, 'A', sent, 0.5)
        for sent in range(11, 25):
            check(rec, 'B', float(sent), 0.4)
        rec.add('fused', 'read-released', 22.0, unique=4, by='marker')
        rec.add('client', 'wait-returned', 22.05, outcome='ok', status=137)
        end, observed = 26.0, final(cgroups, 'running', [])
    else:  # stranded-frozen: worker-handoff shape, deadline then frozen
        for sent in (13.0, 18.0, 23.0, 28.0, 33.0, 38.0, 43.0, 48.0, 53.0, 58.0, 63.0, 68.0):
            check(rec, 'A', sent, 0.5)
        for sent in range(11, 75):
            check(rec, 'B', float(sent), 0.4)
        rec.add('client', 'signal-returned', 70.0, outcome='deadline', latency_ms=60000.0)
        rec.add('fused', 'read-released', 72.0, unique=4, by='marker')
        end, observed = 80.0, final(cgroups, 'frozen', [290, 300])
    pending = ['wait'] if scenario == 'stranded-frozen' else []
    rec.add('client', 'client-end', end, reason='window-elapsed' if pending else 'complete', pending=pending, seen_pids=[290, 300])
    rec.add('observe', 'observation', end + 2, **observed)
    if scenario != 'idle':
        rec.add('release', 'release-requested', 21.9 if scenario != 'stranded-frozen' else 71.9)
    for r in rec.items:
        if r['source'] in {'client', 'observe'}:
            r.update(cid=cid, operation=run + ':stop:' + cid)
        if r['kind'] in {'signal-returned', 'wait-returned'}:
            r.update(method='SignalProcess' if r['kind'] == 'signal-returned' else 'WaitProcess', conn='A',
                     stream=r['seq'] * 2 + 1, code=0, submission='submitted', exec_id='' if r['kind'] == 'signal-returned' else cid)
            r.setdefault('latency_ms', (float((datetime.fromisoformat(r['t'].replace('Z', '+00:00')) - ORIGIN).total_seconds()) - 2.1) * 1000)
            r['sent_t'] = stamp(10.0 if r['kind'] == 'signal-returned' else 2.1)
        if r['kind'] == 'setup-ok':
            r['sent_t'] = r['t']
        if r['kind'] in {'setup-ok', 'wait-returned', 'signal-returned'} and r['outcome'] == 'ok':
            r['received_t'] = r['t']
        if r['source'] == 'observe':
            r['cgroup_state']['path'] = '/sys/fs/cgroup/' + ('system.slice/nerdctl-' + cid + '.scope' if cgroups == 'systemd' else 'finlab/' + cid)
            r.update(started_t=stamp(end + 1), completed_t=stamp(end + 1.5))
            r['check'].update(sent_t=stamp(end + 1.4), received_t=stamp(end + 1.4004))
        if r['kind'] == 'sample':
            r['cgroup_state']['path'] = '/sys/fs/cgroup/' + ('system.slice/nerdctl-' + cid + '.scope' if cgroups == 'systemd' else 'finlab/' + cid)
    # Synthesize allocation in send order, separately on each connection,
    # then mutex-style emission order. Never derive streams from record seq:
    # replies can arrive long after newer calls have been sent.
    streams = {'A': 1, 'B': 1}
    for r in sorted((r for r in rec.items if r['source'] == 'client' and 'method' in r), key=lambda r: r['sent_t']):
        r['stream'] = streams[r['conn']]
        streams[r['conn']] += 2
    rec.items.sort(key=lambda r: r['t'])
    rec.drop(lambda r: False)
    return rec


def order_records(run: str, mutate: str = '') -> Records:
    rec = Records(run)
    identity(rec)
    rec.add('prepare', 'prepared', 0.5, rootfs='/var/lib/finlab/rootfs', rootfs_helper_sha256=HELPER_SHA)
    rec.add('fused', 'mounted', 0.9, mnt='/run/finlab/mnt', hold_max_s=90)
    rec.add('order', 'order-start', 1.0, systemd_version='systemd 255 (255.4-1ubuntu8)')
    running = {'cgroup_state': {'freeze': '0', 'frozen': '0'}, 'unit': {'FreezerState': 'running'}}
    freezing = {'cgroup_state': {'freeze': '1', 'frozen': '0'}, 'unit': {'FreezerState': 'freezing'}}
    ok, aborted = {'exit': 0, 'stderr': '', 'bound_hit': False}, {'exit': 1, 'stderr': 'Call failed: Freeze operation aborted', 'bound_hit': False}

    def step(case: str, name: str, at: float, **fields: Any) -> None:
        rec.add('order', 'order-step', at, case=case, step=name, **fields)

    c = 'thaw-cancels-accepted-freeze'
    rec.add('order', 'held-request', 2.0, case=c, pid=400, unique=2)
    step(c, 'freeze-started', 2.1)
    step(c, 'freeze-accepted', 2.2, observed=True, observation=freezing)
    step(c, 'second-freeze-returned', 2.3, exit=1, stderr="Call failed: Previously requested freezer operation for unit 'x' is still in progress.", bound_hit=False)
    step(c, 'thaw-returned', 2.4, **ok)
    step(c, 'freeze-returned', 2.5, **aborted)
    step(c, 'after-thaw', 2.6, observation=running)
    c = 'thaw-before-freeze'
    rec.add('order', 'held-request', 3.0, case=c, pid=401, unique=4)
    step(c, 'thaw-returned', 3.1, **ok)
    step(c, 'after-thaw', 3.2, observation=running)
    step(c, 'freeze-started', 3.3)
    late = mutate != 'fenced'
    step(c, 'late-freeze-accepted', 3.4, observed=late, observation=freezing if late else running)
    step(c, 'restore-thaw-returned', 3.5, **ok)
    step(c, 'freeze-returned', 3.6, **aborted)
    step(c, 'after-restore', 3.7, observation=running)
    c = 'local-timeout-not-rollback'
    rec.add('order', 'held-request', 4.0, case=c, pid=402, unique=6)
    step(c, 'freeze-client-returned', 6.1, exit=1, stderr='Call failed: Connection timed out', bound_hit=False)
    if mutate != 'missing-step':
        step(c, 'after-client-timeout', 6.2, observation=freezing, still_freezing=True)
    step(c, 'restore-thaw-returned', 6.3, **ok)
    step(c, 'after-restore', 6.4, observation=running)
    rec.add('order', 'order-end', 7.0)
    rec.add('release', 'release-requested', 7.01)
    for index, case in enumerate(ORDER_CASES):
        h = next(r for r in rec.items if r.get('case') == case and r['kind'] == 'held-request')
        unit = 'finlab-' + run[:12] + '-' + case + '.scope'
        h['unit'] = unit
        rec.add('fused', 'read-received', 1.9 + index, pid=h['pid'], unique=h['unique'])
        rec.add('fused', 'read-released', 7.1 + index * 0.1, unique=h['unique'], by='marker')
        for r in rec.items:
            if r.get('case') == case and r['kind'] == 'order-step':
                r['unit'] = unit
                if 'observation' in r:
                    r['observation'] = copy.deepcopy(r['observation'])
                    cg = r['observation']['cgroup_state']
                    cg.update(path='/sys/fs/cgroup/system.slice/' + unit, cgroup='present', populated='1', procs=[h['pid']],
                              all_procs=[h['pid']], descendants={})
                    r['observation']['unit']['ActiveState'] = 'active'
    rec.items.sort(key=lambda r: r['t'])
    rec.drop(lambda r: False)
    return rec


def run_state(run: str, arm: dict[str, Any], cid: str, **overrides: Any) -> dict[str, Any]:
    from scripts.kata_lifecycle_verdict import RUN_STEPS
    required = RUN_STEPS | ({'agent-identity', 'final-observation'} if arm['kind'] == 'agent-stop' else set())
    if arm.get('reader') or arm['kind'] == 'systemd-freezer-order':
        required |= {'release'}
    steps = [{'name': name, 'status': 'ok', 'exit': 1 if name == 'name-free' else 0,
              'missing_confirmed': name == 'name-free', 'detail': 'state running' if name == 'ready-systemd' else ''}
             for name in sorted(required)]
    state = {'schema': 'finite.kata-lifecycle-run.v2', 'run': run, 'arm': arm['name'], 'kind': arm['kind'],
             'synthetic_cid': cid, 'inputs': {'image_id': IMAGE_ID, 'platform': 'linux/arm64', 'helper_sha256': HELPER_SHA},
             'steps': steps, 'runner_status': 'complete', 'creation': {'outcome': 'confirmed', 'nonce': run, 'reconciliation_required': False},
             'container': {'name': 'finlab-' + run, 'id': 'd' * 64, 'ownership_confirmed': True},
             'cleanup': {'attempted': True, 'guard': 'ok', 'removed': True,
                         'rm_exit': 0,
                         'audit': {'inspect_absent': True, 'label_matches': []}}}
    state.update(overrides)
    return state


def write_run(directory: Path, arm: dict[str, Any], state: dict[str, Any], lines: list[str]) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    (directory / 'arm.json').write_text(json.dumps(arm))
    (directory / 'run.json').write_text(json.dumps(state))
    (directory / 'records.jsonl').write_text(''.join(line + '\n' for line in lines))
    return directory
