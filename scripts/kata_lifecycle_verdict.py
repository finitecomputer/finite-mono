"""Strict offline verdict for one kata_lifecycle_lab run directory.

A run directory holds arm.json (named expectations), run.json (the runner's
steps, owned resources and cleanup audit) and records.jsonl (generation-tagged
records from the in-container helper). The verdict is:

- PASS (exit 0): every gate has complete evidence and every observed outcome
  equals the arm's named expectation.
- FAIL (exit 1): evidence is complete and some observed outcome differs.
- INCONCLUSIVE (exit 2): evidence is missing, malformed, from another
  generation or identity, or a runner step failed. Never PASS.

For expected-negative purposes PASS means the negative prediction matched.
Positive-control and candidate purposes additionally require lifecycle_success.
Version 2 requires new operation-bound evidence; version 1 receipts are not
upgraded or reinterpreted as version 2 successes. The claims stay separate:

- health_liveness: both observed Check connections stayed responsive. The
  lab does not execute a host monitor or measure VM shutdown.
- lab_stop_completion: init reaped, no task left, freezer running at the
  final observation, however long that took.
- stop_completed_before_release: a full ordered terminal observation follows
  signal and WaitProcess replies and precedes the daemon's release marker
  (null without a held request). WaitProcess alone is insufficient.
- correct_after_release: liveness, a successful signal and eventual stop
  completion. It does not imply completion before release.
- lifecycle_success: correct_after_release and a stop that did not wait for
  the held request.
"""

from __future__ import annotations

import json
import math
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

RECORD_SCHEMA = 'finite.kata-lifecycle-record.v2'
RUN_SCHEMA = 'finite.kata-lifecycle-run.v2'
ARM_SCHEMA = 'finite.kata-lifecycle-arm.v1'
VERDICT_SCHEMA = 'finite.kata-lifecycle-verdict.v2'
EXIT = {'PASS': 0, 'FAIL': 1, 'INCONCLUSIVE': 2}

STOP_EXPECTATIONS = {
    'held_request': {'accepted-before-signal', 'absent'},
    'signal': {'prompt-ok', 'ok-after-release', 'deadline-exceeded'},
    'same_connection': {'responsive', 'starved'},
    'independent_connection': {'responsive', 'starved', 'either'},
    'init': {'exited-promptly', 'exited-after-release', 'not-exited'},
    'stop_before_release': {'completed', 'incomplete', 'not-applicable'},
    'final_freezer': {'running', 'freezing', 'frozen'},
    'final_tasks': {'none', 'remaining'},
}
ORDER_CASES = {
    'thaw-cancels-accepted-freeze': 'cancelled',
    'thaw-before-freeze': 'late-freeze-applied',
    'local-timeout-not-rollback': 'freeze-still-pending',
}
THRESHOLDS = {'prompt_ms': 2000.0, 'responsive_ms': 1000.0, 'starved_ms': 3000.0,
              'liveness_span_s': 10.0, 'prompt_exit_s': 5.0}
SUCCESS_SHAPE = {'signal': {'prompt-ok', 'ok-after-release'}, 'same_connection': {'responsive'},
                 'independent_connection': {'responsive'},
                 'init': {'exited-promptly', 'exited-after-release'}, 'final_freezer': {'running'},
                 'final_tasks': {'none'}, 'stop_before_release': {'completed', 'not-applicable'}}
NEGATIVE_VALUES = {'deadline-exceeded', 'starved', 'not-exited', 'incomplete', 'freezing', 'frozen', 'remaining'}
OUTCOMES = {'ok', 'deadline', 'rpc-error', 'transport-error', 'not-serving', 'cancelled', 'malformed-response', 'pending', 'censored'}
DEFAULT_BOUNDS = {'total_s': 300, 'create_s': 60, 'ready_s': 60, 'exec_s': 30, 'client_margin_s': 60,
                  'release_s': 15, 'cleanup_s': 60, 'diagnostics_s': 60, 'settle_s': 2, 'order_s': 120}
RUN_STEPS = {'helper-identity', 'image-identity', 'name-free', 'label-free', 'create', 'start',
             'ready-agent-socket', 'ready-systemd', 'install-helper', 'helper-in-container', 'identity',
             'prepare', 'start-fused', 'ready-fuse', 'inner', 'collect fused.jsonl', 'collect ctr.jsonl',
             'collect order-readers.jsonl', 'collect journal'}
KINDS = {
    'identity': {'identity'}, 'prepare': {'prepared'}, 'release': {'release-requested'}, 'observe': {'observation'},
    'client': {'check-started', 'check', 'check-abandoned', 'check-pending', 'check-censored', 'setup-ok', 'setup-error', 'precondition-error', 'shutdown-error', 'container-created',
               'wait-sent', 'wait-returned', 'wait-returned-abandoned', 'signal-sent', 'signal-returned',
               'signal-returned-abandoned', 'held-request', 'sample', 'terminal-observation', 'late-response', 'client-end'},
    'order': {'order-start', 'order-end', 'held-request', 'order-step', 'case-setup-error'},
    'fused': {'mounted', 'fused-error', 'reply-error', 'read-exit', 'read-received', 'read-released', 'interrupt-received'},
    'init': {'init-start', 'init-signal', 'init-exit', 'reader-started', 'reader-start-error'},
}


class Inconclusive(Exception):
    """Evidence cannot support any verdict."""


def need(condition: bool, reason: str) -> None:
    if not condition:
        raise Inconclusive(reason)


def integer(value: Any, low: int = 0, high: int = 2**63 - 1) -> bool:
    return type(value) is int and low <= value <= high


def finite(value: Any, low: float = 0, high: float = 86400) -> bool:
    return type(value) in {int, float} and math.isfinite(value) and low <= value <= high


def hex_id(value: Any, length: int = 64) -> bool:
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{' + str(length) + '}', value) is not None


def pid_list(value: Any) -> bool:
    return isinstance(value, list) and all(integer(p, 1, 2**31 - 1) for p in value) and len(value) == len(set(value))


def rpc_evidence(record: dict[str, Any], method: str, require_submitted: bool = True) -> None:
    need(record.get('outcome') in OUTCOMES, 'unsupported RPC outcome')
    need(finite(record.get('latency_ms'), high=86400000), 'invalid RPC latency')
    unsent = not require_submitted and record.get('submission') == 'not-sent'
    need(record.get('method') == method and ((unsent and record.get('stream') == 0)
         or (integer(record.get('stream'), 1, 2**32 - 1) and record['stream'] % 2 == 1)), 'RPC request/response identity missing')
    need(record.get('submission') in ({'submitted'} if require_submitted else {'not-sent', 'uncertain', 'submitted'}), 'RPC remote submission unresolved')
    if not require_submitted and record['submission'] != 'submitted':
        need(record['outcome'] not in {'ok', 'not-serving', 'rpc-error', 'malformed-response'}, 'unsent/partial RPC claims a remote outcome')
    need(isinstance(record.get('conn'), str) and record['conn'] != '', 'RPC connection missing')
    need(integer(record.get('code'), -(2**31), 2**31 - 1), 'RPC code missing')
    if record['outcome'] == 'ok':
        need(record['code'] == 0, 'RPC success/code disagreement')
        if method == 'Check':
            need(type(record.get('health_status')) is int and record['health_status'] == 1, 'decoded health status missing or not SERVING')
        if method == 'WaitProcess':
            need(integer(record.get('status'), -(2**31), 2**31 - 1), 'decoded WaitProcess status missing')


def validate_bounds(arm: dict[str, Any]) -> dict[str, float]:
    overrides = arm.get('bounds', {})
    need(isinstance(overrides, dict) and set(overrides) <= set(DEFAULT_BOUNDS), 'arm-invalid: bounds')
    bounds = {**DEFAULT_BOUNDS, **overrides}
    for key, value in bounds.items():
        need(finite(value, low=0 if key == 'settle_s' else 0.001, high=3600), f'arm-invalid: {key}')
    need(bounds['settle_s'] <= min(30, bounds['total_s']), 'arm-invalid: settle_s')
    for key, default in {'client_max_s': 120, 'window_s': 20, 'hold_max_s': 90, 'release_after_s': 12}.items():
        need(finite(arm.get(key, default), low=0 if key == 'release_after_s' else 0.001, high=3600), f'arm-invalid: {key}')
    thresholds = arm.get('thresholds', {})
    need(isinstance(thresholds, dict) and set(thresholds) <= set(THRESHOLDS), 'arm-invalid: thresholds')
    for key, value in {**THRESHOLDS, **thresholds}.items():
        need(finite(value, 0.001, 3600000), f'arm-invalid: threshold {key}')
    th = {**THRESHOLDS, **thresholds}
    need(th['responsive_ms'] < th['starved_ms'], 'arm-invalid: liveness thresholds')
    return bounds


def parse_time(value: Any) -> float:
    need(isinstance(value, str) and value.endswith('Z'), 'malformed timestamp')
    base, _, fraction = value[:-1].partition('.')
    try:
        stamp = datetime.strptime(base, '%Y-%m-%dT%H:%M:%S').replace(tzinfo=timezone.utc)
    except ValueError as error:
        raise Inconclusive('malformed timestamp') from error
    need(fraction == '' or (fraction.isdigit() and len(fraction) <= 9), 'malformed timestamp')
    return stamp.timestamp() + (int(fraction[:9].ljust(9, '0')) / 1e9 if fraction else 0.0)


def load_json(path: Path) -> dict[str, Any]:
    need(path.is_file(), f'missing {path.name}')
    try:
        value = json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise Inconclusive(f'malformed {path.name}') from error
    need(isinstance(value, dict), f'malformed {path.name}')
    return value


def validate_arm(arm: dict[str, Any]) -> None:
    need(isinstance(arm, dict), 'arm-invalid: object')
    need(arm.get('schema') == ARM_SCHEMA, 'arm-invalid: schema')
    need(isinstance(arm.get('name'), str) and arm['name'] != '', 'arm-invalid: name')
    purpose = arm.get('purpose')
    need(purpose in {'positive-control', 'expected-negative', 'candidate-qualification'}, 'arm-invalid: purpose')
    identity = arm.get('identity')
    need(isinstance(identity, dict) and isinstance(identity.get('systemd_version_prefix'), str)
         and identity['systemd_version_prefix'] != '', 'arm-invalid: identity')
    inputs = arm.get('inputs')
    need(isinstance(inputs, dict) and set(inputs) == {'image_id', 'platform', 'helper_sha256'}, 'arm-invalid: inputs')
    need(isinstance(inputs['image_id'], str) and inputs['image_id'].startswith('sha256:')
         and hex_id(inputs['image_id'][7:]) and hex_id(inputs['helper_sha256'])
         and inputs['platform'] in {'linux/arm64', 'linux/amd64'}, 'arm-invalid: immutable inputs')
    validate_bounds(arm)
    expect = arm.get('expect')
    need(isinstance(expect, dict), 'arm-invalid: expect')
    if arm.get('kind') == 'systemd-freezer-order':
        need(expect == {'cases': ORDER_CASES}, 'arm-invalid: order cases')
        need(purpose == 'expected-negative', 'arm-invalid: order arm includes negative primitives')
        return
    need(arm.get('kind') == 'agent-stop', 'arm-invalid: kind')
    need(hex_id(identity.get('agent_sha256')), 'arm-invalid: agent_sha256')
    need(integer(arm.get('signal', 15), 1, 64) and arm.get('init_on_term', 'exit') in {'exit', 'stay'}, 'arm-invalid: signal/init')
    need(identity.get('agent_log_level') == 'default', 'arm-invalid: qualification requires the default agent log level')
    need(arm.get('cgroups') in {'systemd', 'fs'} and isinstance(arm.get('reader'), bool), 'arm-invalid: cgroups/reader')
    need(set(expect) == set(STOP_EXPECTATIONS), 'arm-invalid: expectation keys')
    for key, allowed in STOP_EXPECTATIONS.items():
        need(expect[key] in allowed, f'arm-invalid: expect.{key}')
    need((expect['held_request'] == 'accepted-before-signal') == arm['reader'], 'arm-invalid: reader/held_request')
    need((expect['stop_before_release'] == 'not-applicable') == (not arm['reader']), 'arm-invalid: stop_before_release')
    if purpose in {'positive-control', 'candidate-qualification'}:
        for key, allowed in SUCCESS_SHAPE.items():
            need(expect[key] in allowed, f'arm-invalid: {purpose} must expect lifecycle success ({key})')
    else:
        need(any(v in NEGATIVE_VALUES for v in expect.values()), 'arm-invalid: expected-negative arm predicts no negative outcome')


def load_records(path: Path, run: str) -> list[dict[str, Any]]:
    need(path.is_file(), 'missing records.jsonl')
    records: list[dict[str, Any]] = []
    last_seq: dict[str, int] = {}
    last_time: dict[str, float] = {}
    for number, line in enumerate(path.read_text().splitlines(), 1):
        try:
            record = json.loads(line)
        except ValueError as error:
            raise Inconclusive(f'malformed record line {number}') from error
        need(isinstance(record, dict), f'malformed record line {number}')
        need(record.get('schema') == RECORD_SCHEMA, f'unknown record schema line {number}')
        need(record.get('run') == run, f'wrong-generation record line {number}')
        source, seq = record.get('source'), record.get('seq')
        need(isinstance(source, str) and type(seq) is int and isinstance(record.get('kind'), str), f'malformed record line {number}')
        allowed = {'reader-open-error', 'reader-read-start', 'reader-read-done'} if re.fullmatch('reader-[1-9][0-9]*', source) else KINDS.get(source, set())
        need(record['kind'] in allowed, f'unknown record source/kind line {number}')
        need(record['kind'] not in {'fused-error', 'reply-error', 'read-exit', 'reader-open-error', 'reader-start-error'},
             'synthetic workload/observation failed')
        need(seq == last_seq.get(source, 0) + 1, f'record sequence gap or duplicate for {source} at line {number}')
        last_seq[source] = seq
        record['_t'] = parse_time(record.get('t'))
        need(record['_t'] >= last_time.get(source, record['_t']), f'nonmonotone emission for {source}')
        last_time[source] = record['_t']
        records.append(record)
    need(records != [], 'zero records')
    return records


def only(records: list[dict[str, Any]], source: str, kind: str) -> dict[str, Any]:
    found = [r for r in records if r['source'] == source and r['kind'] == kind]
    need(len(found) == 1, f'expected exactly one {source}/{kind} record, found {len(found)}')
    return found[0]


def check_runner(run: dict[str, Any], arm: dict[str, Any]) -> dict[str, Any]:
    need(run.get('schema') == RUN_SCHEMA, 'run.json schema')
    need(run.get('arm') == arm['name'], 'run.json names a different arm')
    need(run.get('kind') == arm['kind'] and hex_id(run.get('synthetic_cid')), 'run operation identity missing')
    need(run.get('inputs') == arm['inputs'], 'run immutable inputs mismatch')
    steps = run.get('steps')
    need(isinstance(steps, list) and steps != [], 'runner steps missing')
    names = []
    for step in steps:
        need(isinstance(step, dict) and isinstance(step.get('name'), str), 'malformed runner step')
        need(step.get('status') == 'ok', f"runner step {step.get('name')} {step.get('status')} (exit {step.get('exit')})")
        code = step.get('exit')
        need(integer(code, -255, 255), f"runner step {step['name']} exit missing")
        if step['name'] == 'name-free':
            need(code != 0 and step.get('missing_confirmed') is True, 'name-free lacks expected absence failure')
        elif step['name'] == 'ready-systemd':
            need(code in {0, 1} and step.get('detail') in {'state running', 'state degraded'}, 'systemd readiness exit/state')
        else:
            need(code == 0, f"runner step {step['name']} unexpected exit {code}")
        names.append(step['name'])
    required = RUN_STEPS | ({'agent-identity', 'final-observation'} if arm['kind'] == 'agent-stop' else set())
    if arm.get('reader') or arm['kind'] == 'systemd-freezer-order':
        required |= {'release'}
    need(required <= set(names) and len(names) == len(set(names)), 'required runner steps missing or duplicated')
    need(run.get('runner_status') == 'complete', f"runner status {run.get('runner_status')}")
    container = run.get('container') or {}
    need(container.get('ownership_confirmed') is True, 'container ownership not confirmed')
    need(hex_id(container.get('id')) and container.get('name') == 'finlab-' + run['run'], 'container identity mismatch')
    creation = run.get('creation') or {}
    need(creation.get('outcome') == 'confirmed' and creation.get('nonce') == run['run']
         and creation.get('reconciliation_required') is False, 'creation outcome unresolved')
    cleanup = run.get('cleanup') or {}
    audit = cleanup.get('audit') or {}
    need(type(cleanup.get('attempted')) is bool and isinstance(audit.get('label_matches'), list), 'cleanup audit malformed')
    if cleanup['attempted'] and cleanup.get('removed') is True and cleanup.get('note') != 'container already absent':
        need(cleanup.get('rm_exit') == 0 and type(cleanup.get('rm_exit')) is int, 'cleanup removal exit missing or failed')
    clean = (cleanup.get('guard') == 'ok' and cleanup.get('removed') is True
             and audit.get('inspect_absent') is True and audit.get('label_matches') == [])
    return {'teardown': 'clean' if clean else 'leaked-or-failed', 'cleanup': cleanup}


def validate_identity(identity: dict[str, Any], arm: dict[str, Any], run: dict[str, Any]) -> dict[str, Any]:
    need(identity.get('schema') == RECORD_SCHEMA and identity.get('run') == run['run']
         and identity.get('source') == 'identity' and identity.get('kind') == 'identity'
         and identity.get('seq') == 1, 'identity schema/generation/source mismatch')
    parse_time(identity.get('t'))
    expected = arm['identity']
    need(identity.get('helper_sha256') == run.get('inputs', {}).get('helper_sha256'), 'helper identity mismatch')
    need(isinstance(identity.get('systemd_version'), str)
         and identity['systemd_version'].startswith(expected['systemd_version_prefix']), 'systemd version mismatch')
    need(identity.get('cgroup2') is True, 'cgroup v2 not observed')
    need(all(identity.get(k) == '' for k in ('helper_error', 'systemd_version_error', 'observation_error', 'agent_unit_error')),
         'identity observation error or missing error audit')
    need(isinstance(identity.get('agent_unit'), str) and identity['agent_unit'] != ''
         and isinstance(identity.get('kernel_release'), str) and identity['kernel_release'] != ''
         and integer(identity.get('nproc'), 1, 65536), 'system identity incomplete')
    if arm['kind'] == 'agent-stop':
        need(integer(identity.get('agent_pid'), 1, 2**31 - 1)
             and pid_list(identity.get('agent_pids')) and identity['agent_pids'] == [identity['agent_pid']], 'not exactly one kata-agent')
        need(identity.get('agent_exe_sha256') == expected['agent_sha256'], 'serving agent executable mismatch')
        need(identity.get('agent_serves_socket') is True, 'agent does not serve the lab socket')
        need(integer(identity.get('agent_tokio_workers'), 1, 65536), 'agent worker identity incomplete')
        need(identity.get('agent_socket') == '/run/fin175/agent.sock' and identity.get('agent_exe_error') == ''
             and isinstance(identity.get('agent_exe_path'), str) and identity['agent_exe_path'].startswith('/'), 'serving process identity incomplete')
        need(type(identity.get('agent_log_level_env_set')) is bool
             and isinstance(identity.get('agent_log_level_env'), str), 'agent logging identity missing')
        need(isinstance(identity.get('kernel_agent_params'), list)
             and all(isinstance(p, str) for p in identity['kernel_agent_params']), 'kernel_agent_params malformed')
        level = identity.get('agent_log_level_env') if identity.get('agent_log_level_env_set') else None
        need(level in {None, 'info'} and not any(p.startswith('agent.log=') for p in identity.get('kernel_agent_params', [])),
             'agent log level is not the default')
    return {key: identity.get(key) for key in ('agent_exe_sha256', 'agent_exe_path', 'agent_tokio_workers',
                                               'systemd_version', 'kernel_release', 'nproc')}


def check_identity(records: list[dict[str, Any]], arm: dict[str, Any], run: dict[str, Any]) -> dict[str, Any]:
    identity = only(records, 'identity', 'identity')
    prepared = only(records, 'prepare', 'prepared')
    need(prepared.get('rootfs') == '/var/lib/finlab/rootfs'
         and prepared.get('rootfs_helper_sha256') == run['inputs']['helper_sha256'], 'prepared helper identity mismatch')
    mounted = only(records, 'fused', 'mounted')
    need(mounted.get('mnt') == '/run/finlab/mnt' and mounted.get('hold_max_s') == arm.get('hold_max_s', 90),
         'FUSE setup identity/bound mismatch')
    authorized = [r for r in records if r['source'] in {'prepare', 'fused', 'client', 'order', 'init'}]
    need(all(identity['_t'] < r['_t'] for r in authorized), 'identity preflight must precede prepare/setup/injection')
    need(prepared['_t'] <= mounted['_t'], 'prepare must precede FUSE mount')
    return validate_identity(identity, arm, run)


def liveness(checks: list[dict[str, Any]], start: float, end: float, th: dict[str, float]) -> str:
    inside = [c for c in checks if start <= parse_time(c.get('sent_t')) <= end]
    need(inside != [], 'no check evidence in the signal window')
    if any(c['outcome'] == 'not-serving' for c in inside):
        return 'unhealthy'
    if any(c['outcome'] == 'deadline' or c['latency_ms'] >= th['starved_ms'] for c in inside):
        return 'starved'
    need(all(c['outcome'] == 'ok' for c in inside), 'Check evidence censored or observation failed in the signal window')
    if all(c['latency_ms'] <= th['responsive_ms'] for c in inside):
        return 'responsive'
    return 'degraded'


def snapshot_check(record: dict[str, Any]) -> tuple[dict[str, Any], float]:
    """The producer calls Check inside snapshot(), on B or observe itself.

    A separate late-response may describe this RPC later. Its receipt does
    not make a Check outside this enclosure a fresh snapshot observation.
    """
    started, completed = parse_time(record.get('started_t')), parse_time(record.get('completed_t'))
    need(started <= completed <= record['_t'], 'snapshot enclosure unordered')
    health = record.get('check')
    need(isinstance(health, dict), 'snapshot Check missing or malformed')
    rpc_evidence(health, 'Check', require_submitted=False)
    expected_conn = 'B' if record['source'] == 'client' and record['kind'] == 'terminal-observation' else 'observe'
    need(health['conn'] == expected_conn, 'snapshot Check supplying connection mismatch')
    sent = parse_time(health.get('sent_t'))
    need(started <= sent <= completed, 'snapshot Check send outside enclosure')
    if 'received_t' in health:
        received = parse_time(health['received_t'])
        need(sent <= received <= completed, 'snapshot Check receipt outside enclosure')
    else:
        need(health['outcome'] not in {'ok', 'not-serving', 'rpc-error', 'malformed-response'},
             'snapshot Check remote receipt missing')
    return health, completed


def snapshot_reply_origin(reply: dict[str, Any], terminal: list[dict[str, Any]]) -> None:
    """A request-ID-less late Check must report a client terminal snapshot's RPC.

    snapshot() is the only Check without a wrapper ID, so its reply names that
    terminal-observation Check's B stream and attempt origin. abandon() can emit
    the reply before the record. A never-sent attempt has no remote reply.
    """
    need(integer(reply.get('stream'), 1) and reply.get('submission') == 'submitted'
         and isinstance(reply.get('sent_t'), str), 'direct Check late-response origin identity malformed')
    origin = [c for c in terminal if (c['conn'], c['stream']) == (reply['conn'], reply['stream'])]
    need(origin != [], 'direct Check late-response without matching terminal snapshot Check')
    need(all(c['sent_t'] == reply['sent_t'] and c.get('request_id') is None and c['submission'] != 'not-sent'
             for c in origin), 'direct Check late-response contradicts terminal snapshot Check')


def rpc_observations(client: list[dict[str, Any]], snapshots: list[dict[str, Any]]) -> list[tuple[dict[str, Any], float]]:
    entries = [(r, r['_t']) for r in client if 'method' in r]
    entries += [snapshot_check(r) for r in snapshots]
    return entries


def correlate_rpcs(client: list[dict[str, Any]], snapshots: list[dict[str, Any]], sent_markers: dict[str, float]) -> None:
    """One unary request per connection/stream; repeated observations must agree.

    A local cancellation and a later wire reply describe the same request.
    Their local outcomes may differ; two decoded remote replies may not.
    """
    calls: dict[tuple[str, int], dict[str, Any]] = {}
    for r, observed_at in rpc_observations(client, snapshots):
        method = r.get('method')
        need(method in {'Check', 'CreateSandbox', 'CreateContainer', 'StartContainer', 'WaitProcess', 'SignalProcess'},
             'unknown RPC method')
        rpc_evidence(r, method, require_submitted=method != 'Check')
        expected = {'check': 'Check', 'check-abandoned': 'Check', 'check-pending': 'Check', 'check-censored': 'Check',
                    'wait-returned': 'WaitProcess', 'wait-returned-abandoned': 'WaitProcess',
                    'signal-returned': 'SignalProcess', 'signal-returned-abandoned': 'SignalProcess'}.get(r.get('kind'))
        need(expected is None or method == expected, 'RPC method contradicts record kind')
        sent = parse_time(r.get('sent_t'))
        received = parse_time(r['received_t']) if 'received_t' in r else observed_at
        need(sent <= received <= observed_at, 'RPC request/response chronology')
        need(abs((received - sent) * 1000 - r['latency_ms']) <= 50, 'RPC timestamp/latency inconsistent')
        if method in sent_markers:
            need(abs(sent - sent_markers[method]) <= 0.05, 'RPC send marker disagreement')
        if r['submission'] == 'not-sent':
            continue  # The attempt exists, but no transport stream was allocated.
        key = (r['conn'], r['stream'])
        identity = (method, sent, r.get('exec_id'), r.get('request_id'))
        remote = r['outcome'] in {'ok', 'not-serving', 'rpc-error', 'malformed-response'} or r.get('received_t') is not None
        reply = (r['outcome'], r['code'], r.get('status'), r.get('health_status'), received)
        if key in calls:
            prior = calls[key]
            need(prior['identity'] == identity, 'connection/stream reused for a different RPC request')
            if prior['reply'] is not None and remote:
                need(prior['reply'] == reply, 'contradictory observations of one RPC response')
            if remote:
                prior['reply'] = reply
        else:
            calls[key] = {'identity': identity, 'reply': reply if remote else None}
    for conn in {key[0] for key in calls}:
        ordered = sorted((stream, data['identity'][1]) for (name, stream), data in calls.items() if name == conn)
        need(all(a[1] <= b[1] for a, b in zip(ordered, ordered[1:])), 'stream allocation contradicts request send order')


def check_attempts(client: list[dict[str, Any]], snapshots: list[dict[str, Any]], end: float) -> list[dict[str, Any]]:
    """Reconcile wrapper attempts and direct snapshot RPCs by genuine stream.

    Snapshot calls have no wrapper request ID/start record. Repeated evidence
    for one transport request contributes one Check with its maximum elapsed
    wait; an unanswered wrapper start still retains its censored interval.
    """
    starts: dict[tuple[str, int], dict[str, Any]] = {}
    results: dict[tuple[str, int], list[tuple[dict[str, Any], float]]] = {}
    direct = []
    for r in client:
        if r['kind'] == 'check-started':
            key = (r.get('conn'), r.get('request_id'))
            need(key[0] in {'A', 'B'} and integer(key[1], 1), 'Check attempt identity missing')
            need(key not in starts and parse_time(r.get('sent_t')) <= r['_t'] <= end, 'Check attempt order/duplicate')
            starts[key] = r
        elif r['kind'] in {'check', 'check-abandoned', 'check-pending', 'check-censored'} or (r['kind'] == 'late-response' and r.get('method') == 'Check'):
            rpc_evidence(r, 'Check', require_submitted=False)
            key = (r.get('conn'), r.get('request_id'))
            need(key[0] in {'A', 'B'}, 'Check result connection missing')
            if r['kind'] == 'late-response' and key[1] is None:
                # Only snapshot() calls Check without a request ID, on B or on
                # observe, whose connection has no late callback.
                need(key[0] == 'B', 'direct Check late-response outside snapshot connection B')
                direct.append((r, r['_t']))
            else:
                need(integer(key[1], 1), 'Check result attempt identity missing')
                results.setdefault(key, []).append((r, r['_t']))
    need(set(results) <= set(starts), 'Check completion without attempted request')
    checks = []
    streams: dict[tuple[str, int], list[tuple[dict[str, Any], float]]] = {}
    for key, start in starts.items():
        sent = parse_time(start['sent_t'])
        observations = results.get(key, [])
        for r, observed_at in observations:
            need(r.get('sent_t') == start['sent_t'] and observed_at >= start['_t'], 'Check start/result disagreement')
        need(len({r['stream'] for r, _ in observations}) <= 1, 'Check attempt uses multiple streams')
        if observations and observations[0][0]['submission'] != 'not-sent':
            r = observations[0][0]
            streams.setdefault((r['conn'], r['stream']), []).extend(observations)
        else:
            # No stream exists for an unsent or unobserved wrapper attempt.
            latency = max((r['latency_ms'] for r, _ in observations), default=max(0, end - sent) * 1000)
            checks.append({'conn': key[0], 'sent_t': start['sent_t'], 'latency_ms': latency,
                           '_t': end, 'outcome': 'deadline' if any(r['outcome'] == 'deadline' for r, _ in observations) else 'censored'})
    nested = [(r, snapshot_check(r)) for r in snapshots]
    terminal = [check for r, (check, _) in nested if r['kind'] == 'terminal-observation']
    for reply, _ in direct:
        snapshot_reply_origin(reply, terminal)
    direct += [observation for _, observation in nested]
    for r, observed_at in direct:
        if r['submission'] == 'not-sent':
            checks.append({'conn': r['conn'], 'sent_t': r['sent_t'], 'latency_ms': r['latency_ms'],
                           '_t': observed_at, 'outcome': 'deadline' if r['outcome'] == 'deadline' else 'censored'})
        else:
            streams.setdefault((r['conn'], r['stream']), []).append((r, observed_at))
    for key, observations in streams.items():
        sent_times = {r['sent_t'] for r, _ in observations}
        need(len(sent_times) == 1, 'Check stream reused for a different request')
        # A received reply can resolve short censoring. Keep the longest known
        # elapsed wait: a later healthy reply cannot erase earlier slowness.
        replies = [(r, t) for r, t in observations if r['outcome'] in {'ok', 'not-serving', 'deadline'}]
        latency = max(r['latency_ms'] for r, _ in observations)
        outcome = 'deadline' if any(r['outcome'] == 'deadline' for r, _ in replies) else replies[-1][0]['outcome'] if replies else 'censored'
        checks.append({'conn': key[0], 'sent_t': next(iter(sent_times)), 'latency_ms': latency,
                       '_t': min((parse_time(r['received_t']) if 'received_t' in r else t for r, t in replies), default=end),
                       'outcome': outcome})
    return checks


def freezer_state(final: dict[str, Any], cgroups: str) -> tuple[str, str]:
    cg = final.get('cgroup_state')
    need(isinstance(cg, dict) and cg.get('cgroup') in {'present', 'absent'}, 'final cgroup state missing')
    if cgroups == 'systemd':
        unit = final.get('unit')
        need(isinstance(unit, dict) and 'error' not in unit and 'FreezerState' in unit, 'final unit state missing')
    if cg['cgroup'] == 'absent':
        if cgroups == 'systemd':
            need(unit['FreezerState'] == 'running' and unit.get('ActiveState') == 'inactive',
                 'absent cgroup/unit disagreement')
        return 'running', 'none'
    need({'freeze', 'frozen', 'populated', 'procs'} <= set(cg), 'final cgroup files missing')
    nodes = [cg, *cg.get('descendants', {}).values()]
    need(isinstance(cg.get('descendants'), dict), 'descendant inventory missing')
    all_procs = set()
    for node in nodes:
        need(isinstance(node, dict) and {'freeze', 'frozen', 'populated', 'procs'} <= set(node)
             and 'error' not in node, 'cgroup observation incomplete')
        need(pid_list(node['procs']) and all(node[k] in {'0', '1'} for k in ('freeze', 'frozen', 'populated')),
             'cgroup files malformed')
        need(not node['procs'] or node['populated'] == '1', 'cgroup population/procs disagreement')
        all_procs.update(node['procs'])
    need(pid_list(cg.get('all_procs')) and set(cg['all_procs']) == all_procs, 'descendant PID inventory mismatch')
    tasks = 'remaining' if all_procs or any(n['populated'] == '1' for n in nodes) else 'none'
    state = {('0', '0'): 'running', ('1', '1'): 'frozen', ('1', '0'): 'freezing'}.get((cg['freeze'], cg['frozen']))
    need(state is not None, 'final freezer state unknown')
    if cgroups == 'systemd':
        need(final['unit']['FreezerState'] == state, 'unit FreezerState disagrees with the kernel')
    if state == 'running' and any((n['freeze'], n['frozen']) != ('0', '0') for n in nodes):
        state = 'frozen' if any(n['frozen'] == '1' for n in nodes) else 'freezing'
    return state, tasks


def terminal_snapshot(record: dict[str, Any], arm: dict[str, Any], run: dict[str, Any],
                      agent: dict[str, Any], seen: set[int], after: float) -> tuple[str, str, bool, float]:
    need(record.get('cid') == run['synthetic_cid'] and record.get('operation') == run['run'] + ':stop:' + run['synthetic_cid'],
         'observation operation mismatch')
    started, completed = parse_time(record.get('started_t')), parse_time(record.get('completed_t'))
    need(after <= started <= completed <= record['_t'], 'stale or unordered terminal observation')
    need(record.get('cgroups_mode') == arm['cgroups'], 'observation cgroups identity mismatch')
    cg = record.get('cgroup_state') or {}
    path = '/sys/fs/cgroup/' + ('system.slice/nerdctl-' + run['synthetic_cid'] + '.scope'
                              if arm['cgroups'] == 'systemd' else 'finlab/' + run['synthetic_cid'])
    need(cg.get('path') == path and 'error' not in cg, 'cgroup path/observation error')
    freezer, population = freezer_state(record, arm['cgroups'])
    tasks = record.get('tasks')
    need(isinstance(tasks, dict) and all(isinstance(k, str) and k.isdigit() and integer(int(k), 1, 2**31 - 1)
                                       and isinstance(v, str) for k, v in tasks.items()), 'task inventory malformed')
    need(seen <= {int(k) for k in tasks}, 'retained PID inventory incomplete')
    need(all(v == 'gone' or re.fullmatch('[RSDZTtXxIP]:.*', v) for v in tasks.values()), 'task observation unknown')
    population = 'remaining' if population == 'remaining' or any(v != 'gone' for v in tasks.values()) else 'none'
    need(record.get('agent_same_pid') is True and pid_list(record.get('agent_pids')) and record['agent_pids'] == [agent['agent_pid']]
         and record.get('agent_exe_sha256') == agent['agent_exe_sha256'] and record.get('agent_serves_socket') is True,
         'agent restarted or serving identity unknown')
    need(record.get('observation_error') == '' and record.get('agent_exe_error') == ''
         and 'shutdown_error' not in record, 'terminal observation error')
    health, _ = snapshot_check(record)
    rpc_evidence(health, 'Check')
    need(health['outcome'] in {'ok', 'deadline', 'not-serving'}, 'final health observation unknown')
    return freezer, population, health['outcome'] == 'ok', completed


def observe_stop(records: list[dict[str, Any]], arm: dict[str, Any], run: dict[str, Any]) -> dict[str, Any]:
    th = {**THRESHOLDS, **arm.get('thresholds', {})}
    client = [r for r in records if r['source'] == 'client']
    need(not [r for r in client if r['kind'] in {'setup-error', 'precondition-error', 'shutdown-error'}], 'client setup or precondition failed')
    setup = [r for r in client if r['kind'] == 'setup-ok']
    steps = {r.get('step') for r in setup}
    need(steps == {'create-sandbox', 'create-container', 'start-container'}, 'agent setup incomplete')
    need(len(setup) == 3, 'duplicated agent setup')
    for record in setup:
        rpc_evidence(record, {'create-sandbox': 'CreateSandbox', 'create-container': 'CreateContainer', 'start-container': 'StartContainer'}[record['step']])
        need(record['outcome'] == 'ok', 'agent setup not successful')
    setup_times = {r['step']: r['_t'] for r in setup}
    need(setup_times['create-sandbox'] <= setup_times['create-container'] <= setup_times['start-container'],
         'agent setup operation order')
    created = only(records, 'client', 'container-created')
    need(created.get('cid') == run.get('synthetic_cid'), 'synthetic container id mismatch')
    expected_path = ('system.slice:nerdctl:' if arm['cgroups'] == 'systemd' else '/finlab/') + run['synthetic_cid']
    need(created.get('cgroups_path') == expected_path, 'cgroupsPath mismatch')
    sent = only(records, 'client', 'signal-sent')
    end = only(records, 'client', 'client-end')
    t_sig = sent['_t']
    wait_sent = only(records, 'client', 'wait-sent')
    need(wait_sent.get('exec_id') == run['synthetic_cid'] and wait_sent.get('conn') == 'A'
         and created['_t'] <= wait_sent['_t'] < t_sig, 'WaitProcess request identity/order missing')
    operation = run['run'] + ':stop:' + run['synthetic_cid']
    for record in client:
        need(record.get('cid') == run['synthetic_cid'] and record.get('operation') == operation,
             'client operation identity mismatch')
    pre_samples = [r for r in client if r['kind'] == 'sample' and r['_t'] < t_sig and r.get('cgroup_state', {}).get('cgroup') == 'present']
    need(pre_samples, 'no owned pre-signal task inventory')
    need(sent.get('signal') == arm.get('signal', 15) and type(sent.get('signal')) is int
         and sent.get('exec_id') == '', 'signal request does not match the arm')
    snapshots = [r for r in records if (r['source'] == 'client' and r['kind'] == 'terminal-observation')
                 or (r['source'] == 'observe' and r['kind'] == 'observation')]
    checks = check_attempts(client, snapshots, end['_t'])
    need(created['_t'] < t_sig and end['_t'] >= t_sig, 'client operation order invalid')
    pre = [c for c in checks if parse_time(c.get('sent_t')) < t_sig]
    for conn in ('A', 'B'):
        ready = [c for c in pre if c.get('conn') == conn and c['_t'] < min(r['_t'] for r in client if r['kind'] == 'setup-ok')]
        need(ready and all(c.get('outcome') == 'ok' for c in ready), 'readiness checks missing or failed before setup')
    held = [r for r in client if r['kind'] == 'held-request']
    received = [r for r in records if r['source'] == 'fused' and r['kind'] == 'read-received']
    released = [r for r in records if r['source'] == 'fused' and r['kind'] == 'read-released']
    t_release = None
    if arm['reader']:
        need(len(held) == 1 and len(received) == 1, 'held request evidence missing')
        h, rr = held[0], received[0]
        need(integer(h.get('pid'), 1, 2**31 - 1) and integer(h.get('unique'), 1, 2**64 - 1), 'held identifiers must be nonzero')
        need(integer(rr.get('pid'), 1, 2**31 - 1) and integer(rr.get('unique'), 1, 2**64 - 1), 'daemon held identifiers malformed')
        need(h.get('pid') == rr.get('pid') and h.get('unique') == rr.get('unique'), 'held request identity mismatch')
        need(any(h['pid'] in r['cgroup_state'].get('all_procs', []) for r in pre_samples), 'held request not bound to owned cgroup tasks')
        need(rr['_t'] < t_sig and h['_t'] < t_sig, 'request was not held before the signal')
        need(len(released) == 1 and released[0].get('unique') == rr.get('unique'), 'held request release missing')
        need(integer(released[0].get('unique'), 1, 2**64 - 1), 'daemon release identifier malformed')
        need(released[0].get('by') == 'marker', 'hold ended by its safety bound, not the runner release')
        release = only(records, 'release', 'release-requested')
        need(release['_t'] >= t_sig and release['_t'] <= released[0]['_t'], 'release ordering/correlation missing')
        t_release = released[0]['_t']
        need(t_release > t_sig, 'request released before the signal')
        held_obs = 'accepted-before-signal'
    else:
        need(held == [] and received == [], 'unexpected held request')
        held_obs = 'absent'
    returned = [r for r in client if r['kind'] == 'signal-returned']
    need(len(returned) == 1 and 'signal' not in end.get('pending', []), 'signal result missing')
    sig = returned[0]
    rpc_evidence(sig, 'SignalProcess')
    need(sig.get('conn') == 'A' and sig.get('exec_id') == '' and sig['_t'] >= t_sig
         and abs((sig['_t'] - t_sig) * 1000 - sig['latency_ms']) <= 50, 'signal response/order mismatch')
    if sig.get('outcome') == 'ok' and sig.get('latency_ms', 1e12) <= th['prompt_ms']:
        signal = 'prompt-ok'
    elif sig.get('outcome') == 'ok' and t_release is not None and sig['_t'] >= t_release:
        signal = 'ok-after-release'
    elif sig.get('outcome') == 'deadline':
        signal = 'deadline-exceeded'
    else:
        signal = 'other:' + str(sig.get('outcome'))
    span_end = max(sig['_t'], t_sig + th['liveness_span_s'])
    need(end['_t'] >= span_end, 'client ended before the liveness span')
    same = liveness([c for c in checks if c.get('conn') == 'A'], t_sig, span_end, th)
    independent = liveness([c for c in checks if c.get('conn') == 'B'], t_sig, span_end, th)
    waits = [r for r in client if r['kind'] == 'wait-returned']
    pending_wait = 'wait' in end.get('pending', [])
    need(isinstance(end.get('pending'), list) and set(end['pending']) <= {'signal', 'wait'}
         and len(end['pending']) == len(set(end['pending']))
         and end.get('reason') in {'complete', 'window-elapsed', 'max-reached'}, 'client-end malformed')
    need(pid_list(end.get('seen_pids')) and end['seen_pids'] != [], 'retained PID inventory missing')
    need(len(waits) + pending_wait == 1, 'init wait evidence inconsistent')
    if pending_wait:
        init = 'not-exited'
    else:
        wait = waits[0]
        rpc_evidence(wait, 'WaitProcess')
        need(integer(wait.get('status'), -(2**31), 2**31 - 1) and wait.get('exec_id') == run['synthetic_cid']
             and wait.get('conn') == 'A' and wait['_t'] >= t_sig, 'init wait status/identity missing')
        need(abs((wait['_t'] - wait_sent['_t']) * 1000 - wait['latency_ms']) <= 50, 'WaitProcess timestamp/latency inconsistent')
        need(wait.get('outcome') == 'ok', 'init wait failed')
        if t_release is not None and wait['_t'] >= t_release:
            init = 'exited-after-release'
        elif wait['_t'] - t_sig <= th['prompt_exit_s']:
            init = 'exited-promptly'
        else:
            init = 'exited-late'
    finals = [r for r in records if r['source'] == 'observe' and r.get('label') == 'final']
    need(len(finals) == 1, 'final observation missing')
    final = finals[0]
    correlate_rpcs(client, snapshots, {'WaitProcess': wait_sent['_t'], 'SignalProcess': t_sig})
    seen = set(end['seen_pids'])
    for sample in [r for r in client if r['kind'] == 'sample']:
        tasks = sample.get('tasks')
        need(isinstance(tasks, dict), 'sample task inventory missing')
        seen.update(int(k) for k in tasks)
        cg = sample.get('cgroup_state') or {}
        expected_dir = '/sys/fs/cgroup/' + ('system.slice/nerdctl-' + run['synthetic_cid'] + '.scope'
                                          if arm['cgroups'] == 'systemd' else 'finlab/' + run['synthetic_cid'])
        need(cg.get('path') == expected_dir, 'sample cgroup identity mismatch')
        freezer_state(sample, arm['cgroups'])
        need('error' not in cg and all(v == 'gone' or (isinstance(v, str) and re.fullmatch('[RSDZTtXxIP]:.*', v)) for v in tasks.values()),
             'sample observation unknown')
        if cg.get('cgroup') == 'present':
            need(pid_list(cg.get('all_procs')), 'sample descendants missing')
            seen.update(cg['all_procs'])
    if arm['reader']:
        seen.add(held[0]['pid'])
    need(seen <= set(end['seen_pids']), 'client-end omitted a retained PID')
    agent = only(records, 'identity', 'identity')
    after = max(end['_t'], sig['_t'], waits[0]['_t'] if waits else t_sig, t_release or t_sig)
    final_freezer, final_tasks, final_health, _ = terminal_snapshot(final, arm, run, agent, seen, after)
    if arm['reader']:
        stop_before = 'incomplete'
        snapshots = [r for r in client if r['kind'] == 'terminal-observation']
        need(len(snapshots) <= 1, 'duplicate terminal observation')
        if waits and snapshots:
            f, p, healthy, completed = terminal_snapshot(snapshots[0], arm, run, agent, seen,
                                                       max(t_sig, sig['_t'], waits[0]['_t']))
            if f == 'running' and p == 'none' and healthy and sig['outcome'] == 'ok' and completed <= snapshots[0]['_t'] < t_release:
                stop_before = 'completed'
    else:
        stop_before = 'not-applicable'
    final_check = final['check']['outcome']
    return {'held_request': held_obs, 'signal': signal, 'same_connection': same,
            'independent_connection': independent, 'init': init, 'stop_before_release': stop_before,
            'final_freezer': final_freezer, 'final_tasks': final_tasks,
            'signal_latency_ms': sig.get('latency_ms'), 'final_check': final_check,
            'final_health': final_health,
            'late_responses': len([r for r in client if r['kind'] == 'late-response'])}


def order_case(steps: dict[str, dict[str, Any]], case: str) -> str:
    def step(name: str) -> dict[str, Any]:
        need(name in steps, f'{case}: missing step {name}')
        return steps[name]

    def obs(name: str, state: str) -> bool:
        o = step(name).get('observation') or {}
        cg, unit = o.get('cgroup_state') or {}, o.get('unit') or {}
        want = {'running': ('0', '0', 'running'), 'freezing': ('1', '0', 'freezing')}[state]
        return (cg.get('freeze'), cg.get('frozen'), unit.get('FreezerState')) == want

    def failed_with(name: str, text: str) -> bool:
        s = step(name)
        return s.get('exit', 0) not in (0, None) and not s.get('bound_hit') and text in str(s.get('stderr', ''))

    def succeeded(name: str) -> bool:
        return step(name).get('exit') == 0 and not step(name).get('bound_hit')

    if case == 'thaw-cancels-accepted-freeze':
        need(step('freeze-accepted').get('observed') is True and obs('freeze-accepted', 'freezing'), f'{case}: acceptance not observed')
        ok = (failed_with('second-freeze-returned', 'still in progress') and succeeded('thaw-returned')
              and failed_with('freeze-returned', 'Freeze operation aborted') and obs('after-thaw', 'running'))
        return 'cancelled' if ok else 'other'
    if case == 'thaw-before-freeze':
        need(succeeded('thaw-returned') and obs('after-thaw', 'running'), f'{case}: thaw precondition')
        late = step('late-freeze-accepted').get('observed') is True and obs('late-freeze-accepted', 'freezing')
        restored = (succeeded('restore-thaw-returned') and failed_with('freeze-returned', 'Freeze operation aborted')
                    and obs('after-restore', 'running'))
        need(restored, f'{case}: restoration not observed')
        return 'late-freeze-applied' if late else 'fenced'
    need(failed_with('freeze-client-returned', 'timed out'), f'{case}: client did not time out by itself')
    pending = step('after-client-timeout').get('still_freezing') is True and obs('after-client-timeout', 'freezing')
    need(succeeded('restore-thaw-returned') and obs('after-restore', 'running'), f'{case}: restoration not observed')
    return 'freeze-still-pending' if pending else 'rolled-back'


def observe_order(records: list[dict[str, Any]], run: dict[str, Any]) -> dict[str, Any]:
    start = only(records, 'order', 'order-start')
    end = only(records, 'order', 'order-end')
    release = only(records, 'release', 'release-requested')
    order = [r for r in records if r['source'] == 'order']
    need(not [r for r in order if r['kind'] == 'case-setup-error'], 'order case setup failed')
    observed: dict[str, Any] = {}
    for case in ORDER_CASES:
        held = [r for r in order if r['kind'] == 'held-request' and r.get('case') == case]
        need(len(held) == 1, f'{case}: held request missing')
        h = held[0]
        unit = 'finlab-' + run['run'][:12] + '-' + case + '.scope'
        need(integer(h.get('pid'), 1, 2**31 - 1) and integer(h.get('unique'), 1, 2**64 - 1)
             and h.get('unit') == unit, f'{case}: held identity missing')
        received = [r for r in records if r['source'] == 'fused' and r['kind'] == 'read-received' and r.get('unique') == h['unique']]
        released = [r for r in records if r['source'] == 'fused' and r['kind'] == 'read-released' and r.get('unique') == h['unique']]
        need(len(received) == len(released) == 1 and received[0].get('pid') == h['pid']
             and integer(received[0].get('pid'), 1, 2**31 - 1) and integer(received[0].get('unique'), 1, 2**64 - 1)
             and integer(released[0].get('unique'), 1, 2**64 - 1)
             and received[0]['_t'] <= h['_t'] and released[0].get('by') == 'marker', f'{case}: independent FUSE correlation missing')
        steps: dict[str, dict[str, Any]] = {}
        for r in order:
            if r['kind'] == 'order-step' and r.get('case') == case:
                need(r.get('step') not in steps, f'{case}: duplicate step')
                need(r.get('unit') == unit and h['_t'] <= r['_t'] <= end['_t'], f'{case}: step identity/order')
                if 'exit' in r:
                    need(integer(r['exit'], -255, 255) and type(r.get('bound_hit')) is bool
                         and isinstance(r.get('stderr'), str), f'{case}: malformed command outcome')
                if 'observation' in r:
                    cg = r['observation'].get('cgroup_state') or {}
                    need(cg.get('path') == '/sys/fs/cgroup/system.slice/' + unit and 'error' not in cg,
                         f'{case}: observation identity/error')
                    freezer_state(r['observation'], 'systemd')
                    need(cg.get('cgroup') == 'present' and h['pid'] in cg.get('all_procs', []),
                         f'{case}: held PID outside observed scope task inventory')
                steps[r['step']] = r
        sequences = {
            'thaw-cancels-accepted-freeze': ['freeze-started', 'freeze-accepted', 'second-freeze-returned', 'thaw-returned', 'freeze-returned', 'after-thaw'],
            'thaw-before-freeze': ['thaw-returned', 'after-thaw', 'freeze-started', 'late-freeze-accepted', 'restore-thaw-returned', 'freeze-returned', 'after-restore'],
            'local-timeout-not-rollback': ['freeze-client-returned', 'after-client-timeout', 'restore-thaw-returned', 'after-restore'],
        }
        sequence = sequences[case]
        need(set(steps) == set(sequence), f'{case}: missing step or unknown step')
        need(all(steps[a]['_t'] <= steps[b]['_t'] for a, b in zip(sequence, sequence[1:])), f'{case}: command order')
        need(start['_t'] <= h['_t'] and released[0]['_t'] >= steps[sequence[-1]]['_t'], f'{case}: premature release')
        need(end['_t'] <= release['_t'] <= released[0]['_t'], f'{case}: release request correlation/order')
        observed[case] = order_case(steps, case)
    return {'cases': observed}


def classify(run_dir: Path) -> dict[str, Any]:
    verdict: dict[str, Any] = {'schema': VERDICT_SCHEMA, 'run_dir': str(run_dir), 'verdict': 'INCONCLUSIVE'}
    try:
        arm = load_json(run_dir / 'arm.json')
        verdict['arm'] = arm.get('name')
        validate_arm(arm)
        run = load_json(run_dir / 'run.json')
        verdict['run'] = run.get('run')
        need(hex_id(run.get('run'), 32), 'run generation missing')
        runner = check_runner(run, arm)
        verdict['teardown'] = runner['teardown']
        need(runner['teardown'] == 'clean', 'owned resources were not cleanly removed')
        records = load_records(run_dir / 'records.jsonl', run['run'])
        verdict['identity'] = check_identity(records, arm, run)
        observed = observe_order(records, run) if arm['kind'] == 'systemd-freezer-order' else observe_stop(records, arm, run)
        verdict['observed'] = observed
        mismatches = {k: {'expected': v, 'observed': observed.get(k)} for k, v in arm['expect'].items() if observed.get(k) != v}
        verdict['mismatches'] = mismatches
        verdict['verdict'] = 'FAIL' if mismatches else 'PASS'
        if arm['kind'] == 'agent-stop':
            liveness_ok = observed['same_connection'] == 'responsive' and observed['independent_connection'] == 'responsive'
            stopped = (observed['init'] != 'not-exited' and observed['final_freezer'] == 'running'
                       and observed['final_tasks'] == 'none' and observed['final_health'])
            signal_ok = observed['signal'] in {'prompt-ok', 'ok-after-release'}
            before = {'completed': True, 'incomplete': False}.get(observed['stop_before_release'])
            correct = liveness_ok and stopped and signal_ok
            verdict['claims'] = {'health_liveness': liveness_ok, 'lab_stop_completion': stopped, 'signal_ok': signal_ok,
                                 'stop_completed_before_release': before, 'correct_after_release': correct,
                                 'lifecycle_success': correct and before is not False}
            if not observed['final_health']:
                verdict['mismatches']['final_health'] = {'expected': True, 'observed': False}
                verdict['verdict'] = 'FAIL'
            if arm['purpose'] in {'candidate-qualification', 'positive-control'} and not verdict['claims']['lifecycle_success']:
                verdict['verdict'] = 'FAIL'
                verdict['mismatches']['lifecycle_success'] = {'expected': True, 'observed': False}
    except Inconclusive as reason:
        verdict['reason'] = str(reason)
    except (TypeError, KeyError, AttributeError, ValueError, IndexError, OSError) as error:
        # A wrong-typed or absent field is malformed evidence, never a crash
        # (an uncaught exception exits 1, which would read as FAIL).
        verdict['verdict'] = 'INCONCLUSIVE'
        verdict['reason'] = f'malformed evidence ({type(error).__name__})'
    return verdict


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print('usage: kata_lifecycle_verdict.py <run-dir>', file=sys.stderr)
        return 2
    verdict = classify(Path(argv[1]))
    print(json.dumps(verdict, indent=2, sort_keys=True))
    return EXIT[verdict['verdict']]


if __name__ == '__main__':
    sys.exit(main(sys.argv))
