import json
import os
import sqlite3
import tempfile
import socket
import threading
import unittest
import sys
from contextlib import closing
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from scripts import finite_status_runtime_incident as probe


class RuntimeIncidentTests(unittest.TestCase):
    def test_retired_customer_recovery_selectors_fail_before_reading_state(self):
        for flag in ('recovery_progress', 'history_preservation', 'guest_output_recovery',
                     'post_recovery_history_preservation'):
            with patch.object(probe, 'retained_metadata') as read:
                with self.assertRaisesRegex(ValueError, 'selectors are retired'):
                    probe.collect({flag: True})
                read.assert_not_called()


    def test_binding_only_probe_keeps_metadata_privacy_and_skips_deep_inspection(self):
        import finite_status_resources, finite_status
        with tempfile.TemporaryDirectory() as directory:
            memory=Path(directory)/'meminfo';memory.write_text('MemAvailable: 500 kB\n')
            real_path=Path
            def paths(*args):
                return memory if args==('/proc/meminfo',) else real_path(*args)
            runtime={'runtime_id':'runtime_synthetic','container_id':'a'*64,
                'container_present':True,'data_root_is_symlink':False,
                'durable_root':'/data/finite-saas-runner/kata/runtime_synthetic',
                'source_machine_id':'synthetic','sandbox_processes':[]}
            inspected=json.dumps([{'Id':'a'*64,'Config':{'Image':'example/runtime@sha256:'+'b'*64}}])
            def command(args,**kwargs):
                self.assertIn(args[0],['nerdctl','nice'])
                return SimpleNamespace(returncode=0,stdout=inspected if args[0]=='nerdctl' else '0 synthetic')
            with patch.object(finite_status_resources,'retirement_compute',return_value={'runtimes':[runtime]}), \
                 patch.object(probe,'retained_metadata',return_value={}) as retained, \
                 patch.object(probe,'container_snapshot_metadata',return_value={'available':True}), \
                 patch.object(probe,'hypervisor_metadata') as deep,patch.object(probe,'Path',paths), \
                 patch.object(probe.shutil,'disk_usage',return_value=SimpleNamespace(free=1,total=2)), \
                 patch.object(finite_status,'run_read_only',side_effect=command):
                report=probe.collect({'metadata_only':True,'recovery':True,'recovery_bindings_only':True})
            retained.assert_called_once_with(real_path(runtime['durable_root']),include_messages=False)
            deep.assert_not_called()
            self.assertNotIn('host_journal',report['runtimes'][0])
            self.assertNotIn('console',report['runtimes'][0])

    def test_blocked_thread_probe_exports_stdio_binding_without_write_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);proc=root/'101';(proc/'task/101').mkdir(parents=True)
            (proc/'task/102').mkdir();(proc/'fd').mkdir()
            fields=['S','1']+['0']*17+['77']
            (proc/'stat').write_text('101 (python3) '+' '.join(fields))
            (proc/'cmdline').write_bytes(b'hermes\0gateway\0private-argv-canary\0')
            (proc/'syscall').write_text('202 0x1234')
            for tid,wait in [(101,'__futex_wait'),(102,'anon_pipe_write')]:
                thread=proc/'task'/str(tid)
                (thread/'stat').write_text(str(tid)+' (python3) '+' '.join(fields))
                (thread/'wchan').write_text(wait)
                (thread/'syscall').write_text('1 0x2 0xprivate-payload-canary 65536')
            for fd in [1,2]:os.symlink('pipe:['+str(fd)+']',proc/'fd'/str(fd))
            rows=[]
            code=probe.GUEST_PROCESSES.replace("Path('/proc')",'Path('+repr(str(root))+')')
            with patch('builtins.print',side_effect=lambda value:rows.append(json.loads(value))):
                exec(code,{})
            thread=next(t for t in rows[0]['processes'][0]['threads'] if t['tid']==102)
            self.assertEqual(thread['blocked_stdio_fd'],2)
            self.assertEqual(thread['start_ticks'],77)
            self.assertNotIn('canary',json.dumps(rows))

    def test_provider_prefill_rejection_is_retained_beyond_recent_tool_activity(self):
        lines = [
            '2026-10-05 19:22:42 ERROR Error code: 400 - This model does not support assistant message prefill. The conversation must end with a user message. token=private-canary',
            '2026-10-03 03:45:43 ERROR HTTP 402: insufficient credits',
        ] + ['2026-10-05 19:40:00 calling tool private-content-canary'] * 120
        report = probe.classify(lines)
        self.assertEqual(report['counts']['provider_invalid_prefill'], 1)
        self.assertEqual(report['counts']['provider_bad_request'], 1)
        self.assertEqual(report['counts']['provider_credit_error'], 1)
        self.assertEqual(len(report['provider_events']), 2)
        self.assertEqual(report['provider_events'][0]['at'], '2026-10-05 19:22:42')
        self.assertNotIn('canary', json.dumps(report))

    def test_provider_owner_classification_never_exports_unknown_argv(self):
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory);proc=base/'proc/101';proc.mkdir(parents=True)
            (proc/'stat').write_text('101 (sleep) '+' '.join(['S','1']+['0']*17+['77']))
            (proc/'wchan').write_text('__do_sys_pause');(proc/'fd').mkdir()
            for name in ['mnt','net','pid']:
                host=base/'proc/1/ns'/name;host.parent.mkdir(parents=True,exist_ok=True);host.touch()
                target=proc/'ns'/name;target.parent.mkdir(exist_ok=True);os.link(host,target)
            bundle=base/'provider';bundle.mkdir();real_path=Path
            def mapped(*args):
                value=real_path(*args)
                return base/str(value).lstrip('/') if str(value).startswith('/proc') else value
            exe='/nix/store/'+'a'*32+'-coreutils-9.7/bin/sleep'
            for args,expected in [([exe,'infinity'],True),(['sleep','infinity'],True),([exe,'private-argv-canary'],False)]:
                (proc/'cmdline').write_bytes(b'\0'.join(a.encode() for a in args)+b'\0')
                with patch.object(probe,'Path',mapped),patch.object(probe.os,'readlink',side_effect=lambda p:exe if p.name=='exe' else str(bundle)):
                    result=probe.provider_owner_metadata(101,bundle)
                self.assertEqual(result['verified_coreutils_sleep_infinity'],expected)
                self.assertEqual(result['start_ticks'],77)
                self.assertNotIn('private-argv-canary',json.dumps(result))
                self.assertNotIn(exe,json.dumps(result))
            multi=exe.removesuffix('/sleep')+'/coreutils'
            (proc/'cmdline').write_bytes(b'sleep\0infinity\0')
            with patch.object(probe,'Path',mapped),patch.object(probe.os,'readlink',side_effect=lambda p:multi if p.name=='exe' else str(bundle)):
                self.assertTrue(probe.provider_owner_metadata(101,bundle)['verified_coreutils_sleep_infinity'])



    def test_logging_uri_projection_excludes_query_values_and_unknown_labels(self):
        item={'ID':'a'*64,'SnapshotKey':'snapshot','Snapshotter':'overlayfs','Spec':{},
            'Labels':{'nerdctl/log-uri':'binary:///nix/store/package/bin/.nerdctl-wrapped?token=private-canary',
                'other-private-canary':'private-canary'}}
        answers=[SimpleNamespace(returncode=0,stdout=json.dumps(item)),SimpleNamespace(returncode=1,stdout='')]
        status=SimpleNamespace(run_read_only=lambda *args,**kwargs:answers.pop(0))
        with patch.dict('sys.modules',{'finite_status':status}):report=probe.container_snapshot_metadata('a'*64)
        self.assertEqual(report['logging']['binary'],'.nerdctl-wrapped')
        self.assertNotIn('private-canary',json.dumps(report))

    def test_logger_metadata_binds_shim_start_and_does_not_export_environment_or_fd_content(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);proc=root/'101';proc.mkdir()
            fields=['S','1']+['0']*18;fields[19]='123'
            (proc/'stat').write_text('101 (containerd-shim) '+' '.join(fields))
            (proc/'comm').write_text('containerd-shim')
            (proc/'cmdline').write_bytes(b'containerd-shim\0private-argv-canary\0')
            (proc/'environ').write_bytes(b'CONTAINER_ID='+b'a'*64+b'\0CONTAINER_NAMESPACE=finite\0TOKEN=private-canary\0')
            (proc/'cwd').symlink_to('/run/containerd/io.containerd.runtime.v2.task/finite/'+'a'*64)
            for sub in ['fd','fdinfo','task/101']:(proc/sub).mkdir(parents=True)
            secret=root/'private-canary';secret.write_text('private-fd-content-canary')
            (proc/'fd/1').symlink_to(secret);(proc/'fdinfo/1').write_text('flags:\t01\n')
            (proc/'task/101/wchan').write_text('futex_do_wait')
            real_path=Path
            def mapped(*args):return root if args==('/proc',) else real_path(*args)
            runtime={'container_id':'a'*64,'sandbox_processes':[{'pid':101,'comm':'containerd-shim','pid_start_ticks':123}]}
            with patch.object(probe,'Path',mapped):report=probe.provider_logging_processes(runtime)
            self.assertTrue(report[0]['logger_binding']['container_matches'])
            self.assertNotIn('private',json.dumps(report))
            runtime['sandbox_processes'][0]['pid_start_ticks']=124
            with patch.object(probe,'Path',mapped),self.assertRaisesRegex(ValueError,'start identity changed'):
                probe.provider_logging_processes(runtime)

    def test_wal_projection_never_changes_original_db_wal_or_shared_memory(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);home=root/'agent/hermes-home';home.mkdir(parents=True)
            database=home/'state.db'
            with closing(sqlite3.connect(database)) as writer:
                writer.execute('PRAGMA journal_mode=WAL')
                writer.executescript('CREATE TABLE sessions(id TEXT,source TEXT); '
                    'CREATE TABLE messages(session_id TEXT,role TEXT,timestamp REAL,content TEXT);')
                writer.execute("INSERT INTO sessions VALUES ('s','simplex')")
                writer.execute("INSERT INTO messages VALUES ('s','user',123,'private-wal-canary')")
                writer.commit()
                files=[database,Path(str(database)+'-wal'),Path(str(database)+'-shm')]
                before={str(p):(p.read_bytes(),p.stat().st_mtime_ns) for p in files}
                report=probe.retained_metadata(root)
                self.assertEqual(report['latest_messages'],[{'source':'simplex','role':'user','at_epoch':123}])
                self.assertNotIn('private-wal-canary',json.dumps(report))
                self.assertEqual(before,{str(p):(p.read_bytes(),p.stat().st_mtime_ns) for p in files})

    def test_host_metadata_classifies_freeze_evidence_without_exporting_content(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            home = root / "agent/hermes-home"
            (home / "logs").mkdir(parents=True)
            (home / "logs/agent.log").write_text(
                "2026-10-04 01:00:00 ERROR APIConnectionError connection error token=private-canary\n"
                "2026-10-04 01:00:01 HTTP Request: POST https://private-canary/chat/completions\n")
            (home / "state").mkdir()
            (home / "state/gateway.heartbeat").write_text(json.dumps({"pid": 123,
                "updated_at": "2026-10-04T03:52:55+00:00", "mem": {"rss_kib": 800,
                "mem_available_kib": 12000, "private": "private-canary"}, "private": "private-canary"}))
            (root / "agent/agentd").mkdir()
            (root / "agent/agentd/status.json").write_text(json.dumps({"processes": {"processes": {
                "hermes": {"state": "running", "pid": 123, "restart_count": 1,
                           "last_exit": "private-canary"}}}}))
            database = home / "state.db"
            with sqlite3.connect(database) as connection:
                connection.executescript("CREATE TABLE sessions(id TEXT,source TEXT); "
                    "CREATE TABLE messages(session_id TEXT,role TEXT,timestamp REAL,content TEXT,tool_name TEXT);")
                connection.execute("INSERT INTO sessions VALUES ('s','finitechat')")
                connection.execute("INSERT INTO messages VALUES ('s','user',123,'private-canary',NULL)")
                connection.execute("INSERT INTO messages VALUES ('s','tool',124,'private-canary','terminal')")
            connection.close()
            before = database.read_bytes()
            report = probe.retained_metadata(root)
            self.assertEqual(report["logs"][0]["counts"]["connection_error"], 1)
            self.assertEqual(report["logs"][0]["counts"]["inference_request"], 1)
            self.assertEqual(report["recent_messages"][0]["tool"], "terminal")
            self.assertEqual(report["recent_messages"][0]["source"], "finitechat")
            self.assertEqual(report["agentd"]["processes"]["hermes"]["pid"], 123)
            self.assertEqual(report["heartbeats"]["state/gateway.heartbeat"]["mem"]["mem_available_kib"], 12000)
            self.assertNotIn("private-canary", json.dumps(report))
            self.assertEqual(before, database.read_bytes())

    def test_symlinked_logs_and_database_are_not_read(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            home = root / "agent/hermes-home"
            (home / "logs").mkdir(parents=True)
            secret = root / "secret"
            secret.write_text("ERROR connection error")
            (home / "logs/errors.log").symlink_to(secret)
            (home / "state.db").symlink_to(secret)
            report = probe.retained_metadata(root)
            self.assertEqual(report["logs"], [])
            self.assertNotIn("latest_messages", report)

    def test_fleet_metadata_never_opens_conversation_database(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            home = root / ".hermes"
            (home / "state").mkdir(parents=True)
            (home / "state.db").write_text("private-canary")
            (home / "state/gateway.heartbeat").write_text('{"pid":22}')
            with patch.object(probe.sqlite3, "connect") as connect:
                report = probe.retained_metadata(root, include_messages=False, hermes_home=home)
            connect.assert_not_called()
            self.assertEqual(report["heartbeats"]["state/gateway.heartbeat"]["pid"], 22)
            self.assertNotIn("private-canary", json.dumps(report))

    def test_fleet_inventory_excludes_owner_and_unrecognized_fields(self):
        status = SimpleNamespace(collect_core=lambda: {"runtimes": [{"agent_name": "test",
            "link_state": "active", "runtime_status": "stale", "owner_email": "private-canary",
            "credential": "private-canary"}]})
        query = SimpleNamespace(query=lambda sql: [])
        with patch.dict("sys.modules", {"finite_status": status, "finite_status_resources": query}):
            report = probe.fleet_inventory({})
        self.assertEqual(report["runtimes"][0]["runtime_status"], "stale")
        self.assertNotIn("private-canary", json.dumps(report))


    def test_classification_does_not_copy_provider_error_payloads(self):
        report = probe.classify(["2026-10-04T13:58:51 fatal ttrpc: closed api_key=private-canary",
                                 "2026-10-04T13:58:52 guest unresponsive private-canary"])
        self.assertEqual(report["counts"], {"transport_closed": 1, "guest_unreachable": 1})
        self.assertNotIn("private-canary", json.dumps(report))

    def test_watchdog_log_reports_intended_exit_separately_from_process_exit(self):
        report = probe.classify(["2026-10-04 03:55:07 CRITICAL Gateway event loop missed 3 "
            "consecutive liveness probes; dumping all thread stacks and exiting with code 75 "
            "so the service supervisor can restart it. private-canary"])
        self.assertEqual(report["events"][0]["missed_probes"], 3)
        self.assertEqual(report["events"][0]["exit_code"], 75)
        self.assertIn("event_loop_watchdog", report["events"][0]["signals"])
        self.assertNotIn("private-canary", json.dumps(report))

    def test_mismatched_durable_runtime_is_rejected_before_reading_files(self):
        report = {"runtimes": [{"container_present": True, "runtime_id": "runtime_expected",
                  "durable_root": "/data/finite-saas-runner/kata/runtime_other"}]}
        resources = SimpleNamespace(retirement_compute=lambda request: report)
        with patch.dict("sys.modules", {"finite_status": SimpleNamespace(),
                                       "finite_status_resources": resources}), \
             patch.object(probe, "retained_metadata") as read:
            with self.assertRaisesRegex(ValueError, "durable runtime identity mismatch"):
                probe.collect({})
            read.assert_not_called()

    def test_qmp_probe_sends_only_negotiation_and_read_only_status_query(self):
        # Unix socket paths have a small platform limit; keep the fixture short.
        with tempfile.TemporaryDirectory(dir="/tmp", prefix="qmp-") as directory:
            path = Path(directory) / "q.sock"
            commands = []
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(str(path))
                server.listen(1)
                def serve():
                    client, _ = server.accept()
                    with client:
                        client.sendall(b'{"QMP":{}}\n')
                        stream = client.makefile("rb")
                        for _ in range(2):
                            command = json.loads(stream.readline())
                            commands.append(command["execute"])
                            result = {} if command["execute"] == "qmp_capabilities" else {
                                "status": "paused", "running": False, "secret": "private-canary"}
                            client.sendall((json.dumps({"id":command["id"], "return":result}) + "\n").encode())
                worker = threading.Thread(target=serve, daemon=True)
                worker.start()
                result = probe.qmp_status(path)
                worker.join(timeout=5)
                self.assertFalse(worker.is_alive())
                self.assertEqual(commands, ["qmp_capabilities", "query-status"])
                self.assertEqual(result, {"available":True, "running":False, "status":"paused"})
                self.assertNotIn("private-canary", json.dumps(result))




if __name__ == "__main__":
    unittest.main()
