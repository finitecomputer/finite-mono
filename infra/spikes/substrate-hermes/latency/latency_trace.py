"""Content-free spike timing. These records remain inside the actor."""
import json,os,time
from pathlib import Path

def mode():
    try:return json.loads(Path('/home/agent/latency-mode.json').read_text())
    except (OSError,ValueError):return {}
def trace(stage,**fields):
    record={'stage':stage,'t':time.time(),'mono':time.monotonic(),**fields}
    line=(json.dumps(record,separators=(',',':'))+'\n').encode()
    fd=os.open('/home/agent/latency.jsonl',os.O_WRONLY|os.O_CREAT|os.O_APPEND,0o600)
    try:os.write(fd,line)
    finally:os.close(fd)
