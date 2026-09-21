"""Real Hermes native-auth + JSON-RPC probe; never prints credentials."""
import argparse
from collections import deque
import json
from pathlib import Path
import socket
import time
import requests
from websockets.sync.client import connect

class Client:
    def __init__(self, owner, state, port):
        self.owner = owner
        self.port = port
        self.base = f'http://127.0.0.1:{port}'
        self.http = requests.Session()
        self.http.trust_env = False
        self.http.headers['Host'] = owner['host']
        self.creds = json.loads((state / f"{owner['agent_id']}.credentials.json").read_text())
        self.seq = 0
        self.events = deque()

    def login(self):
        response = self.http.post(self.base + '/auth/password-login', json={
            'provider':'basic','username':self.creds['username'], 'password':self.creds['password']}, timeout=180)
        response.raise_for_status()
        token = next(c.value for c in response.cookies if 'hermes_session_at' in c.name)
        self.http.headers['Authorization'] = 'Bearer ' + token

    def ws(self):
        self.events.clear()
        r = self.http.post(self.base+'/api/auth/ws-ticket', timeout=30)
        r.raise_for_status()
        return connect('ws://'+self.owner['host']+'/api/ws',
            sock=socket.create_connection(('127.0.0.1',self.port)),
            origin='https://'+self.owner['host'],
            subprotocols=['hermes-gateway-v1','hermes-gateway-ticket.'+r.json()['ticket']],
            open_timeout=180, max_size=16*1024*1024)

    def rpc(self, ws, method, params):
        self.seq += 1
        rid = str(self.seq)
        ws.send(json.dumps({'jsonrpc':'2.0','id':rid,'method':method,'params':params}))
        deadline = time.monotonic()+180
        while time.monotonic()<deadline:
            frame=json.loads(ws.recv(timeout=max(1,deadline-time.monotonic())))
            if frame.get('id')==rid:
                if 'error' in frame:
                    raise RuntimeError(f'{method} failed: {frame["error"]}')
                return frame['result']
            if frame.get('method') == 'event':
                self.events.append(frame)
        raise TimeoutError(method)

if __name__=='__main__':
    p=argparse.ArgumentParser()
    p.add_argument('--state-dir',type=Path,required=True)
    p.add_argument('--port',type=int,default=18080)
    p.add_argument('--user',choices=['alice','bob'],default='alice')
    p.add_argument('--prompt')
    a=p.parse_args()
    owners=json.loads(Path(__file__).with_name('owners.json').read_text())
    owner=next(o for o in owners if o['user_id']==a.user)
    c=Client(owner,a.state_dir,a.port)
    anonymous=c.http.get(c.base+'/api/auth/me',timeout=180)
    assert anonymous.status_code==401, f'anonymous status {anonymous.status_code}'
    c.login()
    me=c.http.get(c.base+'/api/auth/me',timeout=30)
    me.raise_for_status()
    other=next(o for o in owners if o!=owner)
    cross=c.http.get(c.base+'/api/auth/me',headers={'Host':other['host']},timeout=180)
    assert cross.status_code==401, f'cross-owner status {cross.status_code}'
    # Untrusted actor selection must be overwritten by the fixed ingress route.
    injected=c.http.get(c.base+'/api/auth/me',headers={'ate-target-actor':'finite-hermes-spike/'+other['agent_id']},timeout=30)
    injected.raise_for_status()
    with c.ws() as ws:
        sessions=c.rpc(ws,'session.list',{})
        print(json.dumps({'user':a.user,'native_auth':'passed','cross_owner':'denied',
                          'routing_header_override':'passed','websocket':'connected','sessions':sessions}))
        if a.prompt:
            session=c.rpc(ws,'session.create',{'title':'Substrate lifecycle proof','cwd':'/home/agent'})
            print(json.dumps({'created':session}))
            sid=session.get('session_id') or session.get('id')
            if not sid:
                raise RuntimeError('session.create returned no session id')
            result=c.rpc(ws,'prompt.submit',{'session_id':sid,'text':a.prompt})
            print(json.dumps({'submitted':result}))
            deadline=time.monotonic()+240
            while time.monotonic()<deadline:
                frame=c.events.popleft() if c.events else json.loads(ws.recv(timeout=max(1,deadline-time.monotonic())))
                params=frame.get('params',{})
                if params.get('session_id') != sid:
                    continue
                event=params.get('event') or params.get('type')
                if event in ('message.complete','error'):
                    print(json.dumps(frame))
                    payload=params.get('payload',{})
                    if event=='error' or payload.get('status') in ('error','interrupted') or payload.get('error'):
                        raise RuntimeError('native model turn failed')
                    if not payload.get('text','').strip():
                        raise RuntimeError('native model turn returned no text')
                    break
            else:
                raise TimeoutError('model turn did not finish')
