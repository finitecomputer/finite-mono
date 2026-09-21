"""Real SimpleX user -> relay -> sleeping Hermes -> real model -> SimpleX user.

During each sleep test this harness only sends/polls the HUMAN endpoint and
reads provider state. It makes no request to the sleeping actor's URL.
"""
import argparse,json,os,re,shlex,subprocess,time,uuid
from pathlib import Path
from websockets.sync.client import connect
from probe import Client

p=argparse.ArgumentParser()
p.add_argument('--state-dir',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
p.add_argument('--user',choices=['alice','bob'],default='alice')
p.add_argument('--relay-port',type=int,default=18765)
p.add_argument('--human-container',default='finite-simplex-test-user')
p.add_argument('--paired',action='store_true',help='The native owner pairing has already been approved')
a=p.parse_args()
owner=next(o for o in json.loads(Path(__file__).with_name('owners-simplex.json').read_text()) if o['user_id']==a.user)
c=Client(owner,a.state_dir,18080)
token=(a.state_dir/('simplex-'+a.user+'.token')).read_text()
cli=os.environ.get('ATE_CLI','kubectl-ate')
base=[cli,'--context','kind-finite-hermes-spike']
report={'checks':[],'wake_source':'real inbound SimpleX event; no test-harness HTTP wake'}
a.output.parent.mkdir(parents=True,exist_ok=True)
def record(name,**details):
 report['checks'].append({'name':name,'passed':True,**details})
 a.output.write_text(json.dumps(report,indent=2)+'\n');print(name,'PASS',flush=True)
def admin(text):
 with connect(f'ws://127.0.0.1:{a.relay_port}/admin',additional_headers={'Authorization':'Bearer '+token}) as ws:
  rid=uuid.uuid4().hex;ws.send(json.dumps({'corrId':rid,'cmd':text}))
  while True:
   r=json.loads(ws.recv(timeout=90))
   if r.get('corrId')==rid:return r['resp']
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
contacts=[x for x in human_contacts() if x.get('localDisplayName')==a.user+'-doorbell']
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
  contacts=[x for x in human_contacts() if x.get('localDisplayName')==a.user+'-doorbell']
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
word='simplexproof'+uuid.uuid4().hex[:12]
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
