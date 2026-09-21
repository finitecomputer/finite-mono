"""Optional external SimpleX transport variant; private manifests outside git."""
import argparse,json,os,secrets
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--state-dir',type=Path,required=True)
p.add_argument('--image',required=True)
p.add_argument('--key-file',type=Path,required=True)
a=p.parse_args()
ns='finite-hermes-spike'
key=a.key_file.expanduser().read_text().strip()
items=[]
def write(name,obj):
 path=a.state_dir/name
 fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600)
 with os.fdopen(fd,'w') as f: json.dump(obj,f,indent=2)
for user in ('alice','bob'):
 name='simplex-'+user
 write(user+'-simplex.credentials.json',json.loads((a.state_dir/(user+'-agent.credentials.json')).read_text()))
 tokenpath=a.state_dir/(name+'.token')
 if tokenpath.exists(): token=tokenpath.read_text()
 else:
  token=secrets.token_urlsafe(32)
  fd=os.open(tokenpath,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
  with os.fdopen(fd,'w') as f:f.write(token)
 items.extend([
  {'apiVersion':'v1','kind':'Secret','metadata':{'name':name,'namespace':ns},'stringData':{'token':token}},
  {'apiVersion':'v1','kind':'PersistentVolumeClaim','metadata':{'name':name,'namespace':ns},'spec':{'accessModes':['ReadWriteOnce'],'resources':{'requests':{'storage':'1Gi'}}}},
  {'apiVersion':'apps/v1','kind':'Deployment','metadata':{'name':name,'namespace':ns},'spec':{
   'replicas':1,'strategy':{'type':'Recreate'},'selector':{'matchLabels':{'app':name}},'template':{
    'metadata':{'labels':{'app':name}},'spec':{'containers':[
     {'name':'simplex','image':a.image,'command':['simplex-chat','-d','/data/identity','-p','5225','--mute','--user-display-name',user+'-doorbell'],
      'volumeMounts':[{'name':'data','mountPath':'/data'}],'resources':{'requests':{'cpu':'50m','memory':'128Mi'},'limits':{'memory':'384Mi'}}},
     {'name':'relay','image':a.image,'command':['python3','/opt/spike/simplex-relay.py'],
      'env':[{'name':'SIMPLEX_WS_TOKEN','valueFrom':{'secretKeyRef':{'name':name,'key':'token'}}},
             {'name':'SPIKE_WAKE_URL','value':'http://atenet-router.ate-system.svc.cluster.local/api/status'},
             {'name':'SPIKE_TARGET_ACTOR','value':ns+'/'+user+'-simplex'}],
      'ports':[{'containerPort':8765}], 'readinessProbe':{'tcpSocket':{'port':8765}},
      'volumeMounts':[{'name':'data','mountPath':'/data'}],'resources':{'requests':{'cpu':'20m','memory':'64Mi'},'limits':{'memory':'128Mi'}}}],
     'volumes':[{'name':'data','persistentVolumeClaim':{'claimName':name}}]}}}},
  {'apiVersion':'v1','kind':'Service','metadata':{'name':name,'namespace':ns},'spec':{'selector':{'app':name},'ports':[{'port':8765}]}}
 ])
 template=json.loads((a.state_dir/(user+'-agent.template.json')).read_text())
 template['metadata']['name']=user+'-simplex'
 container=template['containers'][0];container['image']=a.image
 env={x['name']:x['value'] for x in container['env']}
 env.update(SPIKE_AGENT_ID=user+'-simplex',SIMPLEX_WS_URL='ws://'+name+'.'+ns+'.svc.cluster.local:8765',SIMPLEX_WS_TOKEN=token,FINITE_PRIVATE_API_KEY=key)
 container['env']=[{'name':k,'value':v} for k,v in env.items()]
 write(user+'-simplex.template.json',template)
write('simplex-transports.json',{'apiVersion':'v1','kind':'List','items':items})
ingress=json.loads((a.state_dir/'ingress.json').read_text())
for item in ingress['items']:
 if item['kind']=='ConfigMap':
  item['data']['nginx.conf']=item['data']['nginx.conf'].replace('/alice-agent','/alice-simplex').replace('/bob-agent','/bob-simplex')
write('simplex-ingress.json',ingress)
print('Rendered private transport and actor fixtures')
