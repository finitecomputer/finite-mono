"""Keep Alice asleep, queue a real message with transport down, restore its PVC."""
import argparse,json,os,subprocess,time
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--previous-proof',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
a=p.parse_args()
word=json.loads(a.previous_proof.read_text())['checks'][-1]['reply']
cli=[os.environ.get('ATE_CLI','kubectl-ate'),'--context','kind-finite-hermes-spike']
kube=['kubectl','--context','kind-finite-hermes-spike','-n','finite-hermes-spike']
def human(cmd):
 return json.loads(subprocess.check_output(['docker','exec','finite-simplex-test-user','python3','/home/agent/simplex-command.py',cmd],text=True,timeout=90))
contact=next(c for c in human('/contacts')['contacts'] if c['localDisplayName']=='alice-doorbell')
cid=contact['contactId']
def messages():
 return human(f'/_get chat @{cid} count=100')['chat']['chatItems']
def state():
 return json.loads(subprocess.check_output(cli+['get','actor','alice-simplex','-a','finite-hermes-spike','-o','json'],text=True))['status']['state']
after_id=max(x['meta']['itemId'] for x in messages())
subprocess.run(cli+['suspend','actor','alice-simplex','-a','finite-hermes-spike'],check=True,timeout=180)
subprocess.run(kube+['scale','deployment/simplex-alice','--replicas=0'],check=True)
try:
 subprocess.run(kube+['wait','--for=delete','pod','-l','app=simplex-alice','--timeout=60s'],check=True,timeout=65)
 assert state()=='ACTOR_STATE_SUSPENDED'
 r=human('/_send @'+str(cid)+' json '+json.dumps([{'msgContent':{'type':'text','text':'What verification word did I give you earlier? Reply with only that word. Do not use tools.'}}]))
 assert r['type']=='newChatItems'
 time.sleep(3)
 assert state()=='ACTOR_STATE_SUSPENDED'
 print('Message queued while both actor and transport were unavailable',flush=True)
finally:
 subprocess.run(kube+['scale','deployment/simplex-alice','--replicas=1'],check=True)
started=time.monotonic()
deadline=started+180
while time.monotonic()<deadline:
 replies=[x.get('content',{}).get('msgContent',{}).get('text','') for x in messages()
          if x['meta']['itemId']>after_id and x.get('chatDir',{}).get('type')=='directRcv']
 if any(word in text for text in replies):break
 time.sleep(2)
else:raise TimeoutError('No model reply after transport restart')
assert state()=='ACTOR_STATE_RUNNING'
report={'passed':True,'check':'Persistent transport identity restored; queued SimpleX message automatically woke actor; model remembered conversation','seconds_from_transport_restart':round(time.monotonic()-started,3),'reply':next(t for t in replies if word in t),'actor_http_requests_by_harness':0}
a.output.write_text(json.dumps(report,indent=2)+'\n')
print(report['check'],'PASS',flush=True)
