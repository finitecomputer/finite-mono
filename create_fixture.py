import json, math, time, pathlib, datetime
root=pathlib.Path('/tmp/grafana-cleanup-evidence')
end=json.loads((root/'fixture-time.json').read_text())['to']//1000 if (root/'fixture-time.json').exists() else int(time.time()//60*60)
(root/'fixture-time.json').write_text(json.dumps({'from':(end-86400)*1000,'to':end*1000,'timezone':'UTC','synthetic':True},indent=2)+'\n')
for path in ('prometheus-data','loki-data','provisioning/datasources','provisioning/dashboards','dashboards'):
 (root/path).mkdir(parents=True,exist_ok=True)
(root/'dashboards/finite-production-mvp.json').write_bytes((root/'baseline.json').read_bytes())
(root/'prometheus.yml').write_text('global:\n  scrape_interval: 30s\nscrape_configs: []\n')
(root/'provisioning/datasources/finite.yml').write_text('''apiVersion: 1
datasources:
  - name: Finite Prometheus
    uid: finite-prometheus
    type: prometheus
    access: proxy
    url: http://finite-evidence-prometheus:9090
    isDefault: true
    jsonData:
      timeInterval: 30s
  - name: Finite Loki
    uid: finite-loki
    type: loki
    access: proxy
    url: http://finite-evidence-loki:3100
''')
(root/'provisioning/dashboards/finite.yml').write_text('''apiVersion: 1
providers:
  - name: finite-evidence
    type: file
    updateIntervalSeconds: 5
    options:
      path: /var/lib/grafana/dashboards
''')
(root/'loki.yml').write_text('''auth_enabled: false
server:
  http_listen_port: 3100
common:
  instance_addr: 127.0.0.1
  path_prefix: /loki
  storage:
    filesystem:
      chunks_directory: /loki/chunks
      rules_directory: /loki/rules
  replication_factor: 1
  ring:
    kvstore:
      store: inmemory
schema_config:
  configs:
    - from: 2020-10-24
      store: tsdb
      object_store: filesystem
      schema: v13
      index:
        prefix: index_
        period: 24h
limits_config:
  allow_structured_metadata: false
  reject_old_samples: false
analytics:
  reporting_enabled: false
''')
series={}
def add(name, labels, fn):
 key=name+'{'+','.join(k+'='+json.dumps(str(v)) for k,v in sorted(labels.items()))+'}'
 series[key]=fn
public=['finite.computer','chat.finite.computer','brain.finite.computer','finite.site','uptime-probe.finite.site']
for n,job in enumerate(public):
 add('probe_success',{'job':job,'instance':'https://'+job},lambda t,n=n: 0 if n==2 and t>end-3600 else 1)
 add('probe_duration_seconds',{'job':job,'instance':'https://'+job},lambda t,n=n: round((0.08+n*0.05)*(1+0.15*math.sin(t/1200))+(.9 if n==1 and t>end-7200 else 0),4))
for n,comp in enumerate(['chat','brain','sites','core']):
 add('finite_healthcheck_success',{'component':comp,'host':'synthetic-fixture','check':comp},lambda t,n=n: 0 if n==1 and t>end-3600 else 1)
 add('finite_component_build_info',{'component':comp,'host':'finite-lat-'+str(n+1),'version':'2026.10.02-fixture','revision':'synthetic-a1b2c3','commit':'synthetic-a1b2c3','build_time':'2026-10-01T09:00:00Z'},lambda t:1)
 add('finite_component_version_mismatched_active_agents',{'component':comp,'host':'finite-lat-'+str(n+1)},lambda t,n=n:3 if n==2 else 0)
for host in range(1,6):
 labels={'instance':f'finite-lat-{host}'}
 add('up',dict(labels,job='finite-internal-health'),lambda t,host=host: 0 if host==5 and t>end-5400 else 1)
 for cpu in range(2):
  add('node_cpu_seconds_total',dict(labels,mode='idle',cpu=cpu),lambda t,host=host:round((t-(end-7*86400))*(.08 if host==3 else .75-host*.04),3))
 for duration in (1,5,15):
  add('node_load'+str(duration),labels,lambda t,host=host,duration=duration:round((11 if host==3 else host*.7)*(1+.1*math.sin(t/(900*duration))),3))
 for name,val in [('MemTotal',64e9),('MemAvailable',5e9 if host==2 else (20+host*2)*1e9),('SwapTotal',8e9),('SwapFree',7e9 if host!=2 else 3e9)]:
  add('node_memory_'+name+'_bytes',labels,lambda t,val=val:val)
 for mount in ('/','/data'):
  disklabels=dict(labels,mountpoint=mount,fstype='ext4',device='/dev/nvme0n1')
  add('node_filesystem_size_bytes',disklabels,lambda t:1e12)
  add('node_filesystem_avail_bytes',disklabels,lambda t,host=host,mount=mount:4e10 if host==4 and mount=='/data' else 6e11)
  add('node_filesystem_readonly',disklabels,lambda t,host=host,mount=mount:1 if host==4 and mount=='/data' else 0)
 for name,rate in [('read_bytes',2e6*host),('written_bytes',3e6*host),('io_time_seconds',.04*host)]:
  add('node_disk_'+name+'_total',dict(labels,device='nvme0n1'),lambda t,rate=rate:(t-(end-7*86400))*rate)
 for name,rate in [('receive_bytes',3e6*host),('transmit_bytes',2e6*host),('receive_errs',2 if host==4 else 0),('transmit_errs',.2 if host==4 else 0)]:
  add('node_network_'+name+'_total',dict(labels,device='eth0'),lambda t,rate=rate:(t-(end-7*86400))*rate)
 add('node_hwmon_temp_celsius',dict(labels,chip='k10temp',sensor='temp1'),lambda t,host=host:97 if host==3 else 45+host*4)
 add('node_hwmon_sensor_label',dict(labels,chip='k10temp',sensor='temp1',label='Tctl'),lambda t:1)
 add('node_cpu_scaling_frequency_hertz',dict(labels,cpu='0'),lambda t,host=host:1.8e9 if host==3 else 3.7e9)
 add('node_cpu_scaling_frequency_max_hertz',dict(labels,cpu='0'),lambda t:4e9)
for name,val in [('up',1),('finite_sites_existing',428),('finite_sites_published',311)]:
 add(name,{'job':'finite-sites-metrics','instance':'synthetic-sites'},lambda t,val=val:val)
add('finite_sites_metrics_collected_at_seconds',{'job':'finite-sites-metrics','instance':'synthetic-sites'},lambda t:t)
for day in range(30):
 date=datetime.datetime.fromtimestamp(end-day*86400,datetime.timezone.utc).strftime('%Y-%m-%d')
 add('finite_sites_created_by_day',{'job':'finite-sites-metrics','instance':'synthetic-sites','date':date},lambda t,day=day:3+(day*7)%19)
for n in range(2):
 add('finite_runtime_artifact_active_agents',{'artifact_id':'synthetic-artifact-'+str(n+1),'version_label':'2026.10.0'+str(2-n),'promoted':'true' if n==0 else 'false','source_host_id':'finite-lat-1'},lambda t,n=n:24 if n==0 else 3)
times=list(range(end-7*86400,end-86400,300))+list(range(end-86400,end+1,30))
with (root/'metrics.openmetrics').open('w') as f:
 for key,fn in series.items():
  name=key.split('{')[0]
  f.write('# TYPE '+name+' gauge\n')
  for t in times:
   if key.startswith('probe_success{') and 'job="finite.site"' in key and end-6*3600 < t < end-5*3600: continue
   f.write(f'{key} {fn(t)} {t}\n')
 f.write('# EOF\n')
(root/'fixture-summary.json').write_text(json.dumps({'synthetic':True,'series':len(series),'samples_per_series':len(times),'from':end-7*86400,'to':end,'conditions':['finite.site has no probe_success samples between 16:23 and 17:23 UTC (synthetic missing-data gap)','brain public endpoint down in last hour','chat latency elevated in last two hours','finite-lat-5 scrape down in last 90 minutes','finite-lat-3 high CPU and thermal pressure','finite-lat-2 high memory use','finite-lat-4 /data 96% used and read only; network errors','sites counts and 30 daily cohorts','one old runtime artifact and component version drift']},indent=2)+'\n')
print('fixture seed generated:',len(series),'series;',len(times),'samples per series')
