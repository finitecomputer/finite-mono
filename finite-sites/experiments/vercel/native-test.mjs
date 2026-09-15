import assert from 'node:assert/strict';
import {readFile,writeFile} from 'node:fs/promises';
import {randomUUID} from 'node:crypto';
import {hash} from './app/lib/model.js';
import {fixture,sign,fsite,nativeShare,nativeLogin,callExchange} from './native-client.mjs';
import {call,secrets,root,state} from './operator.mjs';
const f=await fixture(),status=await call({op:'site.status',site:f.site}),checks=[];
const check=async(name,fn)=>{await fn();checks.push(name);console.log(`PASS ${name}`);};
const api=async(role,path,body,mutate)=>{
 const url='https://finite-sites-poc.vercel.app'+path,raw=body===undefined?'':JSON.stringify(body),method=body===undefined?'GET':'POST';
 const signed=sign(role,method,url,raw);return fetch(url,{method,headers:{Authorization:signed.authorization,'Content-Type':'application/json'},...(method==='POST'?{body:mutate?mutate(raw):raw}:{})});
};
let cookie;
try{
 await check('real fsite registration, config replay and status',async()=>{
  assert.equal(fsite('owner',['auth','register']).pubkey,f.identities.owner.pubkey);
  for(let i=0;i<2;i++)assert.equal(fsite('owner',['project','init','--config',`${state}/managed-source/finite.toml`]).slug,f.site);
  assert.equal(fsite('owner',['project','status',f.site]).git_remote_url,`https://github.com/${f.repo}.git`);
 });
 await check('another native identity cannot inspect or change this Project',async()=>{
  assert.equal((await api('stranger','/api/v2/projects/gamma')).status,403);
  assert.equal((await api('stranger','/api/v2/projects/gamma/site/sharing',{add_npubs:[f.identities.stranger.npub]})).status,403);
 });
 await check('signed body alteration rejected; exact whitespace preserved',async()=>{
  assert.equal((await api('owner','/api/v2/projects/gamma/site/sharing',{add_npubs:[]},raw=>raw+' ')).status,401);
  const url='https://finite-sites-poc.vercel.app/api/v2/projects/gamma/site/sharing',body=' { "add_npubs": [] }\n';
  const signed=sign('owner','POST',url,body);assert.equal((await fetch(url,{method:'POST',headers:{Authorization:signed.authorization,'Content-Type':'application/json'},body})).status,200);
 });
 await nativeShare(false);
 await check('unshared native viewer cannot obtain a handoff',async()=>assert.rejects(()=>nativeLogin()));
 await check('native share mutation replay is safe',async()=>{await nativeShare(true);const result=await nativeShare(true);assert.equal(result.shared_npubs.filter(n=>n===f.identities.viewer.npub).length,1);});
 const url=await nativeLogin();
 await check('native callback establishes host-only session exactly once',async()=>{
  const responses=await Promise.all([fetch(url,{redirect:'manual'}),fetch(url,{redirect:'manual'})]);assert.deepEqual(responses.map(r=>r.status).sort(),[303,403]);
  const value=responses.find(r=>r.status===303).headers.get('set-cookie');assert.match(value,/^__Host-/);assert.match(value,/Secure/);assert.doesNotMatch(value,/Domain=/);cookie=value.split(';')[0];
 });
 await check('native viewer reads HTML and assets; cannot cross Site boundary',async()=>{
  for(const path of ['/','/assets/style.css'])assert.equal((await fetch(status.origin+path,{headers:{Cookie:cookie}})).status,200);
  assert.equal((await fetch('https://wild-alpha.sites-poc.lwn.lol/',{headers:{Cookie:cookie}})).status,403);
 });
 await check('native nonce replay cannot produce another handoff',async()=>{
  const body=JSON.stringify({purpose:'finite_site_view_session',return_to:'/',client:'finite-dashboard',nonce:randomUUID()});const signed=sign('viewer','POST',status.origin+'/_finite/auth/native-session',body);
  const envelope={output_url:status.origin+'/',signed_body:body,authorization:signed.authorization};await callExchange(envelope);await assert.rejects(()=>callExchange(envelope));
 });
 const pending=await nativeLogin();
 await check('revoke invalidates existing native session and pending handoff',async()=>{
  await nativeShare(false);await nativeShare(false);
  assert.equal((await fetch(status.origin+'/',{headers:{Cookie:cookie}})).status,403);assert.equal((await fetch(pending,{redirect:'manual'})).status,403);
 });
 await check('content rollback does not restore native access',async()=>{
  const older=status.versions.find(v=>v.ready&&v.id!==status.active_version);assert.ok(older,'second managed Git publish required');
  await call({op:'version.activate',site:f.site,version:older.id,expectedVersion:status.active_version});
  assert.equal((await fetch(status.origin+'/assets/private.txt',{headers:{Cookie:cookie}})).status,403);
  await call({op:'version.activate',site:f.site,version:status.active_version,expectedVersion:older.id});
 });
 await check('scoped Git publisher cannot grant access or publish Alpha',async()=>{
  const env=await secrets();for(const body of [{op:'grant.set',site:'gamma',email:'bad@example.invalid',allowed:true},{op:'site.status',site:'wild-alpha'}]){
   const r=await fetch(env.POC_CONTROL_URL,{method:'POST',headers:{Authorization:`Bearer ${f.publisherToken}`,'Content-Type':'application/json'},body:JSON.stringify(body)});assert.equal(r.status,401);
  }
 });
 await check('managed publication cannot activate without a complete source bundle',async()=>{
  const env=await secrets();const publish=async body=>{const r=await fetch(env.POC_CONTROL_URL,{method:'POST',headers:{Authorization:`Bearer ${f.publisherToken}`,'Content-Type':'application/json'},body:JSON.stringify({site:f.site,...body})});return {status:r.status,body:await r.json()};};
  const bytes='source-required';const begin=await publish({op:'version.begin',commit:hash('missing-source-proof').slice(0,40),deployPath:'site',files:[{path:'index.html',sha256:hash(bytes),size:bytes.length}]});assert.equal(begin.status,200);
  assert.equal((await publish({op:'file.put',version:begin.body.version,path:'index.html',base64:Buffer.from(bytes).toString('base64')})).status,200);
  assert.equal((await publish({op:'version.complete',version:begin.body.version})).status,409);
  assert.equal((await publish({op:'source.put',version:begin.body.version,sha256:hash(bytes),base64:Buffer.from(bytes).toString('base64')})).status,400);
  assert.equal((await call({op:'site.status',site:f.site})).active_version,status.active_version);
 });
 await check('publisher admission revocation blocks native and Git publishing authority',async()=>{
  await call({op:'publisher.set',site:f.site,pubkey:f.identities.owner.pubkey,allowed:false});
  try{assert.equal((await api('owner','/api/v2/projects/gamma')).status,403);const env=await secrets();assert.equal((await fetch(env.POC_CONTROL_URL,{method:'POST',headers:{Authorization:`Bearer ${f.publisherToken}`,'Content-Type':'application/json'},body:JSON.stringify({op:'site.status',site:f.site})})).status,401);}
  finally{await call({op:'publisher.set',site:f.site,pubkey:f.identities.owner.pubkey,allowed:true});}
 });
 // Keep the owner shared for an authorized recovery test; revoked viewer stays absent.
 fsite('owner',['project','share',f.site,'--add-npub',f.identities.owner.npub]);
 await writeFile(`${root}/evidence/fidelity-native.json`,JSON.stringify({at:new Date().toISOString(),site:f.site,origin:status.origin,checks},null,2));
}finally{await nativeShare(false);}
