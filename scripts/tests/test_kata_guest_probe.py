from __future__ import annotations

import copy
import json
import socket
import struct
import time
import unittest
from unittest import mock

from scripts import kata_guest_probe as probe

CID = 'a' * 64
HOST = 'finite-lat-5'
RID = 'runtime_fixture'
ROOT = '/data/finite-saas-runner/kata/' + RID


def report():
    return {'schema': 'finite.lifecycle-probe.v1', 'runtime': {
        'project_id': 'project_fixture', 'agent_runtime_id': RID,
        'source_machine_id': 'finite-kata-fixture', 'container_name': 'finite-kata-fixture'},
        'checks': [{'name': 'canonical_handle', 'status': 'pass', 'evidence': {
            'container_id': CID, 'container_name': 'finite-kata-fixture', 'state_root': ROOT}}]}


def spec():
    return {'mounts': [{'destination': '/data', 'source': ROOT, 'options': ['rw', 'rbind']}],
            'process': {'args': ['/opt/agent-entrypoint.sh'], 'env': ['SECRET=must-not-be-reported']},
            'annotations': {'nerdctl/namespace': 'finite', 'nerdctl/name': 'finite-kata-fixture',
                            'nerdctl/state-dir': '/var/lib/nerdctl/fixture/containers/finite/' + CID,
                            'computer.finite.v2.runtime': 'true',
                            'computer.finite.v2.project_id': 'project_fixture',
                            'computer.finite.v2.source_machine_id': 'finite-kata-fixture',
                            'computer.finite.v2.source_host_id': HOST}}


def process(pid=42, cid=17):
    return {'pid': pid, 'identity': ('123', '/nix/store/qemu/bin/qemu-system-x86_64',
            ('qemu', '-qmp', 'unix:/run/vc/vm/' + CID + '/qmp.sock',
             '-device', 'vhost-vsock-pci,guest-cid=' + str(cid))), 'cids': [cid]}


class GuestProbeTests(unittest.TestCase):
    def test_target_requires_unique_pass_and_exact_root(self):
        for mutate in (
            lambda x: x['checks'].append(copy.deepcopy(x['checks'][0])),
            lambda x: x['checks'][0].update(status='fail'),
            lambda x: x['checks'][0]['evidence'].update(state_root='/data/other'),
            lambda x: x['runtime'].update(source_machine_id='wrong'),
        ):
            value = report(); mutate(value)
            with self.assertRaises(ValueError): probe.target_from_report(value, HOST)

    def test_spec_rejects_root_and_label_mismatches(self):
        target = probe.target_from_report(report(), HOST)
        probe.validate_spec(spec(), target)
        for key in ('computer.finite.v2.project_id', 'computer.finite.v2.source_machine_id',
                    'computer.finite.v2.source_host_id', 'computer.finite.v2.runtime'):
            value = spec(); value['annotations'][key] = 'wrong'
            with self.assertRaises(ValueError): probe.validate_spec(value, target)
        value = spec(); value['mounts'][0]['source'] += '-other'
        with self.assertRaises(ValueError): probe.validate_spec(value, target)

    def test_vendor_internal_annotations_without_custom_label_copies(self):
        value = spec()
        value['annotations'] = {k: v for k, v in value['annotations'].items() if k.startswith('nerdctl/')}
        target = probe.target_from_report(report(), HOST)
        probe.validate_spec(value, target)
        for key in ('nerdctl/name', 'nerdctl/namespace', 'nerdctl/state-dir'):
            bad = copy.deepcopy(value); bad['annotations'][key] = 'wrong'
            with self.assertRaises(ValueError): probe.validate_spec(bad, target)

    def test_persist_rejects_wrong_container_and_endpoint(self):
        good = {'SandboxContainer': CID, 'HypervisorState': {'Pid': 42},
                'AgentState': {'URL': 'vsock://17:1024'}}
        self.assertEqual(probe.validate_persist(good, CID), (42, 17))
        for key, value in [('SandboxContainer', 'b' * 64),
                           ('HypervisorState', {'Pid': 0}),
                           ('AgentState', {'URL': 'vsock://17:9999'})]:
            bad = copy.deepcopy(good); bad[key] = value
            with self.assertRaises(ValueError): probe.validate_persist(bad, CID)

    def test_renamed_candidate_is_provenance_not_logical_identity(self):
        target = probe.target_from_report(report(), HOST)
        target['source_machine_id'] = 'finite-kata-' + '1' * 20
        value = spec()
        value['annotations']['computer.finite.v2.source_machine_id'] = target['source_machine_id']
        for role in ('candidate', 'recovery'):
            value['annotations']['nerdctl/name'] = target['source_machine_id'] + '-' + role + '-123456abcd'
            probe.validate_spec(value, target)
            self.assertEqual(probe.safe_annotation_observations(value)['nerdctl/name']['value'],
                             value['annotations']['nerdctl/name'])
        for name in ('finite-kata-' + '2' * 20 + '-candidate-123456abcd',
                     target['source_machine_id'] + '-rollback-123456abcd',
                     target['source_machine_id'] + '-candidate-123456abcde', 'SECRET'):
            value['annotations']['nerdctl/name'] = name
            with self.assertRaises(ValueError): probe.validate_spec(value, target)
        self.assertTrue(probe.safe_annotation_observations(value)['nerdctl/name']['value_redacted'])

    def test_pid_cid_collision_or_target_swap_refuses(self):
        target = probe.target_from_report(report(), HOST)
        allowed = {process()['identity'][1]}
        probe.select_qemu([process()], target, 42, 17, allowed)
        for items, pid, cid in [([process()], 43, 17), ([process()], 42, 18),
                                 ([process(), process(pid=43)], 42, 17)]:
            with self.assertRaises(ValueError): probe.select_qemu(items, target, pid, cid, allowed)
        swapped = process(); identity = list(swapped['identity'])
        identity[2] = tuple(word.replace(CID, 'b' * 64) for word in identity[2])
        swapped['identity'] = tuple(identity)
        with self.assertRaises(ValueError): probe.select_qemu([swapped], target, 42, 17, allowed)

    def test_signal_and_destroy_are_unrepresentable_through_rpc(self):
        connection = mock.Mock()
        for method in ('SignalProcess', 'DestroySandbox', 'ExecProcess', 'WaitProcess'):
            with self.assertRaises(ValueError):
                probe.read_only_rpc(connection, 1, 'grpc.AgentService', method, b'', time.monotonic()+3)
        connection.sendall.assert_not_called()

    def test_wire_rejects_malformed_and_error_responses(self):
        for malformed in (b'\x80', b'\x00', b'\x0a\x08a', b'\x08\x01\x08\x02', b'\xff'*10):
            with self.assertRaises(ValueError): probe.fields(malformed)
        status = probe.field(1, 13) + probe.field(2, 'SECRET must never leave collector')
        body = probe.field(1, status)
        packets = [struct.pack('>IIBB', len(body), 1, 2, 0), body]
        connection = mock.Mock(); connection.recv.side_effect = packets
        with self.assertRaisesRegex(ValueError, '^guest RPC returned nonzero status$'):
            probe.read_only_rpc(connection, 1, 'grpc.Health', 'Version', b'', time.monotonic()+3)

    def test_fragmented_valid_response(self):
        expected = probe.field(2, '3.29.0')
        body = probe.field(1, b'') + probe.field(2, expected)
        wire = struct.pack('>IIBB', len(body), 1, 2, 0) + body
        connection = mock.Mock(); connection.recv.side_effect = [bytes([byte]) for byte in wire]
        actual = probe.read_only_rpc(connection, 1, 'grpc.Health', 'Version', b'', time.monotonic()+3)
        self.assertEqual(actual, expected)

    def test_stats_pids_presence_defaults_and_malformed_shapes(self):
        f = probe.field
        self.assertIsNone(probe.stats_pids_current(b''))
        self.assertIsNone(probe.stats_pids_current(f(1, b'')))
        self.assertEqual(probe.stats_pids_current(f(1, f(3, b''))), 0)
        self.assertEqual(probe.stats_pids_current(f(1, f(3, f(1, 17)))), 17)
        for malformed in (f(1, 0), f(1, f(3, 0)), f(1, f(3, f(1, b''))),
                          f(1, f(3, b'') + f(3, b'')), f(1, b'') + f(1, b'')):
            with self.assertRaises(ValueError): probe.stats_pids_current(malformed)

    def test_collector_never_connects_after_binding_swap_or_leaks_errors(self):
        binding = {'target': {'container_id': CID}, 'pid': 42, 'identity': ('123', 'qemu', ()), 'vsock_cid': 17}
        changed = dict(binding, vsock_cid=18)
        with mock.patch.object(probe, 'bind_target', side_effect=[binding, changed]), \
             mock.patch.object(probe.os, 'pidfd_open', return_value=90, create=True), \
             mock.patch.object(probe.os, 'close'), \
             mock.patch.object(probe.select, 'select', return_value=([], [], [])), \
             mock.patch.object(probe, 'process_identity', return_value=binding['identity']), \
             mock.patch.object(probe.socket, 'socket') as connect:
            result = probe.collect_guest_agent_probe(report())
        connect.assert_not_called()
        self.assertEqual(result['status'], 'unknown')
        self.assertFalse(result['repair_authority'])
        self.assertFalse(result['principal_identity_established'])
        with mock.patch.object(probe, 'bind_target', side_effect=ValueError('SECRET')):
            result = probe.collect_guest_agent_probe(report())
        self.assertNotIn('SECRET', json.dumps(result))


if __name__ == '__main__':
    unittest.main()
