import json,sys,time
def clean(x):
 if isinstance(x,dict): return {k:clean(v) for k,v in x.items() if k not in ('image','fullPreferences','mergedPreferences')}
 if isinstance(x,list): return [clean(v) for v in x]
 return x
from websockets.sync.client import connect
with connect('ws://127.0.0.1:5225') as ws:
 rid=str(time.time_ns())
 ws.send(json.dumps({'corrId':rid,'cmd':sys.argv[1]}))
 while True:
  r=json.loads(ws.recv(timeout=60))
  if r.get('corrId')==rid:
   print(json.dumps(clean(r['resp'])));break
