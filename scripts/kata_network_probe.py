#!/usr/bin/env python3
"""Read-only, bounded Kata-created network namespace observations."""
from __future__ import annotations
import ctypes
import fcntl
import json
import os
from pathlib import Path
import re
import select
import shutil
import socket
import subprocess
import time
from typing import Any
if __package__:
    from . import kata_guest_probe as gp
else:
    import kata_guest_probe as gp


def parse_network(persist: dict[str, Any], spec: dict[str, Any]) -> dict[str, Any]:
    network=persist['Network'];config=persist['Config']['NetworkConfig']
    path=network['NetworkID']
    gp.require(isinstance(path,str) and bool(re.fullmatch(r'/(?:var/run|run)/netns/cnitest-[a-f0-9-]{36}',path)), 'unrecognized Kata namespace path')
    gp.require(network['NetworkCreated'] is True and config['NetworkCreated'] is True and config['NetworkID']==path,'network creation metadata mismatch')
    gp.require(config['DisableNewNetwork'] is False and not config.get('DanConfigPath'),'nondefault network config')
    namespaces=[entry for entry in spec['linux']['namespaces'] if entry['type']=='network']
    gp.require(len(namespaces)==1 and namespaces[0].get('path','')=='','OCI network namespace not Kata-created')
    endpoints=network['Endpoints'] or []
    gp.require(isinstance(endpoints,list) and len(endpoints)<=8,'endpoint bound exceeded')
    selected=[]
    for endpoint in endpoints:
        gp.require(set(endpoint)=={'Type','Veth'} and endpoint['Type']=='virtual','non-veth endpoint refused')
        pair=endpoint['Veth']['NetPair']
        gp.require(pair['NetInterworkingModel']==2,'network model not TC-filter')
        selected.append({'type':'virtual','model':2,'tap_name':pair['TAPIface']['Name'],
                         'tap_mac':pair['TAPIface']['HardAddr'],'veth_name':pair['VirtIface']['Name'],
                         'veth_mac':pair['VirtIface']['HardAddr']})
        gp.require(all(re.fullmatch(r'[A-Za-z0-9_.-]{1,15}',selected[-1][key]) for key in ('tap_name','veth_name')),'interface name invalid')
        gp.require(all(value=='' or re.fullmatch(r'(?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}',value) for key,value in selected[-1].items() if key.endswith('_mac')),'interface MAC invalid')
    return {'path':path,'network_created':True,'endpoints':selected}



def exact_shim(cid: str) -> tuple[int, tuple[str,str,tuple[str,...]]]:
    candidates=[]
    for path in Path('/proc').iterdir():
        if not path.name.isdecimal():continue
        try:
            executable=os.readlink(path/'exe')
            if not executable.startswith('/nix/store/') or Path(executable).name!='containerd-shim-kata-v2':continue
            fields=(path/'stat').read_text().rsplit(')',1)[1].split()
            if fields[0] in ('Z','X'):continue
            identity=gp.process_identity(int(path.name));argv=identity[2]
            if '-id' in argv and argv[argv.index('-id')+1]==cid and '-namespace' in argv and argv[argv.index('-namespace')+1]=='finite':
                candidates.append((int(path.name),identity))
        except (FileNotFoundError,ProcessLookupError):continue
    gp.require(len(candidates)>0,'exact target shim absent')
    gp.require(len(candidates)==1,'exact target shim ambiguous')
    return candidates[0]


def open_shim_network(cid: str,network_path: str) -> tuple[int,dict[str,Any]]:
    pid,identity=exact_shim(cid);pidfd=os.pidfd_open(pid)
    descriptor=None
    try:
        gp.require(gp.process_identity(pid)==identity and not select.select([pidfd],[],[],0)[0],'shim lifetime changed before namespace open')
        normalized='/run/netns/'+Path(network_path).name
        view=Path('/proc',str(pid),'root'+normalized)
        # /proc/PID/root is a kernel magic link to this exact task's mount/root
        # view; the canonical cached path is never reinterpreted as host state.
        descriptor=os.open(view,os.O_RDONLY|os.O_CLOEXEC)
        identity_ns=namespace_identity(descriptor)
        gp.require(gp.process_identity(pid)==identity and not select.select([pidfd],[],[],0)[0],'shim changed during namespace open')
        return descriptor,{'shim_pid':pid,'shim_start':identity[0],'shim_executable':identity[1],
                           'shim_mount_namespace':os.readlink('/proc/'+str(pid)+'/ns/mnt'),
                           'view':'exact-shim-proc-root','namespace_identity':identity_ns}
    except BaseException:
        if descriptor is not None:os.close(descriptor)
        raise
    finally:os.close(pidfd)


def namespace_identity(fd: int) -> dict[str,int]:
    gp.require(fcntl.ioctl(fd,0xb703)==0x40000000,'not a network namespace')
    state=os.fstat(fd)
    return {'device':state.st_dev,'inode':state.st_ino}


def read_links(fd: int) -> list[dict[str,Any]]:
    executable=shutil.which('ip')
    gp.require(executable is not None and os.path.realpath(executable).startswith('/nix/store/'),'immutable ip executable unavailable')
    reader,writer=os.pipe();child=os.fork()
    if child==0:
        try:
            os.close(reader)
            gp.require(ctypes.CDLL(None,use_errno=True).setns(fd,0x40000000)==0,'setns failed')
            result=subprocess.run([executable,'-j','-d','link','show'],capture_output=True,timeout=4)
            gp.require(result.returncode==0 and len(result.stdout)<=1024*1024,'link read failed')
            payload=result.stdout
            while payload:payload=payload[os.write(writer,payload):]
            os._exit(0)
        except BaseException:os._exit(2)
    os.close(writer);raw=bytearray()
    try:
        while True:
            gp.require(select.select([reader],[],[],6)[0],'link reader timed out')
            block=os.read(reader,65536)
            if not block:break
            raw.extend(block);gp.require(len(raw)<=1024*1024,'link read bound exceeded')
    finally:os.close(reader)
    _,status=os.waitpid(child,0);gp.require(status==0,'namespace link reader refused')
    links=json.loads(raw);gp.require(isinstance(links,list) and len(links)<=32,'link count bound exceeded')
    return [{key:entry[key] for key in ('ifindex','ifname','address','link_index','link_netnsid','master','linkinfo') if key in entry} for entry in links]


def collect_kata_network_probe(report: dict[str,Any]) -> dict[str,Any]:
    result={'schema':'finite.kata-network-probe.v1','read_only':True,'repair_authority':False,'namespace_ownership_proven':False}
    descriptor=None;phase='bind-target'
    try:
        deadline=time.monotonic()+20
        binding=gp.bind_target(report,deadline);cid=binding['target']['container_id']
        persist=json.loads(gp.bounded(gp.SANDBOXES/cid/'persist.json',2*1024*1024))
        spec=json.loads(gp.bounded(gp.BUNDLES/cid/'config.json',2*1024*1024))
        phase='parse-network'
        network=parse_network(persist,spec);result.update(container_id=cid,network=network)
        phase='resolve-network-path'
        path=Path(network['path']);gp.require(path.resolve()==Path('/run/netns')/path.name,'namespace path redirected')
        phase='open-classify-exact-shim-namespace'
        descriptor,view=open_shim_network(cid,network['path'])
        identity=view['namespace_identity'];result['namespace_identity']=identity;result['namespace_view']=view
        phase='open-host-namespace'
        host=os.open('/proc/1/ns/net',os.O_RDONLY)
        phase='classify-host-namespace'
        try:gp.require(namespace_identity(host)!=identity,'target namespace is host namespace')
        finally:os.close(host)
        phase='enumerate-sandbox-networks'
        peers=[];entries=list(gp.SANDBOXES.iterdir());gp.require(len(entries)<=512,'sandbox count bound exceeded')
        for directory in entries:
            gp.require(time.monotonic()<deadline,'network probe deadline exceeded')
            if not re.fullmatch('[a-f0-9]{64}',directory.name) or directory.name==cid:continue
            try:
                phase='read-peer-persist'
                peer=json.loads(gp.bounded(directory/'persist.json',2*1024*1024));peer_path=peer.get('Network',{}).get('NetworkID')
                if not peer_path:continue
                phase='open-peer-shim-namespace'
                if not re.fullmatch(r'/(?:var/run|run)/netns/cnitest-[a-f0-9-]{36}',peer_path):continue
                try:peer_fd,_=open_shim_network(directory.name,peer_path)
                except gp.ProbeRefusal as error:
                    if str(error)=='exact target shim absent':continue
                    raise
                try:
                    phase='classify-peer-namespace'
                    if namespace_identity(peer_fd)==identity:peers.append(directory.name)
                finally:os.close(peer_fd)
            except FileNotFoundError:continue
        result['other_persisted_sandboxes_sharing_namespace']=peers
        phase='compare-live-qemu-namespaces'
        same_namespace=[]
        for process in gp.inventory(deadline):
            try:
                process_fd=os.open('/proc/'+str(process['pid'])+'/ns/net',os.O_RDONLY|os.O_CLOEXEC)
                try:
                    if namespace_identity(process_fd)==identity:same_namespace.append(process['pid'])
                finally:os.close(process_fd)
            except (FileNotFoundError,ProcessLookupError):continue
        result['live_qemu_pids_in_namespace']=same_namespace
        result['target_qemu_owns_namespace']=binding['pid'] in same_namespace
        result['other_qemu_pids_in_namespace']=[pid for pid in same_namespace if pid!=binding['pid']]
        phase='read-pinned-network-links'
        result['interfaces']=read_links(descriptor)
        phase='reopen-target-namespace'
        current,current_view=open_shim_network(cid,network['path'])
        gp.require(current_view==view,'shim namespace view changed during observation')
        try:gp.require(namespace_identity(current)==identity,'namespace path changed during observation')
        finally:os.close(current)
        phase='revalidate-target'
        gp.require(gp.bind_target(report,deadline)==binding,'target changed during network observation')
        result['observation_complete']=True
    except Exception as error:
        result['observation_complete']=False
        result['refusal_phase']=phase
        if isinstance(error,OSError):result['errno']=error.errno
        result['refusal_reason']=str(error) if isinstance(error,gp.ProbeRefusal) else type(error).__name__
    finally:
        if descriptor is not None:os.close(descriptor)
    return result
