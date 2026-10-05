"""Exact read-only Kata Runtime bindings for canonical incident probes.

Requests supply Runtime/Project/Runner/machine identities and an optional
expected container ID. No selector confers authority to repair or purge data.
"""

import json
import os
import re
import socket
from pathlib import Path
from urllib.request import urlopen
from urllib.error import HTTPError, URLError

import finite_status as status

def query(sql, parameters=None):
    command = ["psql", "--no-psqlrc", "--quiet", "--tuples-only", "--no-align",
               "--set", "ON_ERROR_STOP=1", "--dbname", status.CONTRACT["database"]["name"]]
    for key, value in (parameters or {}).items():
        command.extend(["--set", f"{key}={value}"])
    result = status.run_read_only(
        command, environment=status.postgres_environment(),
        input_text="BEGIN TRANSACTION READ ONLY;\nSET LOCAL statement_timeout = '10s';\n"
        + sql + "\nCOMMIT;\n",
    )
    if result.returncode:
        raise ValueError("resource inventory read-only Core query failed")
    return json.loads(result.stdout)


def command_json(command, timeout=30):
    result = status.run_read_only(command, timeout=timeout)
    if result.returncode:
        raise ValueError(f"{Path(command[0]).name} read-only resource probe failed")
    return json.loads(result.stdout)


def validate_runtime(row, host):
    if row.get("source_host_id") != host:
        raise ValueError("resource subject belongs to a different host")
    if not re.fullmatch(r"runtime_[a-zA-Z0-9]+", row.get("runtime_id", "")):
        raise ValueError("invalid resource runtime identity")
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}", row.get("source_machine_id", "")):
        raise ValueError("invalid resource machine identity")


def validate_inspect(row, item, host):
    labels = item.get("Config", {}).get("Labels", {})
    for suffix, expected in (("source_host_id", host),
                             ("source_machine_id", row["source_machine_id"]),
                             ("project_id", row["project_id"])):
        if labels.get("computer.finite.v2." + suffix) != expected:
            raise ValueError("container ownership does not match Core resource subject")
    mounts = [m for m in item.get("Mounts", []) if m.get("Destination") == "/data"]
    if len(mounts) != 1:
        raise ValueError("durable data mount is ambiguous")
    root = Path(mounts[0]["Source"]).resolve()
    if not root.is_relative_to("/data/finite-saas-runner/kata"):
        raise ValueError("durable data mount is outside the declared runner root")
    return root


def retirement_compute(request):
    """Exact container/task binding for a retirement fence; no guest contents."""
    host = socket.gethostname().split('.', 1)[0]
    rows = request.get('runtimes', [])
    if not 1 <= len(rows) <= 8:
        raise ValueError('one to eight exact retirement subjects required')
    ids = status.run_read_only(['nerdctl', '--namespace', 'finite', 'ps', '-aq', '--no-trunc'])
    if ids.returncode:
        raise ValueError('retirement container inventory failed')
    identifiers = ids.stdout.split()
    if len(identifiers) > 256:
        raise ValueError('retirement container inventory exceeds bound')
    inventory = command_json(['nerdctl', '--namespace', 'finite', 'inspect', *identifiers]) if identifiers else []
    tasks = status.run_read_only(['ctr', '--namespace', 'finite', 'tasks', 'list'])
    if tasks.returncode:
        raise ValueError('retirement task inventory failed')
    task_rows = [line.split() for line in tasks.stdout.splitlines()[1:] if line.split()]
    if any(len(task) < 3 or not task[1].isdigit() for task in task_rows):
        raise ValueError('invalid retirement task inventory')
    result = []
    for row in rows:
        validate_runtime(row, host)
        matches = [item for item in inventory
                   if item.get('Name', '').lstrip('/') == row['source_machine_id']]
        entry = dict(row, container_present=bool(matches))
        if len(matches) > 1:
            raise ValueError('ambiguous retirement container identity')
        expected_id = row.get('expected_container_id')
        if expected_id is not None and not re.fullmatch(r'[a-f0-9]{64}', expected_id):
            raise ValueError('invalid expected retirement container identity')
        root = Path('/data/finite-saas-runner/kata', row['runtime_id'])
        metadata_root = Path('/data/finite-saas-runner/kata-metadata', row['source_machine_id'])
        lock=metadata_root/'runtime-operation.lock'
        entry['operation_lock']={'present':lock.is_file(),'is_symlink':lock.is_symlink()}
        if lock.is_file() and not lock.is_symlink():
            info=lock.stat();entry['operation_lock'].update(device=info.st_dev,inode=info.st_ino,uid=info.st_uid,mode=info.st_mode&0o777)
        entry.update(durable_root=str(root.resolve()), data_root_is_symlink=root.is_symlink(),
                     durable_root_present=root.exists(), metadata_root_present=metadata_root.exists(),
                     metadata_root_is_symlink=metadata_root.is_symlink())
        if root.exists():
            info = root.stat()
            entry['durable_root_device'] = info.st_dev
            entry['durable_root_inode'] = info.st_ino
        if matches:
            item = matches[0]
            if expected_id is not None and item['Id'] != expected_id:
                raise ValueError('retirement container identity changed')
            root = validate_inspect(row, item, host)
            entry.update(container_id=item['Id'], container_state=item.get('State', {}).get('Status'),
                         container_running=item.get('State', {}).get('Running'),
                         durable_root=str(root), data_root_is_symlink=any(
                             Path(m['Source']).is_symlink() for m in item.get('Mounts', [])
                             if m.get('Destination') == '/data'))
            if request.get('readiness'):
                selected = {}
                for path in ['/etc/finite/runner-shared.env', '/etc/finite/runner.env']:
                    selected.update(status.read_environment_values(Path(path), {
                        'FC_RUNNER_KATA_HOST_ADDRESS', 'FC_RUNNER_KATA_CONTAINER_PORT',
                        'FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS'}))
                port = status.run_read_only(['nerdctl', '--namespace', 'finite', 'port',
                    row['source_machine_id'], selected.get('FC_RUNNER_KATA_CONTAINER_PORT', '8080')+'/tcp'])
                outputs = port.stdout.strip().splitlines()
                if port.returncode or len(outputs) != 1:
                    raise ValueError('ambiguous retirement HTTP port')
                host_port = int(outputs[0].rsplit(':', 1)[1])
                address = selected.get('FC_RUNNER_KATA_HOST_ADDRESS', '127.0.0.1')
                import ipaddress
                if not ipaddress.ip_address(address).is_private or not 1 <= host_port <= 65535:
                    raise ValueError('invalid retirement HTTP binding')
                entry['readiness'] = {'timeout_seconds': selected.get('FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS', '180')}
                try:
                    with urlopen(f'http://{address}:{host_port}/contact', timeout=5) as response:
                        body = response.read(65537)
                        entry['readiness']['http_status'] = response.status
                    if len(body) > 65536:
                        raise ValueError('retirement contact response exceeds bound')
                    contact = json.loads(body)
                    principal = contact.get('agent_npub')
                    entry['readiness']['valid_principal'] = isinstance(principal, str) and principal.startswith('npub1') and len(principal) <= 256
                    if entry['readiness']['valid_principal']:
                        import hashlib
                        entry['readiness']['principal_sha256'] = hashlib.sha256(principal.encode()).hexdigest()
                    if row.get('expected_principal'):
                        entry['readiness']['expected_principal_matches'] = principal == row['expected_principal']
                    if type(contact.get('ready')) is bool:
                        entry['readiness']['ready'] = contact['ready']
                except HTTPError as error:
                    entry['readiness']['http_status'] = error.code
                except (URLError, TimeoutError):
                    entry['readiness']['error'] = 'contact unavailable'
        entry['shared_mount_containers'] = [other['Id'] for other in inventory
            if any(Path(m.get('Source', '/')).resolve() == root or
                   Path(m.get('Source', '/')).resolve().is_relative_to(root)
                   for m in other.get('Mounts', []))]
        container_id = entry.get('container_id', expected_id)
        matching_tasks = [task for task in task_rows if task[0] == container_id]
        entry['tasks'] = [{'id':task[0], 'pid':int(task[1]), 'state':task[2],
                           'pid_exists':Path('/proc', task[1]).exists()}
                          for task in matching_tasks]
        for task in entry['tasks']:
            if task['pid_exists']:
                process = Path('/proc', str(task['pid']))
                try:
                    task['pid_comm'] = (process/'comm').read_text().strip()
                    fields = (process/'stat').read_text().rsplit(')', 1)[1].split()
                    task['pid_scheduler_state'] = fields[0]
                    task['pid_parent'] = int(fields[1])
                    task['pid_start_ticks'] = int(fields[19])
                    task['pid_sandbox_matches'] = container_id.encode() in (process/'cmdline').read_bytes()
                except (FileNotFoundError, ProcessLookupError):
                    task['pid_exists'] = False
        entry['sandbox_processes'] = []
        if container_id:
            for process in Path('/proc').iterdir():
                if not process.name.isdigit():
                    continue
                try:
                    arguments = (process/'cmdline').read_bytes().decode(errors='replace').split('\0')
                    if not any(container_id in argument for argument in arguments):
                        continue
                    fields = (process/'stat').read_text().rsplit(')', 1)[1].split()
                    entry['sandbox_processes'].append({'pid':int(process.name),
                        'comm':(process/'comm').read_text().strip(), 'pid_parent':int(fields[1]),
                        'pid_start_ticks':int(fields[19]), 'pid_scheduler_state':fields[0],
                        'exact_id_argument':container_id in arguments,
                        'namespace_finite':any(arguments[i:i+2] == ['-namespace', 'finite'] for i in range(len(arguments)-1))})
                except (FileNotFoundError, ProcessLookupError):
                    continue
        request_id = row.get('request_id')
        if request_id is not None:
            if not re.fullmatch(r'runtime_ctl_[a-zA-Z0-9]+', request_id):
                raise ValueError('invalid retirement operation identity')
            progress_path = Path('/data/finite-saas-runner/kata-metadata',
                                 row['source_machine_id'], 'retirement-'+request_id+'.json')
            entry['progress'] = None
            if progress_path.exists():
                if progress_path.is_symlink() or progress_path.stat().st_size > 32768:
                    raise ValueError('invalid retirement progress metadata')
                progress = json.loads(progress_path.read_text())
                for key, value in [('request_id', request_id), ('project_id', row['project_id']),
                                   ('agent_runtime_id', row['runtime_id']),
                                   ('container_name', row['source_machine_id'])]:
                    if progress.get(key) != value:
                        raise ValueError('retirement progress binding changed')
                entry['progress'] = {key: progress.get(key) for key in
                    ['request_id', 'phase', 'baseline_active_sandbox_count', 'container_id']}
        if request.get('quiescence'):
            entry.update(retirement_quiescence(root))
        result.append(entry)
    activity = []
    for process in Path('/proc').iterdir():
        if not process.name.isdigit():
            continue
        try:
            comm = (process/'comm').read_text().strip()
            if not any(part in comm for part in ['nerdctl', 'borg', 'finite-saas-run']):
                continue
            arguments = (process/'cmdline').read_bytes().decode(errors='replace').split('\0')
            subjects = [row['source_machine_id'] for row in rows if row['source_machine_id'] in arguments]
            verbs = [value for value in arguments if value in ['stop', 'rm', 'inspect', 'port', 'create', 'extract', 'run-once']]
            activity.append({'pid':int(process.name), 'comm':comm,
                             'verbs':verbs, 'matching_machines':subjects})
        except (FileNotFoundError, ProcessLookupError):
            continue
    managed = [item for item in inventory if
        item.get('Config', {}).get('Labels', {}).get('computer.finite.v2.runtime') == 'true' and
        item.get('Config', {}).get('Labels', {}).get('computer.finite.v2.source_host_id') == host]
    states = {}
    for item in managed:
        state = item.get('State', {}).get('Status') or 'unknown'
        states[state] = states.get(state, 0) + 1
    return {'host':host, 'runtimes':result, 'lifecycle_processes':activity[:64],
            'managed_container_count':len(managed), 'managed_container_states':states,
            'namespace_task_count':len(task_rows),
            'limitation':'Read-only container/task identity; does not prove offboarding or authorize deletion.'}


def retirement_quiescence(root):
    """Look for host writers and nested mounts without exporting their contents."""
    references = []
    needle = str(root).encode()
    total_descriptors = 0
    processes = [p for p in Path('/proc').iterdir() if p.name.isdigit()]
    if len(processes) > 8192:
        raise ValueError('retirement process inventory exceeds bound')
    for process in processes:
        try:
            linked = needle in process.joinpath('cmdline').read_bytes()
            linked = linked or needle in process.joinpath('maps').read_bytes()
            descriptors = list(process.joinpath('fd').iterdir())
            total_descriptors += len(descriptors)
            if total_descriptors > 4194304:
                raise ValueError('retirement descriptor inventory exceeds bound')
            for path in [process/'cwd', process/'root', *descriptors]:
                try:
                    target = Path(os.readlink(path).removesuffix(' (deleted)'))
                except FileNotFoundError:
                    continue
                linked = linked or target == root or target.is_relative_to(root)
            if linked:
                references.append({'pid': int(process.name), 'comm': (process/'comm').read_text().strip()})
        except (FileNotFoundError, ProcessLookupError):
            continue
        except PermissionError:
            raise ValueError('retirement process inventory is incomplete') from None
    def unescape(value):
        return re.sub(r'\\([0-7]{3})', lambda m: chr(int(m[1], 8)), value)
    mounts = []
    for line in Path('/proc/self/mountinfo').read_text().splitlines():
        path = Path(unescape(line.split()[4]))
        if path == root or path.is_relative_to(root):
            mounts.append(str(path))
    return {'durable_root_process_references': references, 'durable_root_nested_mounts': mounts}


def collect(request):
    if not isinstance(request, dict):
        raise ValueError("resource evidence request must be an object")
    if request.get("action") not in ("runtime-incident", "runtime-fleet", "retirement-compute"):
        raise ValueError("unsupported resource evidence action")
    from finite_status_runtime_incident import collect as runtime_incident, fleet_inventory
    collectors = {"runtime-incident": runtime_incident,
                  "runtime-fleet": fleet_inventory,
                  "retirement-compute": retirement_compute}
    action = request.get("action")
    if action not in collectors:
        raise ValueError("unsupported resource evidence action")
    return {"schema_version": "finite.resource-usage.v1",
            "generated_at": status.isoformat(status.utc_now()),
            "exit_code": 0, **collectors[action](request)}
