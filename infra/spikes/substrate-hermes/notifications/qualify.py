"""Real SimpleX user -> opaque notification -> sleeping Hermes -> real reply.

During each sleep test this harness only sends/polls the HUMAN endpoint and
reads provider state. It makes no request to the sleeping actor's URL.
"""
import argparse,json,os,re,shlex,subprocess,time,uuid
from pathlib import Path
import sys
sys.path.insert(0,str(Path(__file__).resolve().parent.parent))
from probe import Client

p=argparse.ArgumentParser()
p.add_argument('--state-dir',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
p.add_argument('--user',choices=['alice','bob'],default='alice')
p.add_argument('--human-container',default='finite-simplex-test-user')
p.add_argument('--owners',type=Path,default=Path(__file__).with_name('owners.json'))
p.add_argument('--bootstrap-only',action='store_true')
p.add_argument('--rotate',action='store_true',help='Rotate Alice/Bob receive queue before wake tests')
p.add_argument('--paired',action='store_true',help='The native owner pairing has already been approved')
a=p.parse_args()
owner=next(o for o in json.loads(a.owners.read_text()) if o['user_id']==a.user)
c=Client(owner,a.state_dir,18080)
notify_config=json.loads((a.state_dir/'notification-owners.json').read_text())
token=notify_config[a.user]['token']
cli=os.environ.get('ATE_CLI','kubectl-ate')
base=[cli,'--context','kind-finite-hermes-spike']
report={'checks':[],'wake_source':'SMP NMSG only; no test-harness actor HTTP wake'}
a.output.parent.mkdir(parents=True,exist_ok=True)
def record(name,**details):
 report['checks'].append({'name':name,'passed':True,**details})
 a.output.write_text(json.dumps(report,indent=2)+'\n');print(name,'PASS',flush=True)
def admin(text):
 with c.ws() as ws:
  r=c.rpc(ws,'shell.exec',{'command':'python3 /opt/spike/simplex-command.py '+shlex.quote(text)})
  assert r['code']==0,'SimpleX admin failed'
  return json.loads(r['stdout'])
def notify_status():
 import requests
 r=requests.get('http://127.0.0.1:18767/status',headers={'Authorization':'Bearer '+token},timeout=10)
 r.raise_for_status();return r.json()
def human(text):
 return json.loads(subprocess.check_output(['docker','exec',a.human_container,'python3','/home/agent/simplex-command.py',text],text=True,timeout=100))
def human_contacts():return human('/contacts').get('contacts',[])
def send(text):
 result=human('/_send @'+str(contact_id)+' json '+json.dumps([{'msgContent':{'type':'text','text':text}}]))
 assert result['type']=='newChatItems',result['type']
def messages():
 result=human(f'/_get chat @{contact_id} count=100')
 assert result['type']=='apiChat',result['type']
 return [(x['meta']['itemId'],x.get('content',{}).get('msgContent',{}).get('text',''))
         for x in result['chat']['chatItems'] if x.get('chatDir',{}).get('type')=='directRcv']
def wait_text(predicate,timeout=180):
 deadline=time.monotonic()+timeout
 while time.monotonic()<deadline:
  for mid,text in messages():
   if mid>after_id and predicate(text):return mid,text
  time.sleep(2)
 raise TimeoutError('No matching SimpleX reply')
def provider_state():
 result=json.loads(subprocess.check_output(base+['get','actor',owner['agent_id'],'-a','finite-hermes-spike','-o','json'],text=True))
 return result['status']['state']

# Bootstrap is allowed to wake the actor. The later measured cycles are not.
c.login()
contacts=[x for x in human_contacts() if x.get('localDisplayName')==owner['agent_id']]
if not contacts:
 r=admin('/_show_address 1')
 if r['type']=='chatCmdError':r=admin('/_address 1')
 if r['type']=='userContactLinkCreated':link=r['connLinkContact']['connFullLink']
 else:
  contact_link=r['contactLink']
  link=contact_link.get('connLinkContact',{}).get('connFullLink') or contact_link.get('connReqContact')
 if not link:raise RuntimeError('Unknown SimpleX address schema')
 human('/connect '+link)
 for _ in range(90):
  contacts=[x for x in human_contacts() if x.get('localDisplayName')==owner['agent_id']]
  if contacts:break
  time.sleep(2)
 assert contacts,'Contact connection timed out'
contact_id=contacts[0]['contactId']
for _ in range(60):
 ready=[x for x in human_contacts() if x['contactId']==contact_id and x.get('activeConn',{}).get('connStatus',{}).get('type')=='ready']
 if ready:break
 time.sleep(1)
assert ready,'Contact handshake is not ready'
record('Real SimpleX contact connected')
if not a.paired:
 existing=[t for _,t in messages() if 'pairing approve simplex' in t]
 if existing:text=existing[-1]
 else:
  after_id=max([i for i,_ in messages()]+[0])
  send('Hello, please give me the Hermes pairing code.')
  _,text=wait_text(lambda t: 'pairing approve' in t)
 code=re.search(r'pairing approve simplex ([A-Za-z0-9-]+)',text).group(1)
 with c.ws() as ws:
  result=c.rpc(ws,'shell.exec',{'command':'hermes pairing approve simplex '+shlex.quote(code)})
  assert result['code']==0,'Native pairing approval failed'
 record('Native Hermes pairing approval; no allow-all setting')
if a.rotate:
 def registration_digest():
  code="import sqlite3,json,hashlib;d=sqlite3.connect('file:/data/notifications.db?mode=ro',uri=True);p=d.execute('SELECT payload FROM registrations WHERE owner=?',("+repr(a.user)+",)).fetchone()[0];r=json.loads(p);print(json.dumps({'count':len(r),'digest':hashlib.sha256(p.encode()).hexdigest()}))"
  return json.loads(subprocess.check_output(['kubectl','--context','kind-finite-hermes-spike','-n','finite-hermes-spike','exec','deployment/simplex-notifications','--','python3','-c',code],text=True))
 before=registration_digest()
 own_contacts=admin('/contacts')['contacts']
 human_name='SpikeHuman' if a.user=='alice' else 'SpikeBob'
 own_contact=next(x for x in own_contacts if x['localDisplayName']==human_name)
 result=admin('/_switch @'+str(own_contact['contactId']))
 assert result['type']!='chatCmdError',result['type']
 for _ in range(90):
  after=registration_digest()
  if after['digest']!=before['digest'] and after['count']==before['count']:break
  time.sleep(2)
 else:raise TimeoutError('Notification registrations did not follow receive-queue rotation')
 record('Native receive-queue rotation replaces shared notification registration',before=before,after=after)
for _ in range(60):
 if notify_status()['active']>0:break
 time.sleep(2)
assert notify_status()['active']>0,'No active notification subscriptions'
record('Shared listener has active notification-only subscription',**notify_status())
if a.bootstrap_only:raise SystemExit(0)
word='simplexproof' +uuid.uuid4().hex[:12]
after_id=max(i for i,_ in messages())
send(f'Remember this verification word: {word}. Reply with only that word. Do not use tools.')
_,reply=wait_text(lambda t:word in t)
record('Real model reply over SimpleX before sleep',reply=reply)
for cycle in range(2):
 after_id=max(i for i,_ in messages())
 subprocess.run(base+['suspend','actor',owner['agent_id'],'-a','finite-hermes-spike'],check=True,timeout=180)
 assert provider_state()=='ACTOR_STATE_SUSPENDED'
 time.sleep(3)
 assert provider_state()=='ACTOR_STATE_SUSPENDED','Actor woke with no user message'
 started=time.monotonic()
 send('What verification word did I give you earlier? Reply with only that word. Do not use tools.')
 _,reply=wait_text(lambda t:word in t,timeout=240)
 assert provider_state()=='ACTOR_STATE_RUNNING'
 record('SimpleX message automatically wakes suspended actor and preserves conversation',cycle=cycle+1,seconds=round(time.monotonic()-started,3),reply=reply)
