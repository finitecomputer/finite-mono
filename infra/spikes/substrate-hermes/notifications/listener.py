"""Shared notification-only listener. No chat identities or decryption keys.

One SMP connection per server, multiplexing all owners' notification queues.
Durable pending wake intent; configuration is owner-token scoped. Only fixed
operator-supplied actor destinations are accepted, never a client wake URL.
"""
import hashlib,hmac,json,os,socket,sqlite3,threading,time,urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
from cryptography.hazmat.primitives import serialization as ser
from smp import SMP,Ed25519PrivateKey,un64

ROOT=Path(os.environ.get('SPIKE_NOTIFY_STATE','/data'));ROOT.mkdir(parents=True,exist_ok=True)
OWNERS=json.loads(Path(os.environ['SPIKE_NOTIFY_OWNERS']).read_text())
LOCK=threading.RLock();DB=sqlite3.connect(ROOT/'notifications.db',check_same_thread=False)
DB.executescript('CREATE TABLE IF NOT EXISTS registrations (owner TEXT PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS pending (owner TEXT PRIMARY KEY, since REAL NOT NULL); CREATE TABLE IF NOT EXISTS counts (owner TEXT PRIMARY KEY, received INTEGER NOT NULL DEFAULT 0, wakes INTEGER NOT NULL DEFAULT 0);')
REV=0;ACTIVE={};STOP=threading.Event()
def event(owner):
    with LOCK:
        DB.execute('INSERT INTO pending VALUES (?,?) ON CONFLICT(owner) DO UPDATE SET since=excluded.since',(owner,time.time()))
        DB.execute('INSERT INTO counts(owner,received) VALUES (?,1) ON CONFLICT(owner) DO UPDATE SET received=received+1',(owner,));DB.commit()
    print(json.dumps({'notification':owner}),flush=True)
def subscriptions():
    with LOCK:return [(o,json.loads(p)) for o,p in DB.execute('SELECT owner,payload FROM registrations')]
def watch(server,rows,revision):
    while not STOP.is_set():
        with LOCK:
            if REV!=revision:return
        conn=None
        try:
            conn=SMP(*server);mapping={};expected={}
            for owner,row in rows:
                nid=un64(row['notifier_id']);mapping[nid]=owner
                corr=conn.send(nid,b'NSUB',ser.load_der_private_key(un64(row['notifier_key']),None));expected[corr]=nid
            conn.sock.settimeout(30)
            while not STOP.is_set():
                with LOCK:
                    if REV!=revision:return
                try:corr,nid,msg=conn.receive()
                except socket.timeout:
                    conn.send(b'',b'PING');continue
                if msg.startswith(b'NMSG '):event(mapping[nid])
                elif corr in expected:
                    if msg!=b'OK' and not msg.startswith(b'SOK'):raise RuntimeError('NSUB rejected: '+msg[:20].decode(errors='replace'))
                    with LOCK:ACTIVE[(server,nid)]=mapping[nid]
                elif msg.startswith(b'END') or msg.startswith(b'ERR'):raise RuntimeError('Notification subscription ended')
        except Exception as e:print(json.dumps({'subscription_retry':type(e).__name__,'detail':str(e)[:90]}),flush=True)
        finally:
            if conn:conn.close()
            with LOCK:
                for _,row in rows:ACTIVE.pop((server,un64(row['notifier_id'])),None)
        STOP.wait(2)
def supervise():
    last=-1
    while not STOP.wait(1):
        with LOCK:
            revision=REV
            if revision==last:continue
            last=revision
        grouped={}
        for owner,rows in subscriptions():
            for row in rows:grouped.setdefault((row['host'],row['port'],row['key_hash']),[]).append((owner,row))
        # Old readers exit on their next frame/timeout; no per-agent daemon.
        for server,rows in grouped.items():threading.Thread(target=watch,args=(server,rows,revision),daemon=True).start()
def wake():
    while not STOP.wait(1):
        with LOCK:pending=list(DB.execute('SELECT owner,since FROM pending'))
        for owner,since in pending:
            try:
                cfg=OWNERS[owner]
                req=urllib.request.Request(cfg['wake_url'],headers={'ate-target-actor':cfg['actor']})
                with urllib.request.urlopen(req,timeout=120) as response:
                    if response.status!=200:continue
                with LOCK:
                    # A later signal must not be cleared by an earlier wake.
                    DB.execute('DELETE FROM pending WHERE owner=? AND since=?',(owner,since))
                    DB.execute('UPDATE counts SET wakes=wakes+1 WHERE owner=?',(owner,));DB.commit()
                print(json.dumps({'wake':owner}),flush=True)
            except Exception as e:print(json.dumps({'wake_retry':owner,'error':type(e).__name__}),flush=True)
class HTTP(BaseHTTPRequestHandler):
    def log_message(self,*_):pass
    def owner(self):
        auth=self.headers.get('Authorization','')
        return next((o for o,c in OWNERS.items() if hmac.compare_digest(auth,'Bearer '+c['token'])),None)
    def reply(self,code,data):
        body=json.dumps(data).encode();self.send_response(code);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(body)
    def do_GET(self):
        if self.path=='/health':return self.reply(200,{'ok':True})
        owner=self.owner()
        if not owner:return self.reply(401,{'error':'unauthorized'})
        with LOCK:
            counts=DB.execute('SELECT received,wakes FROM counts WHERE owner=?',(owner,)).fetchone()
            self.reply(200,{'active':sum(o==owner for o in ACTIVE.values()),'received':counts[0] if counts else 0,'wakes':counts[1] if counts else 0})
    def do_POST(self):
        global REV
        owner=self.owner()
        if not owner:return self.reply(401,{'error':'unauthorized'})
        if self.path!='/register':return self.reply(404,{})
        try:
            size=int(self.headers.get('Content-Length','0'))
            if not 0<size<100000:raise ValueError()
            body=json.loads(self.rfile.read(size));rows=body['subscriptions']
            if set(body)!={'subscriptions'} or len(rows)>128:raise ValueError()
            for row in rows:
                if set(row)!={'host','port','key_hash','notifier_id','notifier_key'}:raise ValueError()
                if not isinstance(ser.load_der_private_key(un64(row['notifier_key']),None),Ed25519PrivateKey):raise ValueError()
                if len(un64(row['key_hash']))!=32 or not un64(row['notifier_id']):raise ValueError()
            payload=json.dumps(rows,sort_keys=True)
            with LOCK:
                old=DB.execute('SELECT payload FROM registrations WHERE owner=?',(owner,)).fetchone()
                if old!=(payload,):
                    DB.execute('INSERT OR REPLACE INTO registrations VALUES (?,?)',(owner,payload));DB.commit();REV+=1
            self.reply(200,{'registered':len(rows)})
        except Exception:self.reply(400,{'error':'invalid notification-only registration'})
for fn in (supervise,wake):threading.Thread(target=fn,daemon=True).start()
ThreadingHTTPServer(('0.0.0.0',8766),HTTP).serve_forever()
