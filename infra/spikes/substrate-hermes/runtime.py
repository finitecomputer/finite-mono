"""Only process startup: Substrate owns scheduling, checkpointing and routing."""
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import time

home = Path('/home/agent')
hermes = home / '.hermes'
hermes.mkdir(parents=True, exist_ok=True)
os.environ.update(HOME=str(home), HERMES_HOME=str(hermes), DISPLAY=':99',
                  HERMES_TUI_WS_ORPHAN_REAP_GRACE_S='0')
config = hermes / 'config.yaml'
if not config.exists():
    config.write_text(json.dumps({'model': {
        'default': os.environ.get('FINITE_PRIVATE_MODEL', 'glm-5-3-flash'),
        'provider': 'custom',
        'base_url': os.environ.get('FINITE_PRIVATE_BASE_URL', 'https://finite-private.finite.containers.tinfoil.dev/v1'),
        'api_key': '${FINITE_PRIVATE_API_KEY}', 'api_mode': 'chat_completions'},
        'terminal': {'backend': 'local', 'cwd': str(home)},
    }))
if os.environ.get('SPIKE_LATENCY_MODE') and not (home/'latency-mode.json').exists():
    (home/'latency-mode.json').write_text(os.environ['SPIKE_LATENCY_MODE'])
children = []
required = []
def start(args, critical=True):
    p = subprocess.Popen(args, start_new_session=True)
    children.append(p)
    if critical:
        required.append(p)
    return p

def stop(*_):
    for p in children:
        if p.poll() is None:
            os.killpg(p.pid, signal.SIGTERM)
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline and any(p.poll() is None for p in children):
        time.sleep(.1)
    for p in children:
        if p.poll() is None:
            os.killpg(p.pid, signal.SIGKILL)
        p.wait()
    raise SystemExit(0)

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
start(['Xvfb', ':99', '-screen', '0', '1280x800x24', '-nolisten', 'tcp'])
for _ in range(100):
    if subprocess.run(['xdpyinfo'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
        break
    time.sleep(.1)
else:
    raise RuntimeError('X11 desktop failed to start')
start(['openbox'])
start(['xmessage', '-center', '-title', 'Finite agent desktop', 'Persistent Hermes desktop'], critical=False)
if os.environ.get('SIMPLEX_WS_URL'):
    start(['python3', '/opt/spike/gateway-start.py'])
else:
    simplex = home / 'simplex'
    simplex.mkdir(exist_ok=True)
    args = ['simplex-chat', '-d', str(simplex / 'identity'), '-p', '5225', '--mute']
    if not (simplex / 'identity_chat.db').exists():
        args += ['--user-display-name', os.environ['SPIKE_AGENT_ID']]
    start(args)
    for _ in range(600):
        try:
            with socket.create_connection(('127.0.0.1', 5225), timeout=.2):
                break
        except OSError:
            time.sleep(.1)
    else:
        raise RuntimeError('SimpleX daemon failed to become ready')
    os.environ['SIMPLEX_WS_URL'] = 'ws://127.0.0.1:5225'
    if os.environ.get('SPIKE_NOTIFY_URL'):
        start(['python3', '/opt/spike/notifications/enroll.py'])
    # No allow-all: owners pair through Hermes's native SimpleX pairing flow.
    start(['hermes', 'gateway', 'run'])
start(['hermes', 'serve', '--isolated', '--host', '0.0.0.0', '--port', '80', '--no-open'])
while True:
    for p in required:
        if p.poll() is not None:
            print(f'child exited: {p.args[0]} ({p.returncode})', flush=True)
            stop()
    time.sleep(1)
