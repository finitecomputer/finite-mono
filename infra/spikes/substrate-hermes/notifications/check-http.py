"""Live negative checks against the shared registration boundary."""
import json,os,sqlite3,urllib.request,urllib.error
from pathlib import Path
owners=json.loads(Path(os.environ['SPIKE_NOTIFY_OWNERS']).read_text())
checks=[]
def request(body,token,expected):
 req=urllib.request.Request('http://127.0.0.1:8766/register',data=json.dumps(body).encode(),headers={'Authorization':'Bearer '+token,'Content-Type':'application/json'})
 try:
  with urllib.request.urlopen(req) as r:status=r.status
 except urllib.error.HTTPError as e:status=e.code
 assert status==expected,(status,expected)
 checks.append({'expected':expected,'actual':status})
request({'subscriptions':[]},'invalid',401)
request({'subscriptions':[],'owner':'bob'},owners['alice']['token'],400)
db=sqlite3.connect('file:/data/notifications.db?mode=ro',uri=True)
row=json.loads(db.execute('SELECT payload FROM registrations WHERE owner=?',('alice',)).fetchone()[0])[0]
request({'subscriptions':[dict(row,metadata_private_key='must-not-be-accepted')]},owners['alice']['token'],400)
print(json.dumps({'passed':True,'checks':checks}))
