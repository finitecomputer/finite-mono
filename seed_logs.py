import json, pathlib, urllib.request
root=pathlib.Path('/tmp/grafana-cleanup-evidence')
end=json.loads((root/'fixture-time.json').read_text())['to']//1000
streams=[]
for source,priority,message in [('kernel','warning','synthetic fixture: temperature threshold exceeded on finite-lat-3'),('systemd','error','synthetic fixture: finite-healthcheck.service failed on finite-lat-5'),('nixos-activation','warning','synthetic fixture: unit restart queued for investigation'),('auth','warning','synthetic fixture: unsuccessful SSH authentication for fixture-user')]:
 streams.append({'stream':{'host':'finite-lat-'+('3' if source=='kernel' else '5'),'source':source,'priority':priority},'values':[[str((end-90-i*900)*10**9),message] for i in reversed(range(6))]})
req=urllib.request.Request('http://127.0.0.1:13100/loki/api/v1/push',data=json.dumps({'streams':streams}).encode(),headers={'Content-Type':'application/json'})
with urllib.request.urlopen(req) as res: print('Synthetic Loki seed:',res.status)
