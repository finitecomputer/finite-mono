"""Stub Core boundary: owner -> stable actor reference -> provider operation.

--user is a trusted test fixture, not an authentication implementation.
No worker placement, IPs, port allocation, scheduler, or lifecycle DB.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess

p = argparse.ArgumentParser()
p.add_argument('--user', required=True)
p.add_argument('action', choices=['create','inspect','sleep','wake','url'])
a = p.parse_args()
owner = next((o for o in json.loads(Path(__file__).with_name('owners.json').read_text())
              if o['user_id'] == a.user), None)
if owner is None:
    p.error('unknown owner')
actor = owner['agent_id']
if a.action == 'url':
    print('https://' + owner['host'])
else:
    cli = os.environ.get('ATE_CLI', 'kubectl-ate')
    base = [cli, '--context', os.environ.get('SPIKE_KUBE_CONTEXT', 'kind-finite-hermes-spike')]
    cmd = {'create':['create','actor',actor,'--template',actor],
           'inspect':['get','actor',actor],
           'sleep':['suspend','actor',actor],
           'wake':['resume','actor',actor]}[a.action]
    subprocess.run(base + cmd + ['-a','finite-hermes-spike'], check=True)
