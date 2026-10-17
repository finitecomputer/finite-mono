"""Resource evidence must not conflate limits, VM totals and workload usage."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finite_status_resources as resources


class ResourceEvidenceTests(unittest.TestCase):
    def test_dispatch_rejects_other_actions_and_non_object_requests(self):
        for request in ([], {"action": "repair"}, {"action": "recovery-progress"}):
            with self.assertRaises(ValueError):
                resources.collect(request)





    def subject(self):
        return {"runtime_id": "runtime_123abc", "source_host_id": "finite-lat-3",
                "source_machine_id": "finite-kata-abc", "project_id": "project_123"}

    def test_wrong_host_and_command_like_identifiers_fail_closed(self):
        row = self.subject()
        with self.assertRaises(ValueError):
            resources.validate_runtime(row, "finite-lat-4")
        row["source_machine_id"] = "--help"
        with self.assertRaises(ValueError):
            resources.validate_runtime(row, "finite-lat-3")

    def test_container_labels_and_durable_mount_must_match_subject(self):
        row = self.subject()
        labels = {"computer.finite.v2."+key:row[key] for key in
                  ("source_host_id", "source_machine_id", "project_id")}
        item = {"Config": {"Labels": labels}, "Mounts": [
            {"Destination": "/data", "Source": "/data/finite-saas-runner/kata/runtime_123abc"}]}
        self.assertEqual(str(resources.validate_inspect(row,item,"finite-lat-3")),
                         "/data/finite-saas-runner/kata/runtime_123abc")
        labels["computer.finite.v2.project_id"] = "project_other"
        with self.assertRaises(ValueError):
            resources.validate_inspect(row,item,"finite-lat-3")
        labels["computer.finite.v2.project_id"] = row["project_id"]
        item["Mounts"][0]["Source"] = "/etc/finite"
        with self.assertRaises(ValueError):
            resources.validate_inspect(row,item,"finite-lat-3")



class RetirementEvidenceTests(unittest.TestCase):
    def test_malformed_task_inventory_fails_closed(self):
        import subprocess
        from unittest import mock
        row = {'runtime_id': 'runtime_123', 'project_id': 'project_123',
               'source_host_id': 'finite-lat-3', 'source_machine_id': 'finite-kata-123'}
        results = [subprocess.CompletedProcess([], 0, '', ''),
                   subprocess.CompletedProcess([], 0, 'TASK PID STATUS\nmalformed unknown\n', '')]
        with mock.patch.object(resources.socket, 'gethostname', return_value='finite-lat-3'), \
             mock.patch.object(resources.status, 'run_read_only', side_effect=results):
            with self.assertRaisesRegex(ValueError, 'invalid retirement task'):
                resources.retirement_compute({'runtimes': [row]})

    def test_quiescence_detects_closed_descriptor_memory_maps_and_open_files(self):
        import tempfile
        from unittest import mock
        real_path = Path
        with tempfile.TemporaryDirectory() as directory:
            base = real_path(directory)
            root = base/'retired'; root.mkdir()
            proc = base/'proc'; proc.mkdir()
            process = proc/'123'; process.mkdir(); (process/'fd').mkdir()
            (process/'cmdline').write_bytes(b'worker\0')
            (process/'comm').write_text('worker\n')
            (process/'maps').write_text('00000000 rw-p 00000000 00:00 1 '+str(root/'state.db'))
            (proc/'self').mkdir(); (proc/'self'/'mountinfo').write_text('')
            def mapped_path(*args):
                if args and str(args[0]) == '/proc':
                    return real_path(proc, *args[1:])
                if args and str(args[0]) == '/proc/self/mountinfo':
                    return proc/'self'/'mountinfo'
                return real_path(*args)
            with mock.patch.object(resources, 'Path', side_effect=mapped_path):
                self.assertEqual(resources.retirement_quiescence(root)['durable_root_process_references'],
                                 [{'pid':123, 'comm':'worker'}])
                (process/'maps').write_text('')
                (process/'fd'/'4').symlink_to(root/'closed-file (deleted)')
                self.assertEqual(resources.retirement_quiescence(root)['durable_root_process_references'],
                                 [{'pid':123, 'comm':'worker'}])
                (process/'fd'/'4').unlink()
                self.assertEqual(resources.retirement_quiescence(root)['durable_root_process_references'], [])



if __name__ == "__main__":
    unittest.main()
