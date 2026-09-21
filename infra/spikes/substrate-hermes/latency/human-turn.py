"""Event-driven timing inside the external human client. No actor access."""
import json,sys,time,uuid
from websockets.sync.client import connect
args=json.load(sys.stdin)
with connect('ws://127.0.0.1:5225',max_size=8*1024*1024) as ws:
 cid=args['contact'];marker=args['marker'];first=None;events=[]
 cmd='/_send @'+str(cid)+' json '+json.dumps([{'msgContent':{'type':'text','text':args['text']}}])
 start=time.time();ws.send(json.dumps({'corrId':uuid.uuid4().hex,'cmd':cmd}))
 while True:
  r=json.loads(ws.recv(timeout=180));resp=r.get('resp',{});kind=resp.get('type')
  if kind not in ('newChatItems','newChatItem','chatItemUpdated'):continue
  items=resp.get('chatItems',[resp.get('chatItem',resp)] if kind=='chatItemUpdated' else [resp])
  for item in items:
   info=item.get('chatInfo',{});chat=item.get('chatItem',{})
   if info.get('contact',{}).get('contactId')!=cid or chat.get('chatDir',{}).get('type')!='directRcv':continue
   text=chat.get('content',{}).get('msgContent',{}).get('text','')
   if (marker not in text) and (not args.get('long') or 'RAIN' not in text):continue
   now=time.time();first=first or now
   events.append({'t':now,'kind':kind,'item':chat.get('meta',{}).get('itemId'),'chars':len(text),'cursor':text.rstrip().endswith('▉')})
   if marker in text and not text.rstrip().endswith('▉'):
    print(json.dumps({'send':start,'first':first,'complete':now,'first_seconds':first-start,'complete_seconds':now-start,'events':events}));raise SystemExit(0)
