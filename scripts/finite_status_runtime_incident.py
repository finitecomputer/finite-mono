"""Content-free evidence for an exact Kata runtime, including an unreachable guest.

Invoked through finite-status --resource-usage with action runtime-incident.
Requests contain one to eight exact Runtime/Project/Runner/machine bindings.
Use metadata_only to skip scratch message metadata; recovery reads provider
bindings and never restores, drains output, or signals a process.
Optional guest process snapshots are bounded and read-only. Never restart
compute or export log/message bodies.
"""

import collections
import json
import re
import sqlite3
import socket
import hashlib
import shutil
import os
import tempfile
import stat
import subprocess
from contextlib import closing
from pathlib import Path

PATTERNS = {
    "out_of_memory": r"out of memory|oom.kill|oom-kill|killed process",
    "timeout": r"timed? ?out|timeouterror|deadline exceeded",
    "connection_error": r"connection error|apiconnectionerror|connection refused|connection reset",
    "provider_error": r"HTTP(?:/\S+)?(?: status)?[ :]+5\d\d|internalservererror",
    "provider_invalid_prefill": r"This model does not support assistant message prefill|conversation must end with a user message",
    "provider_bad_request": r"HTTP(?:/\S+)?(?: status)?[ :]+400\b|Error code: 400\b|BadRequestError",
    "provider_credit_error": r"HTTP(?:/\S+)?(?: status)?[ :]+402\b|insufficient (?:credits|funds)|credit balance.*(?:low|exhaust)",
    "provider_context_limit": r"context_length_exceeded|maximum context length|prompt is too long|too many tokens",
    "provider_tool_sequence": r"tool_use.*tool_result|tool_result.*tool_use|tool_call_ids|tool calls.*(?:response|follow)|unexpected.*tool_use_id",
    "provider_thinking_signature": r"invalid.*signature|signature.*(?:invalid|required)|thinking.*(?:signature|cannot be modified)",
    "provider_image_error": r"image.*(?:exceed|invalid|unsupported|too large)|(?:invalid|unsupported).*image",
    "rate_limit": r"rate.?limit|\b429\b",
    "inference_request": r"HTTP Request: POST .*chat/completions|calling.*(?:LLM|model)|API call",
    "gateway_start": r"gateway.*start|starting.*gateway",
    "gateway_stop": r"shutdown|shutting down|SIGTERM|gateway.*stopp",
    "turn_complete": r"agent.*complet|turn.*complet|response.*complet",
    "tool_activity": r"calling tool|executing tool|tool.*(?:complet|finished)",
    "incomplete_streamed_tool_call": r"Stream ended with no finish_reason while a tool call's arguments were still incomplete",
    "iteration_limit": r"max(?:imum)?[_ ]iterations|iteration limit",
    "transport_closed": r"ttrpc: closed|transport is closing|shim disconnected",
    "guest_unreachable": r"agent.*(?:not responding|unresponsive)|guest.*(?:not responding|unresponsive)",
    "disk_full": r"no space left on device|disk.*full",
    "event_loop_watchdog": r"Gateway event loop missed \d+ consecutive liveness probes",
    "shutdown_watchdog": r"Shutdown watchdog fired after",
}

# No imports from Hermes and no environment, config, log or message contents.
GUEST_PROCESSES = r'''
import json,os,re,time
from pathlib import Path
result={'epoch':time.time(),'processes':[]}
for proc in Path('/proc').iterdir():
 if not proc.name.isdigit():continue
 try:
  args=(proc/'cmdline').read_bytes().decode(errors='replace').split('\0')
  if not any(Path(a).name in ('hermes','hermes-agent','run.py') for a in args) or 'gateway' not in args:continue
  fields=(proc/'stat').read_text().rsplit(')',1)[1].split()
  threads=[]
  for thread in (proc/'task').iterdir():
   wait=(thread/'wchan').read_text().strip()
   item={'tid':int(thread.name),'wait':wait if re.fullmatch(r'[A-Za-z0-9_]{1,80}',wait) else None}
   if wait=='anon_pipe_write':
    item['start_ticks']=int((thread/'stat').read_text().rsplit(')',1)[1].split()[19])
    call=(thread/'syscall').read_text().split()
    if call and call[0]=='1' and int(call[1],16) in (1,2):item['blocked_stdio_fd']=int(call[1],16)
   threads.append(item)
  row={'pid':int(proc.name),'state':fields[0],
   'start_ticks':int(fields[19]),'cpu_ticks':int(fields[11])+int(fields[12]),'threads':threads[:200]}
  row['output_fds']={}
  for fd in ('1','2'):
   target=os.readlink(proc/'fd'/fd)
   row['output_fds'][fd]=target if re.fullmatch(r'pipe:\[\d+\]',target) else 'other'
  try:
   call=(proc/'syscall').read_text().split()
   if call and call[0]=='1':row['blocked_write_fd']=int(call[1],16)
  except (OSError,ValueError):pass
  result['processes'].append(row)
 except (OSError,ValueError):pass
print(json.dumps(result))
'''


GUEST_PIPE_OWNERS = GUEST_PROCESSES.replace('print(json.dumps(result))', r'''
pipes={pipe for process in result['processes'] for pipe in process.get('output_fds',{}).values()
       if re.fullmatch(r'pipe:\[\d+\]',pipe)}
owners=[];truncated=False;deadline=time.monotonic()+5
for proc in Path('/proc').iterdir():
 if not proc.name.isdigit():continue
 if time.monotonic()>deadline or len(owners)>=100:
  truncated=True;break
 try:
  matched=[]
  for fd in (proc/'fd').iterdir():
   pipe=os.readlink(fd)
   if pipe not in pipes:continue
   info=(proc/'fdinfo'/fd.name).read_text()
   flags=re.search(r'^flags:\s+([0-7]+)$',info,re.M)
   access={0:'read',1:'write',2:'read-write'}.get(int(flags[1],8)&3) if flags else None
   matched.append({'fd':int(fd.name),'pipe':pipe,'access':access})
  if not matched:continue
  fields=(proc/'stat').read_text().rsplit(')',1)[1].split()
  exe=Path(os.readlink(proc/'exe')).name
  component=exe if exe in {'finite-agentd','python3','python3.11','python3.12','python3.13','bash','sh','tee','init'} else 'other'
  threads=[]
  for thread in (proc/'task').iterdir():
   wait=(thread/'wchan').read_text().strip()
   threads.append({'tid':int(thread.name),'wait':wait if re.fullmatch(r'[A-Za-z0-9_]{1,80}',wait) else None})
  row={'pid':int(proc.name),'parent_pid':int(fields[1]),'start_ticks':int(fields[19]),
       'component':component,'fds':matched,'threads':threads[:200]}
  try:
   call=(proc/'syscall').read_text().split()
   if call and call[0] in {'0','1'}:
    row['blocked_io']={'syscall':'read' if call[0]=='0' else 'write','fd':int(call[1],16)}
  except (OSError,ValueError):pass
  owners.append(row)
 except (OSError,ValueError):pass
result['output_pipe_owners']={'owners':owners,'truncated':truncated}
print(json.dumps(result))
''')


def classify(lines):
    events = []
    for line in lines:
        signals = [name for name, pattern in PATTERNS.items() if re.search(pattern, line, re.I)]
        level = re.search(r"\b(WARNING|ERROR|CRITICAL)\b", line)
        if not signals and not level:
            continue
        stamp = re.search(r"20\d\d-\d\d-\d\d[ T]\d\d:\d\d:\d\d(?:[.,]\d+)?", line)
        entry = {"at": stamp[0] if stamp else None, "signals": signals}
        if level:
            entry["level"] = level[1]
        watchdog = re.search(r"Gateway event loop missed (\d+) consecutive liveness probes.*exiting with code (\d+)", line)
        if watchdog:
            entry.update(missed_probes=int(watchdog[1]), exit_code=int(watchdog[2]))
        events.append(entry)
    provider_events = [e for e in events if any(s.startswith('provider_') for s in e['signals'])]
    return {"counts": dict(collections.Counter(s for e in events for s in e["signals"])),
            "provider_events": provider_events[-60:],
            "events": events[-100:], "classified_events": len(events)}


def retained_metadata(root, *, include_messages=True, hermes_home=None):
    home = hermes_home if hermes_home is not None else root / "agent/hermes-home"
    report = {"logs": [], "heartbeats": {}}
    def safe_file(path):
        return (path.is_file() and not path.is_symlink()
                and path.resolve().is_relative_to(root.resolve()))
    for name in ("agent.log", "gateway.log", "errors.log"):
        path = home / "logs" / name
        if not safe_file(path):
            continue
        with path.open("rb") as stream:
            stream.seek(max(0, path.stat().st_size - 4_000_000))
            content = stream.read(4_000_000).decode("utf8", "replace")
        report["logs"].append({"file": name, "bytes": path.stat().st_size,
            "modified_at_epoch": path.stat().st_mtime, "scope": "last 4 MB",
            **classify(content.splitlines())})
    for name in ("state/gateway.heartbeat", "cron/ticker_heartbeat", "gateway_state.json", "state/gateway.lifecycle.json"):
        path = home / name
        if safe_file(path):
            report["heartbeats"][name] = {"modified_at_epoch": path.stat().st_mtime}
            if name in ("state/gateway.heartbeat", "state/gateway.lifecycle.json") and path.stat().st_size <= 32768:
                try:
                    value = json.loads(path.read_text())
                    if not isinstance(value, dict):
                        raise ValueError("invalid heartbeat shape")
                    selected = {key: value[key] for key in ("pid", "exit_code", "start_time", "monotonic")
                                if type(value.get(key)) in (int, float)}
                    for key in ("updated_at", "started_at", "exited_at"):
                        if isinstance(value.get(key), str) and re.fullmatch(r"[0-9T:.+Z-]{1,64}", value[key]):
                            selected[key] = value[key]
                    for key, allowed in (("phase", {"running", "exited"}),
                        ("exit_reason", {"loop_liveness_watchdog", "shutdown_watchdog", "graceful_shutdown"})):
                        if value.get(key) in allowed:
                            selected[key] = value[key]
                    if isinstance(value.get("mem"), dict):
                        selected["mem"] = {key: value["mem"][key] for key in
                            ("rss_kib", "mem_total_kib", "mem_available_kib", "swap_used_kib")
                            if type(value["mem"].get(key)) is int}
                    report["heartbeats"][name].update(selected)
                except (ValueError, TypeError):
                    report["heartbeats"][name]["invalid_json"] = True
    database = home / "state.db"
    agentd = root / "agent/agentd/status.json"
    if safe_file(agentd) and agentd.stat().st_size <= 65536:
        try:
            value = json.loads(agentd.read_text())
            report["agentd"] = {"modified_at_epoch": agentd.stat().st_mtime, "processes": {}}
            for name in ("hermes", "finitechat", "health"):
                process = value.get("processes", {}).get("processes", {}).get(name, {})
                selected = {key: process[key] for key in ("restart_count", "updated_at_ms", "pid")
                            if type(process.get(key)) is int}
                if process.get("state") in ("starting", "running", "restarting", "stopped", "exited", "unavailable"):
                    selected["state"] = process["state"]
                report["agentd"]["processes"][name] = selected
        except (ValueError, TypeError, AttributeError):
            report["agentd"] = {"invalid_json": True}
    if include_messages and safe_file(database):
        # A read-only WAL connection can still update the source -shm file.
        # Inspect a private scratch DB/WAL pair so the probe never writes
        # original runtime state or destabilizes a quiesced backup manifest.
        with tempfile.TemporaryDirectory(prefix='finite-incident-db-') as scratch:
            copied = Path(scratch)/'state.db'
            shutil.copyfile(database, copied)
            wal = Path(str(database)+'-wal')
            if safe_file(wal):
                shutil.copyfile(wal, Path(str(copied)+'-wal'))
            connection = sqlite3.connect(copied.as_uri() + "?mode=ro", uri=True, timeout=3)
            with closing(connection):
                return retained_message_projection(connection, report)
    return report


def retained_message_projection(connection, report):
    connection.execute("PRAGMA query_only=ON")
    connection.execute("BEGIN")
    report["latest_messages"] = [{"source": source, "role": role, "at_epoch": stamp}
        for source, role, stamp in connection.execute(
            "SELECT s.source,m.role,max(m.timestamp) FROM messages m JOIN sessions s "
            "ON s.id=m.session_id GROUP BY s.source,m.role")]
    columns = {r[1] for r in connection.execute("PRAGMA table_info(messages)")}
    tool = "m.tool_name" if "tool_name" in columns else "NULL"
    report["recent_messages"] = [{"source": source, "role": role, "at_epoch": stamp,
        "tool": name if isinstance(name, str) and re.fullmatch(r"[A-Za-z0-9_-]{1,80}", name) else None}
        for source, role, stamp, name in connection.execute(
            f"SELECT s.source,m.role,m.timestamp,{tool} FROM messages m JOIN sessions s "
            "ON s.id=m.session_id ORDER BY m.timestamp DESC LIMIT 40")]
    return report



def process_metadata(pid):
    root = Path("/proc") / str(pid)
    result = {"pid": pid}
    try:
        fields = (root / "stat").read_text().rsplit(")", 1)[1].split()
        result.update(scheduler_state=fields[0], cpu_ticks=int(fields[11])+int(fields[12]),
                      start_ticks=int(fields[19]), comm=(root / "comm").read_text().strip())
        result["memory"] = {line.split(":")[0]: line.split(":", 1)[1].strip()
            for line in (root / "status").read_text().splitlines()
            if line.split(":")[0] in ("VmRSS", "VmSwap", "Threads")}
        paths = [line.split(":", 2)[2] for line in (root / "cgroup").read_text().splitlines()
                 if line.startswith("0::")]
        if len(paths) == 1:
            base = Path("/sys/fs/cgroup").resolve()
            cgroup = (base / paths[0].lstrip("/")).resolve()
            if cgroup.is_relative_to(base):
                result["cgroup"] = {name: (cgroup / name).read_text().strip() for name in
                    ("memory.current", "memory.max", "memory.peak", "memory.events",
                     "memory.pressure", "memory.swap.current", "cpu.stat", "cpu.pressure", "io.pressure")
                    if (cgroup / name).is_file()}
    except (FileNotFoundError, ProcessLookupError):
        result["process_disappeared"] = True
    return result


def qmp_status(path):
    """Only negotiate QMP and query status; never stop, resume, reset, or quit."""
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(3)
        client.connect(str(path))
        stream = client.makefile("rb")
        def receive(expected=None):
            for _ in range(20):
                line = stream.readline(65537)
                if not line or len(line) > 65536:
                    raise ValueError("invalid QMP response")
                value = json.loads(line)
                if expected is None or value.get("id") == expected:
                    return value
            raise ValueError("QMP event bound exceeded")
        if "QMP" not in receive():
            raise ValueError("missing QMP greeting")
        for command, identifier in (("qmp_capabilities", "capabilities"), ("query-status", "status")):
            client.sendall((json.dumps({"execute": command, "id": identifier}) + "\n").encode())
            response = receive(identifier)
            if "error" in response:
                return {"available": False, "protocol_error": True}
        result = response.get("return", {})
        allowed = {"running", "paused", "shutdown", "prelaunch", "inmigrate", "internal-error",
                   "io-error", "debug", "finish-migrate", "postmigrate", "restore-vm", "suspended",
                   "watchdog", "guest-panicked", "colo"}
        return {"available": True, "running": result.get("running") is True,
                "status": result.get("status") if result.get("status") in allowed else "unknown"}


def provider_owner_metadata(pid, bundle):
    """Classify a retained provider owner without exporting argv or environment."""
    proc=Path('/proc',str(pid));result={'pid':pid}
    try:
        fields=(proc/'stat').read_text().rsplit(')',1)[1].split()
        exe=os.readlink(proc/'exe');args=(proc/'cmdline').read_bytes().split(b'\0')
        args=[a for a in args if a];wait=(proc/'wchan').read_text().strip()
        coreutils_sleep=bool(re.fullmatch(r'/nix/store/[a-z0-9]{32}-coreutils-[a-zA-Z0-9.+-]+/(?:bin|libexec)/(?:sleep|coreutils)',exe))
        infinity_args=bool(args) and Path(args[0].decode(errors='replace')).name=='sleep' and args[1:]==[b'infinity']
        allowed={'sleep','coreutils','busybox'}
        result.update(start_ticks=int(fields[19]),scheduler_state=fields[0],uid=proc.stat().st_uid,
            cwd_matches_bundle=Path(os.readlink(proc/'cwd'))==bundle,
            verified_coreutils_sleep_infinity=coreutils_sleep and infinity_args,
            coreutils_sleep_executable=coreutils_sleep,sleep_infinity_arguments=infinity_args,
            exe_basename=Path(exe).name if Path(exe).name in allowed else 'other',
            argv0_basename=Path(args[0].decode(errors='replace')).name if args and Path(args[0].decode(errors='replace')).name in allowed else 'other',
            wait=wait if re.fullmatch(r'[A-Za-z0-9_]{1,80}',wait) else None)
        result['same_host_namespaces']={name:os.stat(proc/'ns'/name).st_ino==os.stat(Path('/proc/1/ns')/name).st_ino
            for name in ['mnt','net','pid']}
        result['descriptor_kinds']=dict(collections.Counter(
            'socket' if stat.S_ISSOCK(fd.stat().st_mode) else ('pipe' if stat.S_ISFIFO(fd.stat().st_mode) else 'other')
            for fd in (proc/'fd').iterdir()))
    except (OSError,ValueError):result['available']=False
    return result


def hypervisor_metadata(runtime):
    from finite_status_resources import retirement_quiescence
    container = runtime["container_id"]
    result = {"persist_records": [], "qmp": {"available": False}}
    bundle = Path('/run/containerd/io.containerd.runtime.v2.task/finite') / container
    result['provider_bundle'] = {'present': bundle.is_dir(), 'is_symlink': bundle.is_symlink()}
    if bundle.is_dir() and not bundle.is_symlink():
        result['provider_bundle']['entry_names'] = sorted(p.name for p in bundle.iterdir())
        result['provider_bundle']['ownership'] = retirement_quiescence(bundle)
        result['provider_bundle']['owner_metadata']=[provider_owner_metadata(p['pid'],bundle)
            for p in result['provider_bundle']['ownership']['durable_root_process_references']]
        work=Path('/var/lib/containerd/io.containerd.runtime.v2.task/finite')/container
        result['provider_work']={'path':str(work),'present':work.exists(),'is_symlink':work.is_symlink()}
        if work.is_dir() and not work.is_symlink():
            result['provider_work']['ownership']=retirement_quiescence(work)
        pointer=bundle/'work'
        result['provider_bundle']['expected_work_pointer']=pointer.is_symlink() and pointer.resolve()==work
        sockets=[line.split()[7] for line in Path('/proc/net/unix').read_text().splitlines()[1:] if len(line.split())==8]
        for name,path in [('provider_bundle',bundle),('provider_work',work)]:
            result[name]['live_socket_count']=sum(Path(x).is_relative_to(path) for x in sockets if x.startswith('/'))
    volatile = Path('/run/vc/vm') / container
    mounts = [line.split()[4] for line in Path('/proc/self/mountinfo').read_text().splitlines()]
    sockets = [line.split()[7] for line in Path('/proc/net/unix').read_text().splitlines()[1:] if len(line.split()) == 8]
    result['volatile_vm_directory'] = {'present': volatile.exists(), 'is_symlink': volatile.is_symlink(),
        'mount_count': sum(Path(p).is_relative_to(volatile) for p in mounts),
        'live_socket_count': sum(Path(p).is_relative_to(volatile) for p in sockets if p.startswith('/'))}
    for path in Path("/run/vc/sbs").glob("*/persist.json"):
        if path.is_symlink() or path.stat().st_size > 1048576:
            continue
        value = json.loads(path.read_text())
        if value.get("SandboxContainer") != container:
            continue
        hypervisor = value.get("HypervisorState", {})
        result["persist_records"].append({"sandbox_id": path.parent.name,
            "state": value.get("State") if value.get("State") in ("running", "paused", "stopped", "ready") else "unknown",
            "hypervisor_pid": hypervisor.get("Pid") if type(hypervisor.get("Pid")) is int else None,
            "qemu": hypervisor.get("Type") == "qemu"})
    for process in runtime["sandbox_processes"]:
        if "qemu" not in process["comm"]:
            continue
        proc = Path("/proc", str(process["pid"]))
        try:
            fields = (proc / "stat").read_text().rsplit(")", 1)[1].split()
            arguments = (proc / "cmdline").read_bytes().decode().split("\0")
            if int(fields[19]) != process["pid_start_ticks"] or not any(container in arg for arg in arguments):
                raise ValueError("hypervisor process identity changed")
            sockets = [arguments[index+1].split(",", 1)[0].removeprefix("unix:").removeprefix("path=")
                       for index, arg in enumerate(arguments[:-1]) if arg == "-qmp"
                       and arguments[index+1].startswith("unix:")]
            for index, arg in enumerate(arguments[:-1]):
                if arg == "-chardev" and "id=qmp" in arguments[index+1]:
                    sockets.extend(re.findall(r"(?:^|,)path=([^,]+)", arguments[index+1]))
            result["qmp_socket_count"] = len(sockets)
            if len(sockets) == 1:
                result["qmp_socket_kind"] = "abstract" if sockets[0].startswith("@") else (
                    "inherited_fd" if sockets[0].startswith("fd=") else (
                    "path" if sockets[0].startswith("/") else "unknown"))
                if re.fullmatch(r"fd=\d+", sockets[0]):
                    descriptor = os.readlink(proc / "fd" / sockets[0][3:])
                    inode = re.fullmatch(r"socket:\[(\d+)\]", descriptor)
                    if inode:
                        matches = [line.split()[7] for line in (proc / "net/unix").read_text().splitlines()[1:]
                                   if len(line.split()) == 8 and line.split()[6] == inode[1]]
                        if len(matches) == 1:
                            sockets = matches
                if re.fullmatch(r"/[A-Za-z0-9_./-]{1,255}", sockets[0]):
                    result["qmp_socket_path"] = sockets[0]
                path = Path(sockets[0]).resolve()
                # The argument comes from the verified exact QEMU process.
                # Kata may shorten its socket directory to fit Unix limits.
                result["qmp_path_valid"] = (
                    path.is_relative_to("/run/vc") or path.is_relative_to("/run/kata-containers"))
                if result["qmp_path_valid"]:
                    result["qmp"] = qmp_status(path)
        except (OSError, ValueError) as error:
            result["qmp"] = {"available": False, "error_type": type(error).__name__}
    return result


def container_snapshot_metadata(container, inventory=False):
    """Read retained snapshot ownership and mount instructions; never mount."""
    import finite_status as status
    response=status.run_read_only(['ctr','--namespace','finite','containers','info',container])
    if response.returncode:return {'available':False}
    item=json.loads(response.stdout)
    if item.get('ID')!=container:raise ValueError('snapshot container identity changed')
    key=item.get('SnapshotKey');snapshotter=item.get('Snapshotter')
    if not isinstance(key,str) or not re.fullmatch(r'[a-zA-Z0-9_.-]{1,255}',key) or not isinstance(snapshotter,str) or not re.fullmatch(r'[a-zA-Z0-9_.-]+',snapshotter):
        raise ValueError('snapshot binding unavailable')
    result={'available':True,'key_sha256':hashlib.sha256(key.encode()).hexdigest(),'snapshotter':snapshotter,
        'spec_sha256':hashlib.sha256(json.dumps(item.get('Spec'),sort_keys=True).encode()).hexdigest()}
    labels=item.get('Labels',{})
    result['logging_label_names']=sorted(k for k in labels if re.fullmatch(r'(?:nerdctl|io\.containerd)[A-Za-z0-9_./-]*(?:log|io)[A-Za-z0-9_./-]*',k))
    from urllib.parse import urlsplit
    uri=labels.get('nerdctl/log-uri','')
    parsed=urlsplit(uri)
    binary=Path(parsed.path).name
    result['logging']={'scheme':parsed.scheme if parsed.scheme in {'binary','file','fifo'} else 'other',
        'binary':binary if binary in {'sleep','nerdctl','.nerdctl-wrapped','cat'} else 'other',
        'uri_sha256':hashlib.sha256(uri.encode()).hexdigest()}
    result['restart_monitor']={'policy':labels.get('containerd.io/restart.policy') if labels.get('containerd.io/restart.policy') in {'no','always','unless-stopped','on-failure'} else None,
        'desired_status':labels.get('containerd.io/restart.status') if labels.get('containerd.io/restart.status') in {'running','stopped'} else None,
        'explicitly_stopped':{'true':True,'false':False}.get(labels.get('containerd.io/restart.explicitly-stopped'))}
    # ctr snapshots mounts only prints Mounts RPC instructions. It does not
    # execute a mount, prepare a snapshot, or mutate container metadata.
    response=status.run_read_only(['ctr','--namespace','finite','snapshots','--snapshotter',snapshotter,
        'mounts','/tmp/finite-read-only-no-mount',key])
    paths=re.findall(r'(?:^|[, ])upperdir=([^,\s]+)',response.stdout)
    if response.returncode==0 and len(paths)==1 and re.fullmatch(r'/(?:var/lib|data)/containerd/io.containerd.snapshotter.v1.overlayfs/snapshots/\d+/fs',paths[0]):
        root=Path(paths[0]);result['writable_layer_path']=str(root)
        result['writable_layer_is_symlink']=root.is_symlink()
        if inventory and root.is_dir() and not root.is_symlink():
            size=status.run_read_only(['nice','-n','15','du','-sx','--block-size=1',str(root)],timeout=45)
            if size.returncode==0:result['writable_layer_allocated_bytes']=int(size.stdout.split()[0])
            kinds=collections.Counter();top=collections.Counter()
            allowed={'root','home','usr','tmp','var','opt','data','run','etc','work','workspace'}
            for base,dirs,files in os.walk(root,followlinks=False):
                for name in dirs+files:
                    path=Path(base)/name;info=path.lstat();relative=path.relative_to(root)
                    kind='file' if stat.S_ISREG(info.st_mode) else ('directory' if stat.S_ISDIR(info.st_mode) else (
                        'symlink' if stat.S_ISLNK(info.st_mode) else ('socket' if stat.S_ISSOCK(info.st_mode) else (
                        'overlay_whiteout' if stat.S_ISCHR(info.st_mode) and info.st_rdev==0 else 'other_special'))))
                    kinds[kind]+=1
                    top[relative.parts[0] if relative.parts[0] in allowed else 'other']+=info.st_blocks*512
            result['writable_layer_file_kinds']=dict(kinds);result['writable_layer_top_allocated_bytes']=dict(top)
    return result


def provider_logging_processes(runtime):
    """Metadata for descendants of the exact sandbox shim; no FD data reads."""
    shims=[p for p in runtime['sandbox_processes'] if p['comm'].startswith('containerd-shim')]
    if len(shims)!=1:raise ValueError('exact sandbox shim required for logging evidence')
    shim=shims[0];records={}
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit():continue
        try:
            fields=(proc/'stat').read_text().rsplit(')',1)[1].split()
            records[int(proc.name)]=(int(fields[1]),int(fields[19]),proc)
        except (OSError,ValueError):continue
    if records.get(shim['pid'],(None,None))[1]!=shim['pid_start_ticks']:
        raise ValueError('sandbox shim start identity changed')
    scope={shim['pid']}
    for process in runtime['sandbox_processes']:
        if records.get(process['pid'],(None,None))[1]!=process['pid_start_ticks']:
            continue
        scope.add(process['pid'])
    bundles=[Path('/run/containerd/io.containerd.runtime.v2.task/finite')/runtime['container_id'],
             Path('/var/lib/containerd/io.containerd.runtime.v2.task/finite')/runtime['container_id']]
    for pid,(_,_,proc) in records.items():
        try:
            cwd=Path(os.readlink(proc/'cwd'))
            if any(cwd.is_relative_to(bundle) for bundle in bundles):scope.add(pid)
        except OSError:pass
    for _ in range(20):
        more={pid for pid,(parent,_,_) in records.items() if parent in scope}
        if more.issubset(scope):break
        scope.update(more)
    result=[]
    for pid in sorted(scope):
        parent,start,proc=records[pid];fds=[]
        try:
            comm=(proc/'comm').read_text().strip()
            component=comm if comm in {'sleep','nerdctl','.nerdctl-wrappe','containerd-shim','containerd-shim-kata-v2','containerd'} else 'other'
            env=dict(part.split(b'=',1) for part in (proc/'environ').read_bytes().split(b'\0') if b'=' in part)
            logger_binding={'container_matches':env.get(b'CONTAINER_ID')==runtime['container_id'].encode(),
                'namespace_matches':env.get(b'CONTAINER_NAMESPACE')==b'finite'}
            args=(proc/'cmdline').read_bytes().split(b'\0')
            operation=next((v.decode() for v in args if v in {b'logs',b'exec',b'start',b'stop',b'_NERDCTL_INTERNAL_LOGGING'}),None)
            for fd in (proc/'fd').iterdir():
                target=os.readlink(fd);info=(proc/'fdinfo'/fd.name).read_text()
                flags=re.search(r'^flags:\s+([0-7]+)$',info,re.M)
                access={0:'read',1:'write',2:'read-write'}.get(int(flags[1],8)&3) if flags else None
                mode=fd.stat().st_mode
                kind='pipe' if stat.S_ISFIFO(mode) else ('socket' if stat.S_ISSOCK(mode) else 'other')
                if kind in {'pipe','socket'} or fd.name in {'0','1','2'}:
                    fds.append({'fd':int(fd.name),'kind':kind,'access':access,
                        'pipe_identity':target if re.fullmatch(r'pipe:\[\d+\]',target) else None})
            threads=[]
            for thread in (proc/'task').iterdir():
                wait=(thread/'wchan').read_text().strip()
                threads.append({'tid':int(thread.name),'wait':wait if re.fullmatch(r'[A-Za-z0-9_]{1,80}',wait) else None})
            result.append({'pid':pid,'parent_pid':parent,'start_ticks':start,'component':component,
                'logger_binding':logger_binding,'operation':operation,'fds':fds[:100],'threads':threads[:100]})
        except (OSError,ValueError):continue
    return result


def collect(request):
    retired = {'recovery_progress', 'history_preservation', 'guest_output_recovery',
               'post_recovery_history_preservation'}
    if retired.intersection(request):
        raise ValueError('incident-specific recovery selectors are retired')
    import finite_status as status
    from finite_status_resources import retirement_compute
    report = retirement_compute(request)
    for runtime in report["runtimes"]:
        if not runtime.get("container_present"):
            continue
        if (runtime["durable_root"] != "/data/finite-saas-runner/kata/" + runtime["runtime_id"]
                or runtime.get("data_root_is_symlink")):
            raise ValueError("incident durable runtime identity mismatch")
        runtime["retained_metadata"] = retained_metadata(Path(runtime["durable_root"]),
            include_messages=not request.get("metadata_only", False))
        runtime["host_processes"] = [process_metadata(p["pid"]) for p in runtime["sandbox_processes"]]
        if request.get('provider_logging'):
            runtime['provider_logging']=provider_logging_processes(runtime)
            runtime['container_snapshot']=container_snapshot_metadata(runtime['container_id'])
        if request.get("guest_processes"):
            try:
                guest = status.run_read_only(["nerdctl", "--namespace", "finite", "exec",
                    runtime["container_id"], "/usr/local/bin/python3", "-B", "-c",
                    GUEST_PIPE_OWNERS if request.get('guest_pipe_owners') else GUEST_PROCESSES], timeout=10)
                runtime["guest_processes"] = json.loads(guest.stdout) if guest.returncode == 0 else {"available": False}
            except status.CollectionError:
                runtime["guest_processes"] = {"available": False}
        if request.get("metadata_only") and not request.get('recovery_bindings_only'):
            continue
        if request.get("recovery"):
            if not request.get('recovery_bindings_only'):
                runtime["hypervisor"] = hypervisor_metadata(runtime)
            runtime['container_snapshot']=container_snapshot_metadata(runtime['container_id'],inventory=request.get('writable_layer_inventory',False))
            inspected = status.run_read_only(["nerdctl", "--namespace", "finite", "inspect", runtime["source_machine_id"]])
            if inspected.returncode == 0:
                items = json.loads(inspected.stdout)
                if len(items) == 1 and items[0].get("Id") == runtime["container_id"]:
                    image = items[0].get("Config", {}).get("Image", "")
                    if re.fullmatch(r"[A-Za-z0-9_./:-]+@sha256:[a-f0-9]{64}", image):
                        runtime["image_reference"] = image
            root = Path(runtime["durable_root"])
            usage = shutil.disk_usage(root)
            runtime["storage"] = {"filesystem_free_bytes": usage.free, "filesystem_total_bytes": usage.total}
            size = status.run_read_only(["nice", "-n", "15", "du", "-sx", "--block-size=1", str(root)], timeout=45)
            if size.returncode == 0:
                runtime["storage"]["durable_allocated_bytes"] = int(size.stdout.split()[0])
            identity = root / "agent/identity/identity.json"
            if (identity.is_file() and not identity.is_symlink() and identity.stat().st_size <= 65536
                    and identity.resolve().is_relative_to(root)):
                runtime["identity_sha256"] = hashlib.sha256(identity.read_bytes()).hexdigest()
        if request.get('recovery_bindings_only'):
            continue
        # Filter journal entries on the exact full container id or machine name.
        # Journal text is classified here; no raw line reaches the report.
        journal = status.run_read_only(["journalctl", "--no-pager", "--since", "36 hours ago",
            "-u", "finite-saas-runner.service", "-u", "containerd.service", "-o", "short-iso", "-n", "20000"], timeout=30)
        runtime["host_journal"] = classify(line for line in journal.stdout.splitlines()
            if runtime["container_id"] in line or runtime["source_machine_id"] in line)
        runtime["host_journal"]["available"] = journal.returncode == 0
        try:
            console = status.run_read_only(["nerdctl", "--namespace", "finite", "logs",
                "--tail", "2000", runtime["source_machine_id"]], timeout=10)
            runtime["console"] = classify(console.stdout.splitlines())
            runtime["console"]["available"] = console.returncode == 0
            # Faulthandler stacks contain locations only. Allowlist upstream
            # module basenames and identifier-shaped functions; no source lines.
            frames = []
            allowed = {"run.py", "shutdown_watchdog.py", "threading.py", "queues.py",
                "events.py", "base_events.py", "selector_events.py", "futures.py",
                "subprocess.py", "session.py", "hermes_logging.py", "scheduler.py",
                "execute_code.py", "terminal_tool.py", "agent.py", "run_agent.py"}
            for line in console.stdout.splitlines():
                match = re.search(r'File "([^"]+)", line (\d+) in ([A-Za-z_][A-Za-z0-9_]*)$', line)
                if match and Path(match[1]).name in allowed:
                    frames.append({"module": Path(match[1]).name, "line": int(match[2]), "function": match[3]})
            runtime["console"]["known_stack_frames"] = frames[-100:]
        except status.CollectionError:
            runtime["console"] = {"available": False}
    report["host_memory"] = {line.split(":")[0]: line.split(":", 1)[1].strip()
        for line in Path("/proc/meminfo").read_text().splitlines()
        if line.split(":")[0] in ("MemTotal", "MemAvailable", "SwapTotal", "SwapFree")}
    report["host_pressure"] = {name: Path("/proc/pressure", name).read_text().strip()
        for name in ("cpu", "memory", "io") if Path("/proc/pressure", name).is_file()}
    report["limitation"] = "Host and retained metadata only; running task state does not prove a responsive guest. QMP may be occupied by the Kata shim; a query timeout alone does not prove a QEMU hang. No repair or new inference request."
    if request.get('rehearsal_audit'):
        ids = request.get('rehearsal_container_ids', [])
        if any(not isinstance(cid, str) or not re.fullmatch(r'[a-f0-9]{64}', cid) for cid in ids):
            raise ValueError('invalid rehearsal audit identity')
        processes = []
        for proc in Path('/proc').iterdir():
            if not proc.name.isdigit(): continue
            try:
                args = (proc/'cmdline').read_bytes().split(b'\0')
                matches = [cid for cid in ids if any(cid.encode() in a for a in args)]
                if matches: processes.append(dict(process_metadata(int(proc.name)), container_ids=matches))
            except (OSError, ValueError): continue
        report['rehearsal_audit'] = {'processes': processes}
    return report




def fleet_inventory(request):
    """All Core runtime links plus content-free recent lifecycle failures."""
    import finite_status as status
    from finite_status_resources import query
    rows = status.collect_core()["runtimes"]
    fields = ("source_host_id", "agent_runtime_id", "project_id", "source_machine_id",
              "agent_name", "version_label", "link_state", "control_kind", "control_status",
              "runtime_status", "health_reported_at", "health_ready",
              "health_report_interval_seconds")
    controls = query("""
SELECT coalesce(json_agg(row_to_json(x)), '[]'::json) FROM (
 SELECT r.id, r.agent_runtime_id, r.kind, r.status, r.failure_stage,
        r.created_at, r.completed_at,
        r.failure_message ~* 'nerdctl.*timed out' AS nerdctl_timeout,
        r.failure_message ~* '210s' AS timeout_210_seconds
 FROM runtime_control_requests r
 WHERE r.created_at >= NOW() - INTERVAL '7 days'
 ORDER BY r.created_at DESC LIMIT 1000
) x;
""")
    return {"runtimes": [{key: row.get(key) for key in fields} for row in rows],
            "recent_controls": controls, "control_history_scope": "last 7 days, at most 1000 operations"}
