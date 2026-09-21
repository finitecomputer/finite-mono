"""Real WS authentication and durable replay checks for the experimental relay."""
import asyncio,importlib.util,json,tempfile,unittest
from pathlib import Path
from websockets.asyncio.client import connect
from websockets.asyncio.server import serve
from websockets.exceptions import InvalidStatus
spec=importlib.util.spec_from_file_location('relay',Path(__file__).with_name('simplex-relay.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class RelayTest(unittest.IsolatedAsyncioTestCase):
 async def test_auth_admin_isolation_and_replay_until_ack(self):
  with tempfile.TemporaryDirectory() as tmp:
   path=Path(tmp)/'queue.db'
   relay=module.Relay(path,'test-only-token','http://unused.invalid','fixture/alice')
   event={'resp':{'type':'newChatItems','chatItems':[{'chatItem':{'chatDir':{'type':'directRcv'}}}]}}
   relay.enqueue(event)
   row=relay.db.execute('SELECT id,wake FROM events').fetchone()
   self.assertEqual(row[1],1)
   relay.db.close()
   # Simulate a process restart before delivery, retaining only SQLite state.
   relay=module.Relay(path,'test-only-token','http://unused.invalid','fixture/alice')
   async with serve(relay.accept_hermes,'127.0.0.1',0,process_request=relay.authorize) as server:
    url='ws://127.0.0.1:'+str(server.sockets[0].getsockname()[1])
    with self.assertRaises(InvalidStatus) as rejected:
     async with connect(url): pass
    self.assertEqual(rejected.exception.response.status_code,401)
    auth={'Authorization':'Bearer test-only-token'}
    async with connect(url,additional_headers=auth) as client:
     first=json.loads(await client.recv())
     self.assertEqual(first['_spike_delivery_id'],row[0])
     async with connect(url+'/admin',additional_headers=auth) as admin:
      with self.assertRaises(TimeoutError): await asyncio.wait_for(admin.recv(),.05)
      await asyncio.wait_for(await client.ping(),1)
    # Disconnect without ACK: the next adapter must receive the same record.
    async with connect(url,additional_headers=auth) as client:
     replay=json.loads(await client.recv())
     self.assertEqual(replay,first)
     await client.send(json.dumps({'_spike_ack':row[0]}))
     for _ in range(100):
      if relay.db.execute('SELECT count(*) FROM events').fetchone()[0]==0:break
      await asyncio.sleep(.01)
     self.assertEqual(relay.db.execute('SELECT count(*) FROM events').fetchone()[0],0)
   relay.db.close()

 async def test_outbound_messages_and_network_noise_do_not_wake(self):
  with tempfile.TemporaryDirectory() as tmp:
   relay=module.Relay(Path(tmp)/'queue.db','test','http://unused.invalid','fixture/alice')
   relay.enqueue({'resp':{'type':'newChatItems','chatItems':[{'chatItem':{'chatDir':{'type':'directSnd'}}}]}})
   relay.enqueue({'resp':{'type':'connectionsDiff'}})
   relay.enqueue({'resp':{'type':'receivedContactRequest'}})
   self.assertEqual([r[0] for r in relay.db.execute('SELECT wake FROM events ORDER BY id')],[0,0,1])
   relay.db.close()

if __name__=='__main__': unittest.main()
