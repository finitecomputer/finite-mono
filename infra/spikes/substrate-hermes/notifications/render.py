"""Private fixture renderer; reuses the local baseline templates, never old identities."""
import argparse,json,os,secrets
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--state-dir',type=Path,required=True);p.add_argument('--image',required=True);p.add_argument('--key-file',type=Path,required=True);p.add_argument('--suffix',default='notify');p.add_argument('--service',default='simplex-notifications');a=p.parse_args()
ns='finite-hermes-spike';name=a.service;owners={}
def write(name,obj):
 with os.fdopen(os.open(a.state_dir/name,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600),'w') as f:json.dump(obj,f,indent=2)
config_path=a.state_dir/'notification-owners.json'
old=json.loads(config_path.read_text()) if config_path.exists() else {}
for user in ('alice','bob'):
 actor=user+'-'+a.suffix;token=old.get(user,{}).get('token') or secrets.token_urlsafe(32)
 owners[user]={'token':token,'actor':ns+'/'+actor,'wake_url':'http://atenet-router.ate-system.svc.cluster.local/api/status'}
 template=json.loads((a.state_dir/(user+'-agent.template.json')).read_text());template['metadata']['name']=actor
 container=template['containers'][0];container['image']=a.image
 env={x['name']:x['value'] for x in container['env']}
 env.update(SPIKE_AGENT_ID=actor,SPIKE_NOTIFY_URL='http://'+name+'.'+ns+'.svc.cluster.local:8766',SPIKE_NOTIFY_TOKEN=token,FINITE_PRIVATE_API_KEY=a.key_file.expanduser().read_text().strip())
 container['env']=[{'name':k,'value':v} for k,v in env.items()]
 write(actor+'.template.json',template)
 write(actor+'.credentials.json',json.loads((a.state_dir/(user+'-agent.credentials.json')).read_text()))
write(config_path.name,owners)
write('owners.json',[{'user_id':u,'agent_id':u+'-'+a.suffix,'host':u+'.agents.test'} for u in ('alice','bob')])
items=[{'apiVersion':'v1','kind':'Secret','metadata':{'name':name,'namespace':ns},'stringData':{'owners.json':json.dumps(owners)}},
 {'apiVersion':'v1','kind':'PersistentVolumeClaim','metadata':{'name':name,'namespace':ns},'spec':{'accessModes':['ReadWriteOnce'],'resources':{'requests':{'storage':'1Gi'}}}},
 {'apiVersion':'apps/v1','kind':'Deployment','metadata':{'name':name,'namespace':ns},'spec':{'replicas':1,'strategy':{'type':'Recreate'},'selector':{'matchLabels':{'app':name}},'template':{'metadata':{'labels':{'app':name}},'spec':{'automountServiceAccountToken':False,'containers':[{'name':'notifications','image':a.image,'command':['python3','/opt/spike/notifications/listener.py'],'env':[{'name':'SPIKE_NOTIFY_OWNERS','value':'/config/owners.json'}],'ports':[{'containerPort':8766}],'readinessProbe':{'httpGet':{'path':'/health','port':8766}},'volumeMounts':[{'name':'config','mountPath':'/config','readOnly':True},{'name':'data','mountPath':'/data'}],'resources':{'requests':{'cpu':'20m','memory':'64Mi'},'limits':{'memory':'192Mi'}}}],'volumes':[{'name':'config','secret':{'secretName':name}},{'name':'data','persistentVolumeClaim':{'claimName':name}}]}}}},
 {'apiVersion':'v1','kind':'Service','metadata':{'name':name,'namespace':ns},'spec':{'selector':{'app':name},'ports':[{'port':8766}]}}]
write('notification-service.json',{'apiVersion':'v1','kind':'List','items':items})
ingress=json.loads((a.state_dir/'ingress.json').read_text())
for item in ingress['items']:
 if item['kind']=='ConfigMap':item['data']['nginx.conf']=item['data']['nginx.conf'].replace('/alice-agent','/alice-'+a.suffix).replace('/bob-agent','/bob-'+a.suffix)
write('notification-ingress.json',ingress)
print('Private notification fixtures rendered')
