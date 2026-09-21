"""Live tests of the wake-only credential boundary, without printing credentials."""
import argparse,json
from pathlib import Path
import requests
p=argparse.ArgumentParser();p.add_argument('--state',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
owners=json.loads((a.state/'fixtures/notification-owners.json').read_text())
s=requests.Session();s.trust_env=False
results=[]
for user in ('alice','bob'):
 headers={'Host':user+'.agents.test'};url='http://127.0.0.1:18080'
 for label,token,status in [('invalid','invalid',401),('other-owner',owners['bob' if user=='alice' else 'alice']['token'],401),('valid',owners[user]['token'],200)]:
  r=s.get(url+'/api/status',headers=dict(headers,**{'x-simplex-wake':token}),timeout=30)
  assert r.status_code==status,(user,label,r.status_code)
  results.append({'owner':user,'credential':label,'status':status})
 r=s.get(url+'/api/auth/me',headers=dict(headers,**{'x-simplex-wake':owners[user]['token']}),timeout=30)
 assert r.status_code==401,'Wake credential granted native chat authentication'
 results.append({'owner':user,'wake_credential_grants_native_auth':False})
a.output.write_text(json.dumps({'passed':True,'checks':results},indent=2)+'\n');print('Wake-only owner boundary PASS')
