"""Run in the notification pod: prove notification credentials cannot fetch chat.
No keys, notification payloads, message bodies, or queue IDs are printed.
"""
import json,sqlite3,sys
from pathlib import Path
from cryptography.hazmat.primitives import serialization as ser
from smp import SMP,un64,b64
rows=sqlite3.connect('file:/data/notifications.db?mode=ro',uri=True).execute('SELECT owner,payload FROM registrations').fetchall()
checks=[]
for owner,payload in rows:
 for row in json.loads(payload):
  assert set(row)=={'host','port','key_hash','notifier_id','notifier_key'}
  try:SMP(row['host'],row['port'],b64(bytes(32)))
  except ValueError as e:assert str(e)=='SMP CA fingerprint mismatch'
  else:raise AssertionError('Wrong CA pin accepted')
  conn=SMP(row['host'],row['port'],row['key_hash'])
  key=ser.load_der_private_key(un64(row['notifier_key']),None)
  try:
   result=conn.command(un64(row['notifier_id']),b'GET',key)
   assert result==b'ERR AUTH',result[:20]
   checks.append({'owner':owner,'message_fetch':'ERR AUTH','incorrect_CA_pin':'rejected'})
  finally:conn.close()
assert {c['owner'] for c in checks}=={'alice','bob'}
print(json.dumps({'passed':True,'registration_fields':'notification capability only','checks':checks},indent=2))
