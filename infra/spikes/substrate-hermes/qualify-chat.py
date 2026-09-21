"""Real two-owner model conversation, including native session resume after sleep."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import uuid
from probe import Client

p=argparse.ArgumentParser()
p.add_argument('--key-file',type=Path,required=True)
p.add_argument('--state-dir',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
p.add_argument('--owners',type=Path,default=Path(__file__).with_name('owners.json'))
a=p.parse_args()
key=a.key_file.expanduser().read_text().strip()
assert key and len(key.split())==1,'Expected a file containing only the API key'
owners=json.loads(a.owners.read_text())
report={'checks':[]}
a.output.parent.mkdir(parents=True,exist_ok=True)
def record(name,**details):
    report['checks'].append({'name':name,'passed':True,**details})
    a.output.write_text(json.dumps(report,indent=2)+'\n')
    print(name,'PASS',flush=True)
def turn(c,ws,sid,text):
    c.rpc(ws,'prompt.submit',{'session_id':sid,'text':text})
    deadline=time.monotonic()+240
    while time.monotonic()<deadline:
        frame=c.events.popleft() if c.events else json.loads(ws.recv(timeout=max(1,deadline-time.monotonic())))
        params=frame.get('params',{})
        if params.get('session_id')!=sid: continue
        if params.get('type')=='message.complete':
            payload=params.get('payload',{})
            if payload.get('status') in ('error','interrupted') or payload.get('error'):
                # Redact the authorized test key from provider diagnostics.
                raise RuntimeError(('Model turn failed: '+str(payload.get('error') or payload.get('text'))).replace(key,'[REDACTED]'))
            assert payload.get('text','').strip(),'Empty assistant reply'
            return payload['text']
    raise TimeoutError('Model reply')
clients=[]
for owner in owners:
    c=Client(owner,a.state_dir,18080)
    c.login()
    r=c.http.put(c.base+'/api/config',json={'config':{'model':{'api_key':key}}},timeout=30)
    r.raise_for_status()
    with c.ws() as ws:
        created=c.rpc(ws,'session.create',{'title':'Real model sleep/wake proof'})
        sid=created['session_id']
        stored_sid=created['stored_session_id']
        word='proof'+uuid.uuid4().hex[:12]
        reply=turn(c,ws,sid,f'Remember this verification word for later: {word}. Reply with only that word. Do not use tools.')
        assert word in reply,'Reply did not include verification word'
    clients.append((c,stored_sid,word))
    record(owner['user_id']+' real model chat through native Hermes WebSocket',reply=reply)
cli=os.environ.get('ATE_CLI','kubectl-ate')
base=[cli,'--context','kind-finite-hermes-spike']
for c,sid,word in clients:
    actor=c.owner['agent_id']
    subprocess.run(base+['suspend','actor',actor,'-a','finite-hermes-spike'],check=True,timeout=180)
    started=time.monotonic()
    with c.ws() as ws:
        resumed=c.rpc(ws,'session.resume',{'session_id':sid})
        resumed_sid=resumed.get('session_id') or resumed.get('id') or sid
        reply=turn(c,ws,resumed_sid,'What verification word did I give you earlier? Reply with only that word. Do not use tools.')
        assert word in reply,'Conversation context was lost'
    record(c.owner['user_id']+' URL wake and model conversation continuity',reply=reply,seconds=round(time.monotonic()-started,3))
