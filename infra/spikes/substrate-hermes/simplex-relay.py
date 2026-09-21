"""Spike: one SimpleX reader, disk spool, authenticated Hermes WS, URL wake.

ACK means accepted by the Hermes adapter, not a crash-safe model-turn commit.
Do not claim exactly-once processing from this experimental protocol.
"""
import asyncio
import hmac
import json
import os
from pathlib import Path
import sqlite3
import urllib.request
from websockets.asyncio.client import connect
from websockets.asyncio.server import serve

class Relay:
    def __init__(self, db_path, token, wake_url, actor):
        self.db=sqlite3.connect(db_path)
        self.db.execute('PRAGMA journal_mode=WAL')
        self.db.execute('PRAGMA synchronous=FULL')
        self.db.execute('CREATE TABLE IF NOT EXISTS events (id INTEGER PRIMARY KEY AUTOINCREMENT, frame TEXT NOT NULL, wake INTEGER NOT NULL)')
        self.token,self.wake_url,self.actor=token,wake_url,actor
        self.upstream=None
        self.downstream=None
        self.changed=asyncio.Event()
        self.changed.set()
        self.reply_routes={}

    async def authorize(self, connection, request):
        if not hmac.compare_digest(request.headers.get('Authorization',''),'Bearer '+self.token):
            return connection.respond(401,'Unauthorized\n')
        if request.path=='/health': return connection.respond(200,'OK\n')
        if request.path not in ('/','/admin'): return connection.respond(404,'Not found\n')

    def enqueue(self, frame):
        resp=frame.get('resp',{})
        typ=resp.get('type')
        # Correlated command responses never wake. Network status chatter does
        # not wake either. Contacts are paired using normal Hermes approval.
        wake=typ in ('contactRequest','receivedContactRequest') or (typ=='newChatItems' and any(
            item.get('chatItem',item).get('chatDir',{}).get('type') in ('directRcv','groupRcv')
            for item in resp.get('chatItems',[])))
        self.db.execute('INSERT INTO events(frame,wake) VALUES(?,?)',(json.dumps(frame),int(wake)))
        self.db.commit()
        self.changed.set()

    async def receive_simplex(self):
        while True:
            try:
                async with connect('ws://127.0.0.1:5225') as ws:
                    self.upstream=ws
                    async for raw in ws:
                        frame=json.loads(raw)
                        corr=frame.get('corrId')
                        if corr:
                            client=self.reply_routes.pop(corr,None)
                            if client:
                                try: await client.send(raw)
                                except Exception: pass
                        else: self.enqueue(frame)
            except Exception as e:
                print('SimpleX reconnect:',type(e).__name__,flush=True)
                await asyncio.sleep(2)
            finally: self.upstream=None

    async def accept_hermes(self, ws):
        is_admin=ws.request.path=='/admin'
        if not is_admin and self.downstream:
            await self.downstream.close(1012,'Replacement connection')
        if not is_admin: self.downstream=ws
        self.changed.set()
        async def deliver():
            last_sent=0
            while True:
                row=self.db.execute('SELECT id,frame FROM events WHERE id>? ORDER BY id LIMIT 1',(last_sent,)).fetchone()
                if row:
                    frame=json.loads(row[1]);frame['_spike_delivery_id']=row[0]
                    await ws.send(json.dumps(frame));last_sent=row[0]
                else:
                    self.changed.clear()
                    # No await between query and clear: enqueue cannot race.
                    await self.changed.wait()
        sender=None if is_admin else asyncio.create_task(deliver())
        try:
            async for raw in ws:
                frame=json.loads(raw)
                ack=frame.get('_spike_ack')
                if isinstance(ack,int):
                    self.db.execute('DELETE FROM events WHERE id=?',(ack,));self.db.commit()
                    continue
                if self.upstream is None:
                    await ws.close(1013,'SimpleX unavailable');return
                corr=frame.get('corrId')
                if corr: self.reply_routes[corr]=ws
                await self.upstream.send(raw)
        finally:
            if sender: sender.cancel()
            if self.downstream is ws: self.downstream=None
            self.reply_routes={k:v for k,v in self.reply_routes.items() if v is not ws}

    def wake(self):
        request=urllib.request.Request(self.wake_url,headers={'ate-target-actor':self.actor})
        with urllib.request.urlopen(request,timeout=60) as r:
            if r.status!=200: raise RuntimeError('Wake failed')

    async def doorbell(self):
        while True:
            if self.db.execute('SELECT 1 FROM events WHERE wake=1 LIMIT 1').fetchone():
                try:
                    await asyncio.to_thread(self.wake)
                    print('Wake request completed',flush=True)
                except Exception as e: print('Wake retry:',type(e).__name__,flush=True)
            await asyncio.sleep(2)

async def main():
    relay=Relay('/data/relay.sqlite',os.environ['SIMPLEX_WS_TOKEN'],os.environ['SPIKE_WAKE_URL'],os.environ['SPIKE_TARGET_ACTOR'])
    async with serve(relay.accept_hermes,'0.0.0.0',8765,process_request=relay.authorize):
        await asyncio.gather(relay.receive_simplex(),relay.doorbell())

if __name__=='__main__': asyncio.run(main())
