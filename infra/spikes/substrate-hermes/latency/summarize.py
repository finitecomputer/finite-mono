"""Summarize content-free real-message traces; small samples are not p95 estimates."""
import argparse,json,statistics
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('reports',type=Path,nargs='+');a=p.parse_args()
rows=[]
for path in a.reports:
 report=json.loads(path.read_text())
 for asleep in (False,True):
  trials=[x for x in report['trials'] if x['sleeping']==asleep]
  if not trials:continue
  fields={}
  for x in trials:
   stages={}
   for e in x['actor']:stages.setdefault(e['stage'],e['t'])
   notifications=[e['t'] for e in x['listener'] if 'notification' in e]
   dispatches=[e['t'] for e in x['listener'] if 'wake_start' in e]
   values={'first_text':x['first_seconds'],'complete_reply':x['complete_seconds']}
   if notifications:values['notification']=min(notifications)-x['send']
   if notifications and dispatches:values['dispatch']=min(dispatches)-min(notifications)
   if asleep and 'actor_http_ready' in stages and dispatches:values['restore_to_http']=stages['actor_http_ready']-min(dispatches)
   if asleep and 'actor_http_ready' in stages and 'simplex_received' in stages:values['http_to_message']=stages['simplex_received']-stages['actor_http_ready']
   if 'simplex_received' in stages and 'batch_flushed' in stages:values['batch']=stages['batch_flushed']-stages['simplex_received']
   for key,value in values.items():fields.setdefault(key,[]).append(value)
  rows.append({'report':path.name,'label':report['label'],'sleeping':asleep,'n':len(trials),'seconds':{k:{'median':round(statistics.median(v),3),'min':round(min(v),3),'max':round(max(v),3)} for k,v in fields.items()}})
print(json.dumps(rows,indent=2))
