"""Exercise real listener HTTP concurrency, durable retry, and in-flight intent races."""
import json,os,sqlite3,subprocess,tempfile,threading,time
from pathlib import Path
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
arrived={u:threading.Event() for u in ('alice','bob')}
release=threading.Event();second=threading.Event();release_second=threading.Event()
calls={'alice':0,'bob':0};guard=threading.Lock()
class Target(BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_GET(self):
  owner=self.path[1:]
  assert self.headers['x-simplex-wake']=='synthetic-'+owner
  with guard:calls[owner]+=1;n=calls[owner]
  arrived[owner].set()
  if owner=='alice':
   if n==1:release.wait(8)
   else:second.set();release_second.wait(8)
  self.send_response(503 if owner=='bob' and n==1 else 200)
  self.end_headers();self.wfile.write(b'{}')
server=ThreadingHTTPServer(('127.0.0.1',0),Target)
threading.Thread(target=server.serve_forever,daemon=True).start()
with tempfile.TemporaryDirectory() as tmp:
 root=Path(tmp);db=sqlite3.connect(root/'notifications.db')
 db.executescript('CREATE TABLE pending (owner TEXT PRIMARY KEY,since REAL NOT NULL); CREATE TABLE counts(owner TEXT PRIMARY KEY,received INTEGER NOT NULL DEFAULT 0,wakes INTEGER NOT NULL DEFAULT 0);')
 db.executemany('INSERT INTO pending VALUES (?,?)',[(u,time.time()) for u in arrived])
 db.executemany('INSERT INTO counts(owner) VALUES (?)',[(u,) for u in arrived]);db.commit()
 owners={u:{'token':'synthetic-'+u,'actor':u,'wake_url':f'http://127.0.0.1:{server.server_port}/{u}'} for u in arrived}
 cfg=root/'owners.json';cfg.write_text(json.dumps(owners))
 proc=subprocess.Popen(['python3','/opt/spike/notifications/listener.py'],env=dict(os.environ,SPIKE_NOTIFY_STATE=tmp,SPIKE_NOTIFY_OWNERS=str(cfg)),stdout=subprocess.DEVNULL)
 try:
  assert arrived['alice'].wait(3),'First wake never dispatched'
  assert arrived['bob'].wait(1),'Slow owner blocked second owner'
  time.sleep(.1)
  assert db.execute('SELECT count(*) FROM pending').fetchone()[0]==2,'Failed or in-flight intent was cleared'
  # A newer notification arrives while Alice's old HTTP request is in flight.
  newer=time.time();db.execute('UPDATE pending SET since=? WHERE owner=?',(newer,'alice'));db.commit()
  release.set()
  assert second.wait(2),'New intent lost when older HTTP wake completed'
  assert db.execute('SELECT since FROM pending WHERE owner=?',('alice',)).fetchone()==(newer,)
  release_second.set()
  deadline=time.monotonic()+4
  while db.execute('SELECT count(*) FROM pending').fetchone()[0] and time.monotonic()<deadline:time.sleep(.02)
  assert db.execute('SELECT count(*) FROM pending').fetchone()[0]==0
  assert calls=={'alice':2,'bob':2},calls
  print(json.dumps({'passed':True,'slow_owner_does_not_block_other_owner':True,'pending_recovered_at_startup':True,'failed_http_retried':True,'new_intent_survives_old_completion':True,'cleared_only_after_success':True}))
 finally:release.set();release_second.set();proc.kill();proc.wait();server.shutdown()
