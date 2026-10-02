"""Bounded read-only Kata guest observations for one canonical lifecycle report.

Health is never Principal identity, writer absence, or repair authority. No signal,
exec, sandbox destruction, filesystem write, or caller-selected transport exists.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import select
import shlex
import socket
import struct
import time
import tomllib
from typing import Any

PROC = Path('/proc')
BUNDLES = Path('/run/containerd/io.containerd.runtime.v2.task/finite')
SANDBOXES = Path('/run/vc/sbs')
KATA_CONFIG = Path('/etc/kata-containers/configuration.toml')
ALLOWED_RPC = {('grpc.Health', 'Version'), ('grpc.Health', 'Check'),
               ('grpc.AgentService', 'StatsContainer')}


class ProbeRefusal(ValueError):
    """A fixed, non-secret diagnostic reason chosen by this module."""


def require(condition: bool, reason: str) -> None:
    if not condition:
        raise ProbeRefusal(reason)


def bounded(path: Path, limit: int) -> bytes:
    with path.open('rb') as source:
        data = source.read(limit + 1)
    require(len(data) <= limit, 'bounded metadata read exceeded')
    return data


def target_from_report(report: dict[str, Any], hostname: str) -> dict[str, str]:
    require(report.get('schema') == 'finite.lifecycle-probe.v1', 'unrecognized lifecycle report')
    runtime = report['runtime']
    require(isinstance(runtime, dict), 'invalid runtime identity')
    checks = [c for c in report['checks'] if c.get('name') == 'canonical_handle']
    require(len(checks) == 1 and checks[0].get('status') == 'pass', 'canonical handle not uniquely passed')
    evidence = checks[0]['evidence']
    cid = evidence['container_id']
    rid = runtime['agent_runtime_id']
    project = runtime['project_id']
    machine = runtime['source_machine_id']
    require(isinstance(cid, str) and bool(re.fullmatch('[a-f0-9]{64}', cid)), 'invalid full container ID')
    for value in (rid, project, machine):
        require(isinstance(value, str) and bool(re.fullmatch('[A-Za-z0-9_.-]+', value)), 'invalid durable identity')
    require(rid.startswith('runtime_') and project.startswith('project_'), 'unexpected identity kind')
    require(runtime.get('container_name') == machine == evidence.get('container_name'), 'machine name mismatch')
    root = '/data/finite-saas-runner/kata/' + rid
    require(evidence.get('state_root') == root, 'canonical root mismatch')
    require(hostname.startswith('finite-lat-'), 'unexpected local host')
    if 'source_host_id' in runtime:
        require(runtime['source_host_id'] == hostname, 'wrong local host')
    return {'container_id': cid, 'state_root': root, 'agent_runtime_id': rid,
            'project_id': project, 'source_machine_id': machine, 'source_host_id': hostname}


def safe_annotation_observations(spec: dict[str, Any]) -> dict[str, Any]:
    annotations = spec.get('annotations', {})
    if not isinstance(annotations, dict):
        return {'shape': 'invalid'}
    keys = sorted(k for k in annotations if isinstance(k, str)
                  and re.fullmatch(r'nerdctl/[A-Za-z0-9_.-]{1,96}', k))
    result: dict[str, Any] = {'nerdctl_keys': keys[:128], 'keys_truncated': len(keys) > 128}
    patterns = {
        'nerdctl/namespace': r'[A-Za-z0-9_.-]{0,64}',
        'nerdctl/name': r'finite-kata-[a-f0-9]{20}[a-z0-9_.-]{0,200}',
        'nerdctl/state-dir': r'/var/lib/nerdctl/[A-Za-z0-9_.-]{1,128}/containers/[A-Za-z0-9_.-]{1,64}/[a-f0-9]{64}',
    }
    for key, pattern in patterns.items():
        value = annotations.get(key)
        entry: dict[str, Any] = {'present': key in annotations}
        if isinstance(value, str) and re.fullmatch(pattern, value):
            entry['value'] = value
        elif key in annotations:
            entry['value_redacted'] = True
        result[key] = entry
    return result


def validate_spec(spec: dict[str, Any], target: dict[str, str]) -> None:
    mounts = [m for m in spec['mounts'] if m.get('destination') == '/data']
    require(len(mounts) == 1 and mounts[0].get('source') == target['state_root'], 'OCI durable root mismatch')
    require('ro' not in mounts[0].get('options', []), 'unexpected readonly data mount')
    annotations = spec.get('annotations', {})
    expected = {'computer.finite.v2.runtime': 'true',
                'computer.finite.v2.project_id': target['project_id'],
                'computer.finite.v2.source_machine_id': target['source_machine_id'],
                'computer.finite.v2.source_host_id': target['source_host_id']}
    # nerdctl v2.3.4 create.go propagates only nerdctl/ internal labels to
    # OCI annotations. The caller's fresh canonical_handle PASS established
    # the computer.finite labels on containerd metadata. Reject any conflicting
    # copy, and require the internal identity actually present in the bundle.
    require(all(k not in annotations or annotations[k] == v for k, v in expected.items()),
            'OCI identity labels mismatch')
    require(annotations.get('nerdctl/namespace') == 'finite', 'OCI internal namespace mismatch')
    # nerdctl v2.3.4 rename.go updates the name store and container Labels;
    # it does not rewrite the OCI spec or existing task bundle. Runner's
    # kata_upgrade_helper_name creates canonical + role + last ten request
    # characters. Treat this original name only as a recognized provenance
    # shape. Fresh canonical_handle metadata owns the current logical name;
    # exact bundle CID, namespace, state-dir and durable root bind this task.
    original_name = annotations.get('nerdctl/name')
    canonical = target['source_machine_id']
    recognized_candidate = (isinstance(original_name, str)
        and re.fullmatch(r'finite-kata-[a-f0-9]{20}', canonical) is not None
        and re.fullmatch(re.escape(canonical) + r'-(?:candidate|recovery)-[a-z0-9-]{1,10}', original_name) is not None)
    require(original_name == canonical or recognized_candidate, 'OCI original name not recognized')
    state_dir = annotations.get('nerdctl/state-dir', '')
    require(isinstance(state_dir, str) and re.fullmatch(
        r'/var/lib/nerdctl/[A-Za-z0-9_.-]+/containers/finite/' + target['container_id'], state_dir) is not None,
        'OCI internal container identity mismatch')
    for key, value in annotations.items():
        if 'sandbox-id' in key or 'sandbox.id' in key:
            require(value == target['container_id'], 'OCI sandbox identity mismatch')
    # Never expose process args or environment: either may hold credentials.
    require(isinstance(spec.get('process', {}).get('args'), list), 'OCI process metadata missing')


def validate_persist(persist: dict[str, Any], cid: str) -> tuple[int, int]:
    require(persist.get('SandboxContainer') == cid, 'persist sandbox mismatch')
    pid = persist.get('HypervisorState', {}).get('Pid')
    require(type(pid) is int and pid > 1, 'invalid persisted QEMU PID')
    url = persist.get('AgentState', {}).get('URL')
    require(isinstance(url, str), 'missing persisted agent endpoint')
    match = re.fullmatch(r'vsock://([0-9]+):1024', url)
    require(match is not None, 'unsupported persisted agent endpoint')
    vsock_cid = int(match[1])
    require(2 < vsock_cid < 2**32 - 1, 'invalid guest CID')
    return pid, vsock_cid


def allowed_executables(config: dict[str, Any]) -> set[str]:
    configured = Path(os.path.realpath(config['hypervisor']['qemu']['path']))
    require(str(configured).startswith('/nix/store/'), 'QEMU outside immutable closure')
    require(configured.is_file() and os.access(configured, os.X_OK), 'QEMU executable unavailable')
    allowed = {str(configured)}
    inner = configured.parent / ('.' + configured.name + '-wrapped')
    if inner.exists():
        raw = bounded(configured, 128 * 1024)
        if raw.startswith(b'#!'):
            lines = [line for line in raw.decode().splitlines()
                     if re.match(r'^\s*exec\s', line) and str(inner) in line]
            require(len(lines) == 1, 'ambiguous wrapper exec')
            require(shlex.split(lines[0]) in (['exec', '-a', '$0', str(inner), '$@'],
                                              ['exec', str(inner), '$@']), 'unsupported wrapper exec')
        else:
            require(raw.startswith(b'\x7fELF'), 'unsupported wrapper format')
            require(raw.split(b'\0').count(str(inner).encode()) == 1, 'ambiguous binary wrapper target')
            blocks = [part.decode('latin1') for part in raw.split(b'\0') if b'makeCWrapper ' in part]
            require(len(blocks) == 1, 'ambiguous wrapper build command')
            block = blocks[0]
            command = []
            for line in block[block.index('makeCWrapper '):].splitlines():
                command.append(line)
                if not line.rstrip().endswith('\\'):
                    break
            tokens = shlex.split('\n'.join(command).replace('\\\n', ' '))
            require(tokens[:2] == ['makeCWrapper', str(inner)] and tokens.count('--inherit-argv0') == 1,
                    'wrapper build target mismatch')
        require(inner.is_file() and not inner.is_symlink() and os.access(inner, os.X_OK), 'wrapped executable invalid')
        require(Path(os.path.realpath(inner)).parent == configured.parent, 'wrapped executable escaped closure')
        allowed.add(str(inner))
    return allowed


def process_identity(pid: int) -> tuple[str, str, tuple[str, ...]]:
    path = PROC / str(pid)
    def start() -> str:
        fields = bounded(path / 'stat', 4096).rsplit(b')', 1)[1].split()
        require(fields[0] != b'Z' and fields[19].isdigit(), 'QEMU not live')
        return fields[19].decode('ascii')
    before = start()
    executable = os.readlink(path / 'exe')
    argv = tuple(bounded(path / 'cmdline', 65536).decode().rstrip('\0').split('\0'))
    require(start() == before, 'process identity changed')
    return before, executable, argv


def device_cids(argv: tuple[str, ...]) -> list[int]:
    result = []
    for index, word in enumerate(argv[:-1]):
        if word == '-device' and argv[index + 1].startswith('vhost-vsock-pci,'):
            values = re.findall(r'(?:^|,)guest-cid=([0-9]+)(?=,|$)', argv[index + 1])
            require(len(values) == 1, 'ambiguous vsock device')
            result.append(int(values[0]))
    return result


def inventory(deadline: float) -> list[dict[str, Any]]:
    paths = [p for p in PROC.iterdir() if p.name.isdecimal()]
    require(len(paths) <= 32768, 'process inventory exceeded')
    result = []
    for path in paths:
        require(time.monotonic() < deadline, 'inventory deadline exceeded')
        try:
            exe = os.readlink(path / 'exe')
            if not re.fullmatch(r'\.?qemu-system-[A-Za-z0-9_-]+', Path(exe).name):
                continue
            identity = process_identity(int(path.name))
            result.append({'pid': int(path.name), 'identity': identity, 'cids': device_cids(identity[2])})
        except (FileNotFoundError, ProcessLookupError):
            continue
    return result


def select_qemu(processes: list[dict[str, Any]], target: dict[str, str],
                pid: int, vsock_cid: int, allowed: set[str]) -> dict[str, Any]:
    owners = [p for p in processes if vsock_cid in p['cids']]
    require(len(owners) == 1, 'guest CID not uniquely owned')
    process = owners[0]
    require(process['pid'] == pid and process['cids'] == [vsock_cid], 'persist PID or CID mismatch')
    identity = process['identity']
    require(identity[1] in allowed, 'live QEMU executable mismatch')
    exact_path = '/run/vc/vm/' + target['container_id'] + '/'
    require(any(exact_path in arg for arg in identity[2]), 'QEMU full container path mismatch')
    matches = [p for p in processes if any(exact_path in arg for arg in p['identity'][2])]
    require(len(matches) == 1 and matches[0]['pid'] == pid, 'container QEMU ambiguous')
    return process


def bind_target(report: dict[str, Any], deadline: float, observations: dict[str, Any] | None = None) -> dict[str, Any]:
    target = target_from_report(report, socket.gethostname().split('.', 1)[0])
    cid = target['container_id']
    root = Path(target['state_root'])
    require(root.is_dir() and root.resolve() == root, 'durable root missing or redirected')
    spec_bytes = bounded(BUNDLES / cid / 'config.json', 2 * 1024 * 1024)
    persist_bytes = bounded(SANDBOXES / cid / 'persist.json', 2 * 1024 * 1024)
    spec = json.loads(spec_bytes)
    if observations is not None:
        observations['oci_annotations'] = safe_annotation_observations(spec)
    validate_spec(spec, target)
    pid, vsock_cid = validate_persist(json.loads(persist_bytes), cid)
    config = tomllib.loads(bounded(KATA_CONFIG, 256 * 1024).decode())
    process = select_qemu(inventory(deadline), target, pid, vsock_cid, allowed_executables(config))
    return {'target': target, 'pid': pid, 'vsock_cid': vsock_cid, 'identity': process['identity'],
            'metadata_digest': hashlib.sha256(spec_bytes + b'\0' + persist_bytes).digest()}


def varint(value: int) -> bytes:
    require(0 <= value < 2**64, 'invalid unsigned protobuf value')
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def field(number: int, value: str | bytes | int) -> bytes:
    if isinstance(value, int):
        return varint(number << 3) + varint(value)
    if isinstance(value, str):
        value = value.encode()
    return varint((number << 3) | 2) + varint(len(value)) + value


def fields(data: bytes, *, allow_repeated: bool = False,
           repeated_fields: frozenset[int] = frozenset()) -> dict[int, bytes | int]:
    position = 0
    result = {}
    def take() -> int:
        nonlocal position
        value = 0
        for index in range(10):
            require(position < len(data), 'truncated protobuf')
            byte = data[position]
            position += 1
            require(index < 9 or byte <= 1, 'overflowing protobuf varint')
            value |= (byte & 127) << (index * 7)
            if byte < 128:
                return value
        raise ValueError('overflowing protobuf varint')
    while position < len(data):
        tag = take()
        number, wire = tag >> 3, tag & 7
        require(0 < number < 2**29 and (allow_repeated or number in repeated_fields or number not in result), 'invalid or duplicate protobuf field')
        if wire == 0:
            value = take()
        elif wire == 2:
            size = take()
            require(size <= len(data) - position, 'truncated protobuf field')
            value = data[position:position + size]
            position += size
        else:
            raise ValueError('unsupported protobuf wire type')
        result[number] = value
    return result


def read_exact(connection: socket.socket, size: int, deadline: float) -> bytes:
    result = bytearray()
    while len(result) < size:
        remaining = deadline - time.monotonic()
        require(remaining > 0, 'RPC deadline exceeded')
        connection.settimeout(remaining)
        block = connection.recv(size - len(result))
        require(bool(block), 'RPC stream ended')
        result.extend(block)
    return bytes(result)


def read_only_rpc(connection: socket.socket, stream: int, service: str,
                  method: str, payload: bytes, deadline: float) -> bytes:
    require((service, method) in ALLOWED_RPC, 'non-observational RPC refused')
    if service == 'grpc.Health':
        require(payload == b'', 'unexpected health payload')
    else:
        parsed = fields(payload)
        require(set(parsed) == {1} and isinstance(parsed[1], bytes)
                and re.fullmatch(b'[a-f0-9]{64}', parsed[1]) is not None, 'invalid stats target')
    rpc_deadline = min(deadline, time.monotonic() + 4)
    request = field(1, service) + field(2, method) + field(3, payload) + field(4, 3_000_000_000)
    connection.settimeout(max(0.001, rpc_deadline - time.monotonic()))
    connection.sendall(struct.pack('>IIBB', len(request), stream, 1, 0) + request)
    size, response_stream, kind, flags = struct.unpack('>IIBB', read_exact(connection, 10, rpc_deadline))
    require(size <= 1024 * 1024 and response_stream == stream and kind == 2 and flags == 0, 'invalid RPC frame')
    response = fields(read_exact(connection, size, rpc_deadline))
    require(set(response) <= {1, 2}, 'unexpected RPC response fields')
    status_bytes = response.get(1, b'')
    require(isinstance(status_bytes, bytes), 'invalid RPC status')
    status = fields(status_bytes)
    require(status.get(1, 0) == 0, 'guest RPC returned nonzero status')
    payload_result = response.get(2, b'')
    require(isinstance(payload_result, bytes), 'invalid RPC payload')
    return payload_result


def stats_pids_current(payload: bytes) -> int | None:
    # Kata 3.29 agent.proto: StatsContainerResponse.cgroup_stats=1,
    # CgroupStats.pids_stats=3, PidsStats.current=1 (uint64, proto3 default 0).
    # Absence of either containing message is unknown, never inferred zero.
    outer = fields(payload, repeated_fields=frozenset({2}))
    require(set(outer) <= {1, 2} and all(isinstance(v, bytes) for v in outer.values()), 'invalid stats response')
    if 1 not in outer:
        return None
    cgroup = fields(outer[1], repeated_fields=frozenset({5}))
    require(set(cgroup) <= {1, 2, 3, 4, 5} and all(isinstance(v, bytes) for v in cgroup.values()), 'invalid cgroup stats')
    if 3 not in cgroup:
        return None
    pids = fields(cgroup[3])
    require(set(pids) <= {1, 2} and all(type(v) is int for v in pids.values()), 'invalid PID stats')
    return pids.get(1, 0)


def collect_guest_agent_probe(report: dict[str, Any]) -> dict[str, Any]:
    result: dict[str, Any] = {'status': 'unknown', 'repair_authority': False,
        'principal_identity_established': False, 'writer_absence_proven': False,
        'coverage': 'point_in_time', 'observations': {}, 'errors': []}
    phase = 'binding'
    pidfd = None
    deadline = time.monotonic() + 20
    try:
        binding = bind_target(report, deadline, result['observations'])
        pidfd = os.pidfd_open(binding['pid'])
        def revalidate() -> None:
            require(not select.select([pidfd], [], [], 0)[0], 'QEMU exited')
            require(process_identity(binding['pid']) == binding['identity'], 'QEMU identity changed')
            require(bind_target(report, deadline, result['observations']) == binding, 'target binding changed')
        revalidate()
        phase = 'connecting'
        with socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM) as connection:
            connection.settimeout(min(3, max(.001, deadline - time.monotonic())))
            connection.connect((binding['vsock_cid'], 1024))
            require(connection.getpeername() == (binding['vsock_cid'], 1024), 'vsock peer mismatch')
            revalidate()
            phase = 'observing'
            version = fields(read_only_rpc(connection, 1, 'grpc.Health', 'Version', b'', deadline))
            require(set(version) <= {1, 2}, 'unexpected version fields')
            agent_version = version.get(2, b'')
            require(isinstance(agent_version, bytes) and agent_version == b'3.29.0', 'unexpected guest version')
            health = fields(read_only_rpc(connection, 3, 'grpc.Health', 'Check', b'', deadline))
            require(health == {1: 1}, 'guest not serving')
            stats = read_only_rpc(connection, 5, 'grpc.AgentService', 'StatsContainer',
                                  field(1, binding['target']['container_id']), deadline)
            pids_current = stats_pids_current(stats)
            phase = 'revalidating'
            revalidate()
        result.update(status='observed', container_id=binding['target']['container_id'],
                      qemu_pid=binding['pid'], qemu_starttime_ticks=binding['identity'][0],
                      vsock_cid=binding['vsock_cid'])
        result['observations'].update({'agent_version': '3.29.0', 'health_serving': True,
                                  'exact_container_stats_rpc_succeeded': True,
                                  'container_cgroup_pids_current': pids_current,
                                  'pids_observation_is_writer_absence_proof': False,
                                  'stats_response_bytes': len(stats)})
    except (OSError, ValueError, KeyError, TypeError, IndexError, AttributeError, UnicodeError) as error:
        # Never propagate raw metadata, argv, environment, agent status text or
        # exception strings into status output.
        entry = {'phase': phase, 'kind': type(error).__name__}
        if isinstance(error, ProbeRefusal):
            entry['reason'] = str(error)
        result['errors'].append(entry)
    finally:
        if pidfd is not None:
            os.close(pidfd)
    return result
