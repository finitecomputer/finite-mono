"""Render disposable local fixtures. Credential-bearing output stays outside git."""
import argparse
import json
import os
from pathlib import Path
import secrets

p = argparse.ArgumentParser()
p.add_argument('--image', required=True)
p.add_argument('--worker-image', required=True)
p.add_argument('--version', required=True)
p.add_argument('--snapshot-location', required=True)
p.add_argument('--state-dir', type=Path, required=True)
a = p.parse_args()
a.state_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
os.chmod(a.state_dir, 0o700)
owners = json.loads(Path(__file__).with_name('owners.json').read_text())
ns = 'finite-hermes-spike'
def write(name, obj):
    path = a.state_dir / name
    with open(path, 'w', opener=lambda path, flags: os.open(path, flags, 0o600)) as f:
        f.write(json.dumps(obj, indent=2) + '\n')

write('pool.json', {'apiVersion': 'v1', 'kind': 'List', 'items': [
    {'apiVersion': 'v1', 'kind': 'Namespace', 'metadata': {'name': ns}},
    {'apiVersion': 'ate.dev/v1alpha1', 'kind': 'WorkerPool',
     'metadata': {'name': 'hermes', 'namespace': ns, 'labels': {'workload': ns}},
     'spec': {'replicas': 2, 'workerImage': a.worker_image, 'template': {
         'nodeSelector': {'ate.dev/substrate-version': a.version},
         'resources': {'requests': {'cpu': '250m', 'memory': '2Gi'},
                       'limits': {'cpu': '2', 'memory': '2Gi'}}}}}
]})
servers = []
for owner in owners:
    agent = owner['agent_id']
    creds_path = a.state_dir / f'{agent}.credentials.json'
    if creds_path.exists():
        creds = json.loads(creds_path.read_text())
    else:
        creds = {'username': owner['user_id'], 'password': secrets.token_urlsafe(32),
                 'signing_secret': secrets.token_urlsafe(48)}
        write(creds_path.name, creds)
    env = {'SPIKE_AGENT_ID': agent,
           'HERMES_DASHBOARD_PUBLIC_URL': 'https://' + owner['host'],
           'HERMES_DASHBOARD_BASIC_AUTH_USERNAME': creds['username'],
           'HERMES_DASHBOARD_BASIC_AUTH_PASSWORD': creds['password'],
           'HERMES_DASHBOARD_BASIC_AUTH_SECRET': creds['signing_secret']}
    if os.environ.get('FINITE_PRIVATE_API_KEY'):
        env['FINITE_PRIVATE_API_KEY'] = os.environ['FINITE_PRIVATE_API_KEY']
    for name in ('FINITE_PRIVATE_MODEL', 'FINITE_PRIVATE_BASE_URL'):
        if name in os.environ:
            env[name] = os.environ[name]
    # Separate templates prevent golden snapshots from cloning one owner's
    # initialized chat/connector identity or credentials into another owner.
    write(f'{agent}.template.json', {
        'metadata': {'atespace': ns, 'name': agent},
        'workerSelector': {'matchLabels': {'workload': ns}},
        'containers': [{'name': 'hermes', 'image': a.image,
            'command': ['python3', '/opt/spike/runtime.py'],
            'env': [{'name': k, 'value': v} for k,v in env.items()],
            'readyz': {'httpGet': {'path': '/api/status', 'port': 80}, 'timeoutSeconds': 120},
            'volumeMounts': [{'name': 'home', 'mountPath': '/home/agent'}]}],
        'resources': {'limits': [{'name':'cpu','quantity':'2'}, {'name':'memory','quantity':'2Gi'}]},
        'sandboxConfig': {'sandboxClass': 'SANDBOX_CLASS_GVISOR', 'configName': 'gvisor-default'},
        'snapshotsConfig': {'onPause': 'SNAPSHOT_CONTENT_SCOPE_FULL',
                            'onCommit': 'SNAPSHOT_CONTENT_SCOPE_FULL',
                            'storageLocation': a.snapshot_location},
        'volumes': [{'name': 'home', 'durableDir': {}}]})
    servers.append('''server {
      listen 80;
      server_name %s;
      location / {
        proxy_pass http://atenet-router.ate-system.svc.cluster.local;
        proxy_set_header Host $http_host;
        proxy_set_header ate-target-actor %s/%s;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_http_version 1.1;
        proxy_buffering off;
        proxy_read_timeout 3600s;
      }
    }''' % (owner['host'], ns, agent))
nginx = '''events {}
http {
  access_log off;
  map $http_upgrade $connection_upgrade { default upgrade; '' close; }
  server { listen 80 default_server; server_name _; return 404; }
  %s
}
''' % '\n'.join(servers)
write('ingress.json', {'apiVersion':'v1', 'kind':'List', 'items': [
    {'apiVersion':'v1','kind':'ConfigMap','metadata':{'name':'ingress','namespace':ns}, 'data':{'nginx.conf':nginx}},
    {'apiVersion':'apps/v1','kind':'Deployment','metadata':{'name':'ingress','namespace':ns},'spec':{
        'replicas':1,'selector':{'matchLabels':{'app':'spike-ingress'}},'template':{
            'metadata':{'labels':{'app':'spike-ingress'}},'spec':{'containers':[{
                'name':'nginx','image':'nginx:1.29-alpine','ports':[{'containerPort':80}],
                'volumeMounts':[{'name':'config','mountPath':'/etc/nginx/nginx.conf','subPath':'nginx.conf'}]}],
                'volumes':[{'name':'config','configMap':{'name':'ingress'}}]}}}},
    {'apiVersion':'v1','kind':'Service','metadata':{'name':'ingress','namespace':ns},'spec':{
        'selector':{'app':'spike-ingress'},'ports':[{'port':80,'targetPort':80}]}}
]})
print('Rendered two owner fixtures; credential-bearing manifests are private files in the state directory.')
