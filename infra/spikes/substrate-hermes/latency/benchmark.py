"""Repeat warm/sleep real-model turns; measure events, never poll actor to wake it."""
import argparse,json,os,subprocess,sys,time,uuid
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent.parent))
from probe import Client
p=argparse.ArgumentParser();p.add_argument('--state',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--user',default='alice');p.add_argument('--cycles',type=int,default=5);p.add_argument('--warm',type=int,default=2);p.add_argument('--service',default='simplex-latency');p.add_argument('--owners',type=Path,required=True);p.add_argument('--label',required=True);p.add_argument('--long',action='store_true');p.add_argument('--prepare-simplex',action='store_true');p.add_argument('--idle-seconds',type=float,default=3);a=p.parse_args()
owner=next(o for o in json.loads(a.owners.read_text()) if o['user_id']==a.user)
c=Client(owner,a.state/'fixtures',18080);c.login()
human='finite-simplex-test-user' if a.user=='alice' else 'finite-simplex-test-bob'
cli=[os.environ['ATE_CLI'],'--context','kind-finite-hermes-spike']
def state():return json.loads(subprocess.check_output(cli+['get','actor',owner['agent_id'],'-a','finite-hermes-spike','-o','json'],text=True))['status']['state']
contacts=json.loads(subprocess.check_output(['docker','exec',human,'python3','/home/agent/simplex-command.py','/contacts'],text=True))['contacts']
cid=next(x['contactId'] for x in contacts if x['localDisplayName']==owner['agent_id'])
report={'label':a.label,'user':a.user,'trials':[],'sleep_actor_http_requests_by_harness':0,'prepare_simplex':a.prepare_simplex,'idle_seconds':a.idle_seconds}
for index in range(a.warm+a.cycles):
 sleeping=index>=a.warm
 if sleeping:
  if a.prepare_simplex:
   with c.ws() as ws:
    prepared=c.rpc(ws,'shell.exec',{'command':"python3 /opt/spike/simplex-command.py '/_app suspend 0'"})
   assert prepared['code']==0 and json.loads(prepared['stdout'])['type']=='cmdOk'
  subprocess.run(cli+['suspend','actor',owner['agent_id'],'-a','finite-hermes-spike'],check=True,stdout=subprocess.DEVNULL,timeout=180)
  time.sleep(a.idle_seconds);assert state()=='ACTOR_STATE_SUSPENDED'
 marker='ENDRAIN' if a.long else 'latency'+uuid.uuid4().hex[:10]
 prompt=('Write 20 numbered sentences explaining how rain forms, at most 8 words per sentence. Start with RAIN. End with exactly '+marker+'. Do not use tools.') if a.long else ('Reply with exactly '+marker+'. Do not use tools.')
 result=json.loads(subprocess.check_output(['docker','exec','-i',human,'python3','/home/agent/human-turn.py'],input=json.dumps({'contact':cid,'marker':marker,'text':prompt,'long':a.long}),text=True,timeout=210))
 assert state()=='ACTOR_STATE_RUNNING'
 # Read instrumentation only AFTER the human has received the completed reply.
 r=c.http.get(c.base+'/api/files/download',params={'path':'/home/agent/latency.jsonl'},timeout=30);r.raise_for_status()
 traces=[json.loads(line) for line in r.text.splitlines()]
 result['actor']=[e for e in traces if result['send']-.1<=e['t']<=result['complete']+1]
 raw=subprocess.check_output(['kubectl','--context','kind-finite-hermes-spike','-n','finite-hermes-spike','logs','deployment/'+a.service],text=True)
 result['listener']=[]
 for line in raw.splitlines():
  try:e=json.loads(line)
  except ValueError:continue
  if (e.get('notification')==a.user or e.get('wake')==a.user or e.get('wake_start')==a.user) and result['send']-.1<=e.get('t',0)<=result['complete']+1:result['listener'].append(e)
 result.update(sleeping=sleeping,index=index)
 report['trials'].append(result);a.output.write_text(json.dumps(report,indent=2)+'\n')
 print(a.label,a.user,'sleep' if sleeping else 'warm',index,round(result['first_seconds'],3),round(result['complete_seconds'],3),flush=True)
 time.sleep(1)
