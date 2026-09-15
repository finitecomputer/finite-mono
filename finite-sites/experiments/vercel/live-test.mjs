import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { call, secrets, state, root } from './operator.mjs';
import { hash, COOKIE } from './app/lib/model.js';
const config=JSON.parse(await readFile(join(root,'config.json')));
const env=await secrets(), email='viewer@example.invalid', results=[];
const statuses=Object.fromEntries(await Promise.all(['wild-alpha','wild-beta'].map(async site=>[site,await call({op:'site.status',site})])));
const origins=Object.fromEntries(Object.entries(statuses).map(([s,v])=>[s,v.origin]));
async function check(name,fn){await fn();results.push({name,passed:true});console.log(`PASS ${name}`);}
async function api(body,role='admin'){
  const r=await fetch(env.POC_CONTROL_URL,{method:'POST',redirect:'manual',headers:{'Content-Type':'application/json',Authorization:`Bearer ${role==='issuer'?env.POC_ISSUER_TOKEN:role==='admin'?env.POC_ADMIN_TOKEN:'invalid'}`},body:JSON.stringify(body)});
  return {status:r.status,body:await r.json()};
}
async function visit(site,path='/',cookie,options={}){
  const r=await fetch(origins[site]+path,{redirect:'manual',...options,headers:{...(cookie?{Cookie:cookie}:{}),...options.headers}});
  const body=await r.text(); return {status:r.status,body,headers:r.headers};
}
async function redeem(site,proof,origin=origins[site]){return visit(site,'/_finite/redeem',null,{method:'POST',headers:{Origin:origin,'Content-Type':'application/x-www-form-urlencoded'},body:`proof=${proof}`});}
const sessions={};
try {
 await check('one platform deployment for both Sites',async()=>{assert.equal(statuses['wild-alpha'].deployment,statuses['wild-beta'].deployment);});
 for(const site of ['wild-alpha','wild-beta']){
  await check(`${site}: create replay + admin boundary`,async()=>{
   assert.equal((await api({op:'site.create',site},'issuer')).status,401);
   for(let i=0;i<2;i++)assert.equal((await api({op:'site.create',site})).status,200);
  });
  await call({op:'grant.set',site,email,allowed:false});
  for(const path of ['/','/assets/style.css','/assets/private.txt','/llms.txt','/api/index','/api/control','/_next/static/fake.js']){
   await check(`${site}: anonymous denied ${path}`,async()=>assert.equal((await visit(site,path)).status,403));
  }
  await check(`${site}: ungranted issuer denied`,async()=>assert.equal((await api({op:'viewer.issue',site,verified_email:email},'issuer')).status,403));
  await check(`${site}: grants replay + invalid mutation`,async()=>{
   for(let i=0;i<2;i++)assert.equal((await api({op:'grant.set',site,email,allowed:true})).status,200);
   assert.equal((await api({op:'grant.set',site,email,allowed:'true'})).status,400);
   assert.equal((await api({op:'grant.set',site,email,allowed:false},'issuer')).status,401);
  });
  const {proof}=await call({op:'viewer.issue',site,verified_email:email},'issuer');
  await check(`${site}: cross-origin and cross-site proof rejected`,async()=>{
   assert.equal((await redeem(site,proof,origins[site==='wild-alpha'?'wild-beta':'wild-alpha'])).status,403);
   assert.equal((await redeem(site==='wild-alpha'?'wild-beta':'wild-alpha',proof)).status,403);
  });
  await check(`${site}: proof redemption single winner and replay denied`,async()=>{
   const responses=await Promise.all([redeem(site,proof),redeem(site,proof)]);
   assert.deepEqual(responses.map(r=>r.status).sort(),[303,403]);
   const cookie=responses.find(r=>r.status===303).headers.get('set-cookie');
   assert.match(cookie,/^__Host-finite_poc=/);assert.match(cookie,/Secure/);assert.match(cookie,/HttpOnly/);assert.doesNotMatch(cookie,/Domain=/i);
   sessions[site]=cookie.split(';')[0];
   assert.equal((await redeem(site,proof)).status,403);
  });
  await check(`${site}: authorized HTML/CSS/assets, no-store, current version`,async()=>{
   for(const path of ['/','/assets/style.css','/assets/private.txt']){
    const r=await visit(site,path,sessions[site]);assert.equal(r.status,200);assert.match(r.headers.get('cache-control'),/no-store/);assert.equal(r.headers.get('x-finite-version'),statuses[site].active_version);assert.equal(r.headers.get('x-finite-deployment'),statuses[site].deployment);
   }
  });
 }
 await check('copied cookie and duplicate cookies cannot cross tenant gate',async()=>{
  assert.equal((await visit('wild-beta','/',sessions['wild-alpha'])).status,403);
  assert.equal((await visit('wild-alpha','/',`${sessions['wild-alpha']}; ${sessions['wild-alpha']}`)).status,403);
 });
 await check('forged tenant and forwarded-host headers grant no access',async()=>{
  assert.equal((await visit('wild-alpha','/',null,{headers:{'x-tenant-id':'wild-beta','x-finite-site':'wild-beta','x-forwarded-host':config.controlHost}})).status,403);
 });
 await check('control mutation rejected on site host',async()=>{
  assert.equal((await visit('wild-alpha','/api/control',sessions['wild-alpha'],{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({op:'grant.set',site:'wild-beta',email,allowed:true})})).status,405);
 });
 await check('incomplete/corrupt upload cannot replace current content',async()=>{
  const bytes=Buffer.from('Never published');
  const input={op:'version.begin',site:'wild-alpha',commit:hash('incomplete-single-project-proof').slice(0,40),deployPath:'site',files:[{path:'index.html',sha256:hash(bytes),size:bytes.length}]};
  const begun=await call(input);assert.equal((await call(input)).version,begun.version);
  assert.equal((await api({op:'file.put',site:'wild-alpha',version:begun.version,path:'index.html',base64:Buffer.from('corrupt').toString('base64')})).status,400);
  assert.equal((await api({op:'version.complete',site:'wild-alpha',version:begun.version})).status,409);
  assert.equal((await api({op:'version.activate',site:'wild-alpha',version:begun.version,expectedVersion:statuses['wild-alpha'].active_version})).status,409);
  assert.equal((await call({op:'site.status',site:'wild-alpha'})).active_version,statuses['wild-alpha'].active_version);
 });
 const fixture=JSON.parse(await readFile(join(state,'fixture.json')));
 const old=statuses['wild-alpha'].versions.find(v=>v.source_commit===fixture.commits[0]&&v.ready).id;
 await check('rollback changes Alpha only and keeps platform deployment fixed',async()=>{
  const body={op:'version.activate',site:'wild-alpha',version:old,expectedVersion:statuses['wild-alpha'].active_version};
  assert.equal((await api(body)).status,200);assert.equal((await api(body)).status,200);
  const r=await visit('wild-alpha','/',sessions['wild-alpha']);assert.equal(r.status,200);assert.match(r.body,/Content version 1/);
  const beta=await visit('wild-beta','/',sessions['wild-beta']);assert.equal(beta.headers.get('x-finite-version'),statuses['wild-beta'].active_version);
  assert.equal(r.headers.get('x-finite-deployment'),statuses['wild-alpha'].deployment);
 });
 await check('stale activation rejects lost updates; complete replay is safe',async()=>{
  assert.equal((await api({op:'version.activate',site:'wild-alpha',version:statuses['wild-alpha'].active_version,expectedVersion:hash('stale')})).status,409);
  for(let i=0;i<2;i++)assert.equal((await api({op:'version.complete',site:'wild-alpha',version:old})).status,200);
 });
 for(const site of ['wild-alpha','wild-beta']){
  const pending=await call({op:'viewer.issue',site,verified_email:email},'issuer');
  await check(`${site}: revocation replay and pending proof denial`,async()=>{
   for(let i=0;i<2;i++)await call({op:'grant.set',site,email,allowed:false});
   assert.equal((await redeem(site,pending.proof)).status,403);
  });
  for(const [path,options] of [['/',{}],['/assets/private.txt',{}],['/assets/style.css',{}],['/',{method:'HEAD'}],['/',{headers:{'If-None-Match':'*'}}],['/assets/private.txt',{headers:{Range:'bytes=0-9'}}]]){
   await check(`${site}: revoked ${options.method||Object.keys(options.headers||{}).join()||'GET'} ${path}`,async()=>assert.equal((await visit(site,path,sessions[site],options)).status,403));
  }
 }
 await check('restoring content cannot restore revoked access',async()=>{
  await call({op:'version.activate',site:'wild-alpha',version:statuses['wild-alpha'].active_version,expectedVersion:old});
  assert.equal((await visit('wild-alpha','/',sessions['wild-alpha'])).status,403);
 });
 await check('disable replay and invalid disable input',async()=>{
  await call({op:'grant.set',site:'wild-alpha',email,allowed:true});
  for(let i=0;i<2;i++)await call({op:'site.disable',site:'wild-alpha',disabled:true});
  assert.equal((await visit('wild-alpha','/',sessions['wild-alpha'])).status,403);
  assert.equal((await api({op:'viewer.issue',site:'wild-alpha',verified_email:email},'issuer')).status,403);
  assert.equal((await api({op:'site.disable',site:'wild-alpha',disabled:'false'})).status,400);
  await call({op:'site.disable',site:'wild-alpha',disabled:false});
  assert.equal((await visit('wild-alpha','/',sessions['wild-alpha'])).status,200);
 });
 await writeFile(join(root,'evidence/wildcard-lifecycle.json'),JSON.stringify({at:new Date().toISOString(),project:config.project,deployment:statuses['wild-alpha'].deployment,origins,checks:results},null,2));
 console.log(`${results.length} live checks passed.`);
}finally{
 for(const site of ['wild-alpha','wild-beta']){
  await call({op:'site.disable',site,disabled:false});
  await call({op:'grant.set',site,email,allowed:false});
  const current=await call({op:'site.status',site});
  if(current.active_version!==statuses[site].active_version)await call({op:'version.activate',site,version:statuses[site].active_version,expectedVersion:current.active_version});
 }
}
