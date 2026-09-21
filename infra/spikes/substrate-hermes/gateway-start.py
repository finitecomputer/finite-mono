"""Wait for external SimpleX transport after golden snapshot / egress enablement."""
import os,time,urllib.request
url=os.environ['SIMPLEX_WS_URL'].replace('ws://','http://').replace('wss://','https://').rstrip('/')+'/health'
while True:
    try:
        req=urllib.request.Request(url,headers={'Authorization':'Bearer '+os.environ['SIMPLEX_WS_TOKEN']})
        with urllib.request.urlopen(req,timeout=5) as r:
            if r.status==200: break
    except Exception: time.sleep(2)
os.execvp('hermes',['hermes','gateway','run'])
