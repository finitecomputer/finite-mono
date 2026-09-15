import assert from 'node:assert/strict';
import {readFile,writeFile,mkdtemp} from 'node:fs/promises';
import {execFileSync,spawnSync,spawn} from 'node:child_process';
import {request as httpRequest} from 'node:http';
import {randomUUID} from 'node:crypto';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {root,state} from './operator.mjs';
import {sign} from './native-client.mjs';
const run=mode=>JSON.parse(execFileSync(process.execPath,[`${root}/recovery.mjs`,mode],{encoding:'utf8'}));
run('export');const manifest=JSON.parse(await readFile(`${state}/recovery/manifest.json`));
const checks=[];let server;
const verify=async(name,fn)=>{await fn();checks.push(name);console.log(`PASS ${name}`);};
await verify('corrupt recovery object rejected before target creation',async()=>{
 const object=manifest.objects[0],path=`${state}/recovery/objects/${object.sha256}`,bytes=await readFile(path);
 try{await writeFile(path,Buffer.from('corrupt'));const result=spawnSync(process.execPath,[`${root}/recovery.mjs`,'restore'],{encoding:'utf8'});assert.notEqual(result.status,0);assert.match(result.stderr,/corrupt_recovery_object/);}finally{await writeFile(path,bytes);}
});
const restored=run('restore');
try{
 server=spawn(process.execPath,[`${root}/restored-server.mjs`],{stdio:['ignore','pipe','pipe']});
 const port=await new Promise((resolve,reject)=>{server.once('error',reject);server.stdout.once('data',chunk=>{try{resolve(JSON.parse(chunk).port);}catch(error){reject(error);}});server.once('exit',code=>reject(new Error(`Restore server exited ${code}`)));});
 const request=(path,headers={},body)=>new Promise((resolve,reject)=>{
  const req=httpRequest(`http://127.0.0.1:${port}${path}`,{method:body?'POST':'GET',headers:{Host:restored.hostname,...headers}},res=>{const chunks=[];res.on('data',c=>chunks.push(c));res.on('end',()=>resolve({status:res.statusCode,headers:res.headers,body:Buffer.concat(chunks).toString()}));});req.on('error',reject);req.end(body);
 });
 await verify('fresh process denies anonymous viewer',async()=>assert.equal((await request('/')).status,403));
 const auth=async role=>{
  const body=JSON.stringify({purpose:'finite_site_view_session',return_to:'/',client:'restore-test',nonce:randomUUID()});
  const signed=sign(role,'POST',`https://${restored.hostname}/_finite/auth/native-session`,body);
  return request('/_finite/auth/native-session',{'Content-Type':'application/json',Authorization:signed.authorization},body);
 };
 await verify('revoked native Share remains revoked after recovery',async()=>assert.equal((await auth('viewer')).status,403));
 let cookie;
 await verify('retained Share signs in against restored authority',async()=>{const r=await auth('owner');assert.equal(r.status,303);cookie=r.headers['set-cookie'][0].split(';')[0];});
 await verify('restored HTML and assets served from new Blob namespace',async()=>{
  const r=await request('/',{Cookie:cookie});assert.equal(r.status,200);assert.equal(r.headers['x-finite-version'],restored.activeVersion);assert.match(r.body,/Managed Git version 3/);
  assert.equal((await request('/assets/style.css',{Cookie:cookie})).status,200);
 });
 await verify('complete editable source and Git history restore from bundle',async()=>{
  const version=manifest.tables.poc2_versions.find(v=>v.id===restored.activeVersion),directory=await mkdtemp(join(tmpdir(),'sites-recovered-source-'));
  const bundle=`${state}/recovery/objects/${version.source_hash}`;
  execFileSync('git',['clone',bundle,`${directory}/source`],{stdio:'pipe'});
  const git=args=>execFileSync('git',['-C',`${directory}/source`,...args],{encoding:'utf8'}).trim();
  assert.equal(git(['rev-parse','HEAD']),version.source_commit);git(['fsck','--full']);
  assert.match(await readFile(`${directory}/source/README.md`,'utf8'),/complete source repository/);
  assert.ok(Number(git(['rev-list','--count','HEAD']))>=2);
 });
 await writeFile(`${root}/evidence/fidelity-recovery.json`,JSON.stringify({at:new Date().toISOString(),scope:'empty logical database and Blob namespaces; new serving process',...restored,checks},null,2));
}finally{server?.kill('SIGTERM');}
