// Supplemental proof against the real scratch database and private Blob store.
import assert from 'node:assert/strict';
import { createServer, request as httpRequest } from 'node:http';
import { readFile, writeFile } from 'node:fs/promises';
import { parseEnv } from 'node:util';
import { join } from 'node:path';
import { neon } from './app/node_modules/@neondatabase/serverless/index.mjs';
import { head } from './app/node_modules/@vercel/blob/dist/index.js';
import handler from './app/api/index.js';
import { COOKIE,hash,blobKey } from './app/lib/model.js';
import { root, call, secrets } from './operator.mjs';
const config=JSON.parse(await readFile(join(root,'config.json')));
Object.assign(process.env,parseEnv(await readFile(join(root,'app/.env.local'),'utf8')),await secrets(),{
 POC_CONTROL_HOST:config.controlHost,POC_SITE_HOSTS:JSON.stringify(config.siteHosts),POC_SITE_BASE_DOMAIN:config.siteBaseDomain,
});
const sql=neon(process.env.DATABASE_URL),email='storage-proof@example.invalid';
const server=createServer(handler);await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const local=`http://127.0.0.1:${server.address().port}`, checks=[];
async function check(name,fn){await fn();checks.push(name);console.log(`PASS ${name}`);}
async function request(host,path='/',headers={},body){
 return new Promise((resolve,reject)=>{
  const req=httpRequest(local+path,{method:body?'POST':'GET',headers:{Host:host,...headers}},res=>{
   res.resume();res.on('end',()=>resolve({status:res.statusCode,headers:new Headers(res.headers)}));
  });req.on('error',reject);req.end(body);
 });
}
try {
 const status=await call({op:'site.status',site:'alpha'}),host=new URL(status.origin).host;
 await check('unknown and generated hostnames fail closed, even with forwarded tenant headers',async()=>{
  for(const hostname of ['unknown.vercel.app','alpha.attacker.test',status.deployment])assert.equal((await request(hostname,'/',{'x-forwarded-host':host,'x-tenant-id':'alpha'})).status,404);
 });
 await check('registered suffix still requires a real Site record',async()=>{
  process.env.POC_SITE_BASE_DOMAIN='poc.example.invalid';
  assert.equal((await request('missing.poc.example.invalid')).status,404);
  process.env.POC_SITE_BASE_DOMAIN=config.siteBaseDomain;
 });
 await check('private Blob cannot be read directly without credentials',async()=>{
  const [file]=await sql`SELECT path,sha256,size FROM poc2_files WHERE site_id='alpha' AND version_id=${status.active_version} LIMIT 1`;
  const blob=await head(blobKey('alpha',status.active_version,file));
  const r=await fetch(blob.url,{redirect:'manual'});await r.arrayBuffer();assert.ok([401,403,404].includes(r.status));
 });
 await check('maximum-size upload, duplicate blob paths, and interrupted-upload replay',async()=>{
  const bytes=Buffer.alloc(1024*1024,65),digest=hash(bytes);
  const version=await call({op:'version.begin',site:'alpha',commit:hash('upload-retry-proof').slice(0,40),deployPath:'site',files:[{path:'index.html',sha256:digest,size:bytes.length},{path:'copy.txt',sha256:digest,size:bytes.length}]});
  const upload={op:'file.put',site:'alpha',version:version.version,path:'index.html',base64:bytes.toString('base64')};
  await call(upload);await call(upload);
  // Model a crash after Blob accepted bytes, before uploaded=true was recorded.
  await sql`UPDATE poc2_files SET uploaded=false WHERE site_id='alpha' AND version_id=${version.version} AND path='index.html'`;
  await call(upload);
  await call({...upload,path:'copy.txt'});
  await call({op:'version.complete',site:'alpha',version:version.version});
  assert.equal((await call({op:'site.status',site:'alpha'})).active_version,status.active_version);
 });
 await call({op:'grant.set',site:'alpha',email,allowed:true});
 const expired=await call({op:'viewer.issue',site:'alpha',verified_email:email},'issuer');
 await sql`UPDATE poc2_handoffs SET expires_at=now()-interval '1 second' WHERE token_hash=${hash(expired.proof)}`;
 await check('expired handoff cannot create a session',async()=>{
  assert.equal((await request(host,'/_finite/redeem',{Origin:status.origin,'Content-Type':'application/x-www-form-urlencoded'},`proof=${expired.proof}`)).status,403);
 });
 const issued=await call({op:'viewer.issue',site:'alpha',verified_email:email},'issuer');
 const redeemed=await request(host,'/_finite/redeem',{Origin:status.origin,'Content-Type':'application/x-www-form-urlencoded'},`proof=${issued.proof}`);
 assert.equal(redeemed.status,303);const cookie=redeemed.headers.get('set-cookie').split(';')[0];
 await check('fresh handler reads persisted version and session',async()=>assert.equal((await request(host,'/',{Cookie:cookie})).status,200));
 await sql`UPDATE poc2_sessions SET expires_at=now()-interval '1 second' WHERE token_hash=${hash(cookie.slice(COOKIE.length+1))}`;
 await check('expired session fails closed',async()=>assert.equal((await request(host,'/',{Cookie:cookie})).status,403));
 await check('unavailable database fails closed',async()=>{
  const saved=process.env.DATABASE_URL;process.env.DATABASE_URL='';
  try{assert.equal((await request(host,'/',{Cookie:cookie})).status,503);}finally{process.env.DATABASE_URL=saved;}
 });
 await writeFile(join(root,'evidence/single-project-storage.json'),JSON.stringify({at:new Date().toISOString(),checks},null,2));
}finally{
 await call({op:'grant.set',site:'alpha',email,allowed:false});
 server.closeAllConnections();await new Promise(resolve=>server.close(resolve));
}
