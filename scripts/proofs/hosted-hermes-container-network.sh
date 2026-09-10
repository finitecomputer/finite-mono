#!/usr/bin/env bash
# Real Linux containerd/nerdctl/CNI qualification. No Kata guest claim.
# From macOS: bash scripts/proofs/hosted-hermes-container-network.sh
# This enters the existing OrbStack nixos VM, realizes repo-pinned Nix tools,
# and runs all networking/daemons in disposable net/mount/PID namespaces.
set -euo pipefail
proof_script="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
proof_repo="$(cd "$(dirname "$proof_script")/../.." && pwd)"

if [[ "$(uname -s)" != Linux ]]; then
  exec orb -m nixos -u root bash "$proof_script" "$@"
fi

if [[ "${1:-}" != --inside ]]; then
  [[ "$(id -u)" == 0 ]] || { echo 'Root is required inside the disposable Linux namespace' >&2; exit 2; }
  mapfile -t proof_outputs < <(nix build --impure --file "${proof_script%.sh}.nix" tools image --no-link --print-out-paths)
  [[ ${#proof_outputs[@]} == 2 ]] || { echo 'Pinned tool/image realization failed' >&2; exit 2; }
  proof_scratch="$(mktemp -d /tmp/finite-hermes-container-network.XXXXXX)"
  proof_cgroup="/sys/fs/cgroup/$(basename "$proof_scratch")"
  mkdir "$proof_cgroup"
  # Only this unique cgroup tree: children have exited with namespace PID1.
  cleanup_outer() {
    local result="$?"
    find "$proof_cgroup" -depth -type d -exec rmdir {} \; 2>/dev/null || true
    if [[ -d "$proof_cgroup" ]]; then
      echo "Disposable cgroup cleanup failed: $proof_cgroup" >&2
      result=2
    fi
    rm -rf -- "$proof_scratch"
    [[ ! -e "$proof_scratch" ]] || result=2
    exit "$result"
  }
  trap cleanup_outer EXIT
  unshare --mount --ipc --uts --net --pid --fork --kill-child --mount-proc \
    bash "$proof_script" --inside "$proof_scratch" "${proof_outputs[0]}" "${proof_outputs[1]}"
  exit "$?"
fi

proof_scratch="$2"
proof_tools="$3"
proof_image="$4"
export PATH="$proof_tools/bin:$PATH"
proof_daemon_pid=''
cleanup() {
  local result="$?"
  if [[ -n "$proof_daemon_pid" ]]; then
    kill "$proof_daemon_pid" 2>/dev/null || true
    for _ in {1..30}; do
      kill -0 "$proof_daemon_pid" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL "$proof_daemon_pid" 2>/dev/null || true
    wait "$proof_daemon_pid" 2>/dev/null || true
  fi
  if ((result != 0)); then tail -n 60 "$proof_scratch/containerd.log" >&2 || true; fi
  # Namespace PID1 exiting kills its remaining shims/reservation sleepers.
  # Outer trap removes the private daemon/image/CNI state after namespace exit.
}
trap cleanup EXIT
# Namespace mounts keep CNI caches and named netns files off the VM host.
mount -t tmpfs tmpfs /run
mount -t tmpfs tmpfs /var/lib
ip link set lo up
mkdir -p "$proof_scratch/cni" "$proof_scratch/fixture"
cat > "$proof_scratch/containerd.toml" <<EOF
version = 4
root = "$proof_scratch/containerd-root"
state = "$proof_scratch/containerd-state"
disabled_plugins = ["io.containerd.cri.v1.images", "io.containerd.cri.v1.runtime", "io.containerd.grpc.v1.cri"]
[plugins."io.containerd.server.v1.grpc"]
  address = "$proof_scratch/containerd.sock"
[plugins."io.containerd.server.v1.ttrpc"]
  address = "$proof_scratch/containerd.sock.ttrpc"
[plugins."io.containerd.internal.v1.opt"]
  path = "$proof_scratch/opt"
[[plugins."io.containerd.transfer.v1.local".unpack_config]]
  platform = "linux/arm64"
  snapshotter = "native"
  differ = "walking"
EOF
cat > "$proof_scratch/cni/10-finite.conflist" <<EOF
{"cniVersion":"1.0.0","name":"finite","plugins":[
 {"type":"bridge","bridge":"finite0","isGateway":true,"ipMasq":true,"hairpinMode":true,
  "ipam":{"type":"host-local","dataDir":"$proof_scratch/ipam","ranges":[[{"subnet":"10.89.0.0/16"}]],"routes":[{"dst":"0.0.0.0/0"}]}},
 {"type":"portmap","capabilities":{"portMappings":true}},
 {"type":"firewall"},{"type":"tuning"}]}
EOF
cat > "$proof_scratch/fixture/server.mjs" <<'JS'
import http from 'node:http';
import { randomUUID } from 'node:crypto';
const instance = randomUUID();
http.createServer((request, response) => {
  response.setHeader('content-type', 'application/json');
  response.end(JSON.stringify({identity:process.env.PROOF_ID,instance,pid:process.pid}));
  if(request.url === '/exit') setTimeout(() => process.exit(42), 20);
}).listen(8080, '0.0.0.0');
JS
containerd --config "$proof_scratch/containerd.toml" --log-level info >"$proof_scratch/containerd.log" 2>&1 &
proof_daemon_pid="$!"
proof_nerdctl=(nerdctl --address "$proof_scratch/containerd.sock" --namespace finite-hermes-proof \
  --data-root "$proof_scratch/nerdctl" --cni-path "$proof_tools/bin" --cni-netconfpath "$proof_scratch/cni" \
  --snapshotter native --cgroup-manager cgroupfs)
for _ in {1..100}; do
  if ctr --address "$proof_scratch/containerd.sock" version >/dev/null 2>&1; then break; fi
  if ! kill -0 "$proof_daemon_pid" 2>/dev/null; then cat "$proof_scratch/containerd.log" >&2; exit 2; fi
  sleep 0.1
done
nerdctl --version
containerd --version
runc --version | head -n 1
caddy version
node --version
echo "Pinned fixture tools: $proof_tools"
"${proof_nerdctl[@]}" load --input "$proof_image" >/dev/null
"${proof_nerdctl[@]}" run --detach --name runtime-a --network finite --restart unless-stopped \
  --cgroup-parent "/$(basename "$proof_scratch")" \
  --publish 127.0.0.1::8080 --env PROOF_ID=runtime-a \
  --volume "$proof_scratch/fixture:/proof:ro" finite-hermes-network-proof:local >/dev/null
proof_port="$("${proof_nerdctl[@]}" port runtime-a 8080/tcp | sed 's/.*://')"
node --input-type=module - "$proof_port" <<'JS'
const port=process.argv[2];
const read=()=>fetch(`http://127.0.0.1:${port}/`,{signal:AbortSignal.timeout(1000)}).then(r=>r.json());
let before;
for(let n=0;n<100;n++) {
 try {before=await read();break;}catch {await new Promise(r=>setTimeout(r,100));}
}
if(before?.identity!=='runtime-a')throw Error('Published real container did not become ready');
await fetch(`http://127.0.0.1:${port}/exit`).then(r=>r.arrayBuffer());
let after;
for(let n=0;n<200;n++) {
 try {const candidate=await read();if(candidate.instance!==before.instance){after=candidate;break;}}catch{}
 await new Promise(r=>setTimeout(r,100));
}
if(after?.identity!=='runtime-a')throw Error('Real containerd automatic restart did not restore the same published port');
console.log(JSON.stringify({realContainerPublished:true,automaticRestartObserved:true,samePublishedPort:true,scope:'aarch64 OrbStack Linux + runc + real containerd/nerdctl/CNI; not x86_64 Kata'}));
JS
"${proof_nerdctl[@]}" inspect runtime-a | jq '.[0] | {status:.State.Status,ports:.HostConfig.PortBindings,networks:.NetworkSettings.Networks}'
"${proof_nerdctl[@]}" stop --time 1 runtime-a >/dev/null
proof_stopped_port="$("${proof_nerdctl[@]}" port runtime-a 8080/tcp | sed 's/.*://')"
[[ "$proof_stopped_port" == "$proof_port" ]] || { echo 'Stopped container lost durable port binding' >&2; exit 2; }
"${proof_nerdctl[@]}" inspect runtime-a | jq -e '.[0].HostConfig.PortBindings | . == null or . == {}' >/dev/null
"${proof_nerdctl[@]}" run --detach --name runtime-b --network finite \
  --cgroup-parent "/$(basename "$proof_scratch")" --publish 127.0.0.1::8080 \
  --env PROOF_ID=runtime-b --volume "$proof_scratch/fixture:/proof:ro" finite-hermes-network-proof:local >/dev/null
proof_other_port="$("${proof_nerdctl[@]}" port runtime-b 8080/tcp | sed 's/.*://')"
jq -n --arg saved "$proof_stopped_port" --arg allocated "$proof_other_port" \
  '{stoppedMetadataPreservesBinding:true,nativeRandomAllocatorReusesStoppedBinding:($saved==$allocated),savedPort:$saved,newPort:$allocated}'
"${proof_nerdctl[@]}" rm --force runtime-b >/dev/null
proof_explicit_port=30001
[[ "$proof_explicit_port" != "$proof_stopped_port" ]] || exit 2
"${proof_nerdctl[@]}" run --detach --name runtime-b --network finite \
  --cgroup-parent "/$(basename "$proof_scratch")" --publish "127.0.0.1:$proof_explicit_port:8080" \
  --env PROOF_ID=runtime-b --volume "$proof_scratch/fixture:/proof:ro" finite-hermes-network-proof:local >/dev/null
"${proof_nerdctl[@]}" start runtime-a >/dev/null
node --input-type=module - "$proof_stopped_port" "$proof_explicit_port" <<'JS'
for(const [port,identity] of [[process.argv[2],'runtime-a'],[process.argv[3],'runtime-b']]) {
 let ready=false;
 for(let n=0;n<100;n++) {
  try {const body=await fetch(`http://127.0.0.1:${port}/`,{signal:AbortSignal.timeout(300)}).then(r=>r.json());
   if(body.identity!==identity)throw Error('Wrong runtime on a separately reserved port');
   ready=true;break;
  }catch(error){if(error.message==='Wrong runtime on a separately reserved port')throw error;}
  await new Promise(r=>setTimeout(r,50));
 }
 if(!ready)throw Error('Separately reserved runtime did not become ready');
}
console.log(JSON.stringify({stoppedBindingReader:'nerdctl port',liveInspectBindingEmptyWhileStopped:true,
 explicitOtherPortExcludesStoppedBinding:true,bothIdentitiesCorrectAfterRestart:true}));
JS
"${proof_nerdctl[@]}" rm --force runtime-b >/dev/null
"${proof_nerdctl[@]}" rm --force runtime-a >/dev/null

# Bounded stress can discover an orphan task/publication; a pass is not an
# atomicity proof. Keep the real default ten-second restart monitor interval.
node --input-type=module - "$proof_scratch" "${proof_nerdctl[@]}" <<'JS'
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {basename} from 'node:path';
const run=promisify(execFile);
const scratch=process.argv[2];
const [binary,...args]=process.argv.slice(3);
const nerd=async(...extra)=>(await run(binary,[...args,...extra],{timeout:30000})).stdout.trim();
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const port='30000';
const delays=[0,10,50,100,250,500,1000,2500,5000,7500,9900,10000];
let reusedAfterRemoval=0;
let automaticRestartsBeforeRemoval=0;
const ctrArgs=['--address',`${scratch}/containerd.sock`,'--namespace','finite-hermes-proof'];
const ctr=async(...extra)=>(await run('ctr',[...ctrArgs,...extra],{timeout:30000})).stdout.trim();
for(let i=0;i<delays.length;i++) {
 const name=`crash-remove-${i}`;
 const id=await nerd('run','--detach','--name',name,'--network','finite','--restart','unless-stopped',
  '--cgroup-parent',`/${basename(scratch)}`,'--publish',`127.0.0.1:${port}:8080`,
  '--env',`PROOF_ID=${name}`,'--volume',`${scratch}/fixture:/proof:ro`,'finite-hermes-network-proof:local');
 let ready=false;
 let originalInstance;
 for(let n=0;n<100;n++) {
  try {const r=await fetch(`http://127.0.0.1:${port}/`,{signal:AbortSignal.timeout(300)}).then(r=>r.json());
   if(r.identity===name){ready=true;originalInstance=r.instance;break;}}catch{}
  await pause(50);
 }
 if(!ready)throw Error(`Stress container ${i} did not become ready`);
 // nerdctl kill marks unless-stopped explicitly stopped. Direct ctr sends
 // the real crash signal without disabling containerd's restart policy.
 await ctr('tasks','kill','--signal','SIGKILL',id);
 await pause(delays[i]);
 const info=JSON.parse(await ctr('containers','info',id));
 const labels=info.Labels ?? info.labels;
 if(labels['containerd.io/restart.policy']!=='unless-stopped' ||
    labels['containerd.io/restart.status']!=='running' ||
    ![undefined,'false'].includes(labels['containerd.io/restart.explicitly-stopped']))
  throw Error('Stress lost its active unless-stopped restart policy');
 let restartedBeforeRemoval=false;
 try {const r=await fetch(`http://127.0.0.1:${port}/`,{signal:AbortSignal.timeout(300)}).then(r=>r.json());
  if(r.identity!==name)throw Error('Wrong runtime before removal');
  restartedBeforeRemoval=r.instance!==originalInstance;
 }catch(error){if(error.message==='Wrong runtime before removal')throw error;}
 if(restartedBeforeRemoval)automaticRestartsBeforeRemoval++;
 await nerd('rm','--force',name);
 const all=await nerd('ps','--all','--quiet');
 if(all.split('\n').some(row=>row && (id.startsWith(row)||row.startsWith(id))))throw Error('Removed metadata survived');
 const tasks=(await run('ctr',['--address',`${scratch}/containerd.sock`,'--namespace','finite-hermes-proof','tasks','list','--quiet'])).stdout.trim();
 if(tasks.split('\n').includes(id))throw Error('Task survived successful metadata removal');
 const nat=(await run('iptables',['-t','nat','-S'])).stdout;
 if(nat.includes(`--dport ${port} `))throw Error('Publication survived successful metadata removal');
 // A new identity must be the one reached when the released port is reused.
 await nerd('run','--detach','--name',`reuse-${i}`,'--network','finite',
  '--cgroup-parent',`/${basename(scratch)}`,'--publish',`127.0.0.1:${port}:8080`,
  '--env',`PROOF_ID=reuse-${i}`,'--volume',`${scratch}/fixture:/proof:ro`,'finite-hermes-network-proof:local');
 for(let n=0;n<100;n++) {
  try {const r=await fetch(`http://127.0.0.1:${port}/`,{signal:AbortSignal.timeout(300)}).then(r=>r.json());
   if(r.identity!==`reuse-${i}`)throw Error('Removed runtime was reached after port reassignment');
   reusedAfterRemoval++;break;
  }catch(error){if(error.message==='Removed runtime was reached after port reassignment')throw error;}
  if(n===99)throw Error('Replacement publication never became ready');
  await pause(50);
 }
 await nerd('rm','--force',`reuse-${i}`);
 process.stdout.write(JSON.stringify({crashRemoveIteration:i+1,delayMs:delays[i],restartPolicyStillActive:true,
  restartedBeforeRemoval,portReassignedToCorrectIdentity:true})+'\n');
}
await pause(11000);
if(await nerd('ps','--all','--quiet'))throw Error('Container metadata appeared after stress cleanup');
const tasks=(await run('ctr',['--address',`${scratch}/containerd.sock`,'--namespace','finite-hermes-proof','tasks','list','--quiet'])).stdout.trim();
if(tasks)throw Error('Orphan task appeared after restart monitor interval');
const finalNat=(await run('iptables',['-t','nat','-S'])).stdout;
if(finalNat.includes(`--dport ${port} `))throw Error('Orphan publication appeared after restart monitor interval');
if(!automaticRestartsBeforeRemoval)throw Error('Stress did not observe an automatic restart before removal');
console.log(JSON.stringify({ordinaryCrashRemoveStress:{iterations:delays.length,reusedAfterRemoval,automaticRestartsBeforeRemoval,orphanObserved:false},
 claimLimit:'A passed timing stress is not an atomic deletion/restart proof; no Kata guest or Caddy barrier exercised here'}));
JS
