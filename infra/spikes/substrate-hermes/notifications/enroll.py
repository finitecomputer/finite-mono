"""Runs INSIDE the actor. Read-only live DB access is a spike integration seam.
Only notification credentials leave this process. No DB writes or chat reads.
Production should replace schema inspection with a SimpleX library hook.
"""
import hashlib,json,os,sqlite3,time,urllib.request
from pathlib import Path
from cryptography.hazmat.primitives import serialization as ser
from smp import SMP,Ed25519PrivateKey,X25519PrivateKey,b64,un64,short,take,der_private,der_public

def enroll(db_path,state_path):
    db=sqlite3.connect('file:'+str(db_path)+'?mode=ro',uri=True)
    db.row_factory=sqlite3.Row
    rows=db.execute('SELECT q.host,q.port,q.rcv_id,q.rcv_private_key,q.ntf_private_key,s.key_hash FROM rcv_queues q JOIN servers s USING(host,port) WHERE q.deleted = 0').fetchall()
    db.close()
    state=json.loads(state_path.read_text()) if state_path.exists() else {}
    result=[]; current=set()
    for row in rows:
        ident=hashlib.sha256(row['host'].encode()+row['port'].encode()+row['rcv_id']).hexdigest();current.add(ident)
        if row['ntf_private_key'] is not None:
            raise RuntimeError('Native notification credentials already exist; refusing to replace them')
        if ident not in state:
            key=Ed25519PrivateKey.generate();meta=X25519PrivateKey.generate()
            smp=SMP(row['host'],row['port'],row['key_hash'])
            try:
                reply=smp.command(row['rcv_id'],b'NKEY '+short(der_public(key))+short(der_public(meta)),ser.load_der_private_key(row['rcv_private_key'],None))
                if not reply.startswith(b'NID '):raise RuntimeError('NKEY failed: '+reply[:30].decode(errors='replace'))
                nid,rest=take(reply[4:]);server_meta,rest=take(rest)
                # Metadata decryption key stays exclusively in the actor.
                state[ident]={'host':row['host'],'port':row['port'],'key_hash':row['key_hash'].decode(),
                  'notifier_id':b64(nid),'notifier_key':b64(der_private(key)),
                  'metadata_private_key':b64(der_private(meta)),'metadata_server_key':b64(server_meta)}
                state_path.parent.mkdir(parents=True,exist_ok=True)
                tmp=state_path.with_suffix('.tmp');tmp.write_text(json.dumps(state));os.chmod(tmp,0o600);tmp.replace(state_path)
            finally:smp.close()
        result.append({k:v for k,v in state[ident].items() if k in ('host','port','key_hash','notifier_id','notifier_key')})
    return result

if __name__=='__main__':
    home=Path('/home/agent');last=None
    while True:
        try:
            rows=enroll(home/'simplex/identity_agent.db',home/'simplex/notification-enrollment.json')
            payload=json.dumps({'subscriptions':rows}).encode()
            req=urllib.request.Request(os.environ['SPIKE_NOTIFY_URL']+'/register',data=payload,
                headers={'Authorization':'Bearer '+os.environ['SPIKE_NOTIFY_TOKEN'],'Content-Type':'application/json'})
            with urllib.request.urlopen(req,timeout=20) as response: response.read()
            if len(rows)!=last:print('Notification registrations:',len(rows),flush=True);last=len(rows)
        except Exception as e:print('Notification enrollment retry:',type(e).__name__,str(e)[:100],flush=True)
        time.sleep(10)
