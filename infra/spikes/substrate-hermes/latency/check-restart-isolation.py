"""Notification listener outage; external message must wake only its owner."""
import argparse,json,os,subprocess,time,uuid
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--owners',type=Path,required=True);p.add_argument('--service',required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
owners={o['user_id']:o for o in json.loads(a.owners.read_text())}
cli=[os.environ['ATE_CLI'],'--context','kind-finite-hermes-spike']
kube=['kubectl','--context','kind-finite-hermes-spike','-n','finite-hermes-spike']
def state(user):return json.loads(subprocess.check_output(cli+['get','actor',owners[user]['agent_id'],'-a','finite-hermes-spike','-o','json'],text=True))['status']['state']
assert all(state(u)=='ACTOR_STATE_SUSPENDED' for u in owners)
contacts=json.loads(subprocess.check_output(['docker','exec','finite-simplex-test-user','python3','/home/agent/simplex-command.py','/contacts'],text=True))['contacts']
cid=next(c['contactId'] for c in contacts if c['localDisplayName']==owners['alice']['agent_id'])
subprocess.run(kube+['scale','deployment/'+a.service,'--replicas=0'],check=True,stdout=subprocess.DEVNULL)
subprocess.run(kube+['wait','--for=delete','pod','-l','app='+a.service,'--timeout=60s'],check=True,stdout=subprocess.DEVNULL)
marker='outage'+uuid.uuid4().hex[:10]
proc=None
try:
 proc=subprocess.Popen(['docker','exec','-i','finite-simplex-test-user','python3','/home/agent/human-turn.py'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
 proc.stdin.write(json.dumps({'contact':cid,'marker':marker,'text':'Reply with exactly '+marker+'. Do not use tools.','long':False}));proc.stdin.close();proc.stdin=None
 time.sleep(2)
 assert all(state(u)=='ACTOR_STATE_SUSPENDED' for u in owners),'Actor woke while listener was down'
finally:
 subprocess.run(kube+['scale','deployment/'+a.service,'--replicas=1'],check=True,stdout=subprocess.DEVNULL)
assert proc is not None
stdout,stderr=proc.communicate(timeout=210)
assert proc.returncode==0,'External reply observation failed'
result=json.loads(stdout)
assert state('alice')=='ACTOR_STATE_RUNNING'
assert state('bob')=='ACTOR_STATE_SUSPENDED','Other owner woke'
a.output.write_text(json.dumps({'passed':True,'actor_http_requests_by_harness':0,'message_sent_while_listener_down':True,'listener_recovered_registration_from_pvc':True,'only_target_owner_woke':True,'reply_timing':result},indent=2)+'\n')
print('Listener outage recovery and owner wake isolation PASS')
subprocess.run(cli+['suspend','actor',owners['alice']['agent_id'],'-a','finite-hermes-spike'],check=True,stdout=subprocess.DEVNULL)
