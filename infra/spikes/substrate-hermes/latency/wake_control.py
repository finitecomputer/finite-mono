"""Authenticated, acknowledged resume hook, using the adapter's sole WS reader.

The clock gap is a spike heuristic; replace it with a provider resume event.
No message keys or content cross this boundary.
"""
import asyncio,hmac,os,time
from pathlib import Path
from latency_trace import mode,trace
REQUEST=Path('/home/agent/simplex/wake-request')
ACK=REQUEST.with_name('wake-ack')
def read(path):
    try:return path.read_text()
    except OSError:return ''
def write(path,value):
    path.parent.mkdir(parents=True,exist_ok=True)
    tmp=path.with_name(path.name+'.'+value+'.tmp')
    tmp.write_text(value);os.chmod(tmp,0o600);tmp.replace(path)
async def request_wake(request):
    supplied=request.headers.get('x-simplex-wake','') if request else ''
    if not supplied:return
    from fastapi import HTTPException
    token=os.environ.get('SPIKE_NOTIFY_TOKEN','')
    if not token or not hmac.compare_digest(supplied,token):
        raise HTTPException(status_code=401,detail='Invalid wake credential')
    nonce=str(time.time_ns());write(REQUEST,nonce);trace('actor_http_ready')
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        if read(ACK)==nonce:return
        await asyncio.sleep(.01)
    # The shared listener keeps the durable wake intent and retries.
    raise HTTPException(status_code=503,detail='Transport wake not acknowledged')
async def watch(adapter):
    previous=time.time();seen='';restored=False;last_reconnect=0
    while adapter._running:
        await asyncio.sleep(.05)
        now=time.time();gap=now-previous;previous=now
        if gap>1:
            restored=True;trace('resume_gap',gap=gap)
        request=read(REQUEST)
        if not request or request==seen:continue
        if not mode().get('reconnect',False) or not restored:
            seen=request;write(ACK,request);continue
        if now-last_reconnect<2 or not adapter._ws:continue
        trace('simplex_reconnect_start');last_reconnect=now
        commands=mode().get('wake_commands',['/_app activate'])
        needs_activate='/_app suspend 0' in commands
        ok=True
        try:
            for command in commands:
                if command not in ('/_app suspend 0','/_app activate','/reconnect','/_resubscribe all'):continue
                result=await adapter._send_command(command,timeout=5)
                success=bool(result and result.get('type')=='cmdOk')
                trace('simplex_reconnect_done',command=command,ok=success)
                ok=ok and success
                if command=='/reconnect':await asyncio.sleep(.1)
                if command=='/_app activate' and success:needs_activate=False
        finally:
            # Never leave the daemon suspended if reset fails halfway through.
            if needs_activate:
                result=await adapter._send_command('/_app activate',timeout=5)
                ok=bool(result and result.get('type')=='cmdOk') and ok
            previous=time.time()
        if ok:
            restored=False;seen=request;write(ACK,request)
