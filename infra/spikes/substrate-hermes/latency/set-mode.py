"""Change synthetic actor latency settings through authenticated native Hermes."""
import argparse,base64,json,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent.parent))
from probe import Client
p=argparse.ArgumentParser();p.add_argument('--state',type=Path,required=True);p.add_argument('--owners',type=Path,required=True);p.add_argument('--user',default='alice');p.add_argument('--baseline',action='store_true');p.add_argument('--streaming',action='store_true');p.add_argument('--wake-command',action='append',choices=['/_app suspend 0','/_app activate','/reconnect','/_resubscribe all']);p.add_argument('--no-reconnect',action='store_true');a=p.parse_args()
o=next(o for o in json.loads(a.owners.read_text()) if o['user_id']==a.user)
c=Client(o,a.state/'fixtures',18080);c.login()
mode={'batch_delay':.8 if a.baseline else .05,'reconnect':not (a.baseline or a.no_reconnect),'streaming':a.streaming}
if a.wake_command:mode['wake_commands']=a.wake_command
encoded=base64.b64encode(json.dumps(mode).encode()).decode()
r=c.http.post(c.base+'/api/files/upload',json={'path':'/home/agent/latency-mode.json','data_url':'data:application/json;base64,'+encoded,'overwrite':True},timeout=30)
r.raise_for_status();print(json.dumps(mode))
