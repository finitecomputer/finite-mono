"""Exercise actual Substrate + native Hermes, without mocking either service."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import uuid
from probe import Client

p=argparse.ArgumentParser()
p.add_argument('--state-dir', type=Path, required=True)
p.add_argument('--port', type=int, default=18080)
p.add_argument('--owners',type=Path,default=Path(__file__).with_name('owners.json'))
p.add_argument('--output', type=Path, required=True)
a=p.parse_args()
a.output.parent.mkdir(parents=True, exist_ok=True)
owners=json.loads(a.owners.read_text())
clients=[Client(o,a.state_dir,a.port) for o in owners]
report={'checks':[], 'model_chat':'not tested by this transport/lifecycle probe',
        'simplex_message_wake':'not tested'}

def passed(name, **details):
    report['checks'].append({'name':name,'passed':True,**details})
    print(name, 'PASS', flush=True)
    a.output.write_text(json.dumps(report,indent=2)+'\n')

def shell(c, command):
    with c.ws() as ws:
        r=c.rpc(ws,'shell.exec',{'command':command})
        assert r['code']==0,r
        return r['stdout'].strip()

def simplex(c):
    # Real RFC6455 handshake against the local daemon; no model or exec-script
    # bypass is needed. Curl times out after a successful open connection.
    command = "curl --max-time 1 -si --http1.1 -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' http://127.0.0.1:5225/"
    with c.ws() as ws:
        r=c.rpc(ws,'shell.exec',{'command':command})
        assert r['code'] in (0,28) and '101' in r['stdout'].splitlines()[0], r

for c in clients:
    anon=c.http.get(c.base+'/api/auth/me',timeout=180)
    assert anon.status_code==401,anon.status_code
    c.login()
    assert c.http.get(c.base+'/api/auth/me',timeout=30).status_code==200
    other=next(o for o in owners if o!=c.owner)
    assert c.http.get(c.base+'/api/auth/me',headers={'Host':other['host']},timeout=180).status_code==401
    assert c.http.get(c.base+'/api/auth/me',headers={'ate-target-actor':'finite-hermes-spike/'+other['agent_id']},timeout=30).status_code==200
    with c.ws() as ws:
        c.rpc(ws,'session.list',{})
    marker=uuid.uuid4().hex
    shell(c, f"printf {marker} > /home/agent/proof-marker; DISPLAY=:99 scrot /home/agent/desktop-before.png")
    c.marker=marker
    simplex(c)
    image=c.http.get(c.base+'/api/files/download',params={'path':'/home/agent/desktop-before.png'},timeout=30)
    image.raise_for_status()
    assert image.content.startswith(b'\x89PNG\r\n\x1a\n')
    (a.output.parent/(c.owner['user_id']+'-desktop.png')).write_bytes(image.content)
    passed(c.owner['user_id']+' native auth, isolation, WS, shell and desktop screenshot')

alice,bob=clients
# A newly created, unused session lives only in Hermes's in-memory session
# registry. The active-list API explicitly excludes historical DB sessions.
with alice.ws() as ws:
    created=alice.rpc(ws,'session.create',{'title':'RAM-only restore proof'})
    sid=created['session_id']
    before=alice.rpc(ws,'session.active_list',{})
    before_row=next(row for row in before['sessions'] if row['id']==sid)
cli=os.environ.get('ATE_CLI','kubectl-ate')
base=[cli,'--context',os.environ.get('SPIKE_KUBE_CONTEXT','kind-finite-hermes-spike')]
started=time.monotonic()
subprocess.run(base+['suspend','actor',alice.owner['agent_id'],'-a','finite-hermes-spike'],check=True,timeout=180)
suspend_s=time.monotonic()-started
state=subprocess.check_output(base+['get','actor',alice.owner['agent_id'],'-a','finite-hermes-spike','-o','json'],text=True)
assert 'ACTOR_STATE_SUSPENDED' in state,state
passed('provider confirms Alice suspended',seconds=round(suspend_s,3))
assert shell(bob,'cat /home/agent/proof-marker')==bob.marker
passed('Bob remains available while Alice sleeps')
time.sleep(3)
started=time.monotonic()
assert shell(alice,'cat /home/agent/proof-marker')==alice.marker
wake_s=time.monotonic()-started
with alice.ws() as ws:
    after=alice.rpc(ws,'session.active_list',{})
    after_row=next(row for row in after['sessions'] if row['id']==sid)
    assert before_row['started_at']==after_row['started_at']
shell(alice,'DISPLAY=:99 scrot /home/agent/desktop-after.png')
simplex(alice)
passed('URL wakes Alice; disk, live in-memory Hermes session, SimpleX and desktop survive',seconds=round(wake_s,3))
