// Real DNS, trusted HTTPS and browser checks against one shared wildcard.
// Only synthetic Sites are mutated; no Vercel domains/deployments are created.
import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { randomBytes } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { resolve4 } from 'node:dns/promises';
import { get } from 'node:https';
import { chromium } from 'playwright';
import { root, state, call, secrets, vc } from './operator.mjs';
import { COOKIE } from './app/lib/model.js';

const config=JSON.parse(await readFile(`${root}/config.json`));
assert.ok(config.siteBaseDomain,'Configure a real wildcard suffix first');
const fixture=JSON.parse(await readFile(`${state}/fixture.json`));
const email='viewer@example.invalid', results=[], publications=[];
const sites=['wild-alpha','wild-beta'];
const fresh=`wild-probe-${randomBytes(6).toString('hex')}`;
const origin=site=>`https://${site}.${config.siteBaseDomain}`;
const domains=()=>JSON.parse(vc(['api',`/v9/projects/${config.projectId}/domains`],undefined,`${root}/app`)).domains.map(d=>d.name).sort();
const request=async(url,options={})=>fetch(url,{redirect:'manual',signal:AbortSignal.timeout(30000),...options});
const check=async(name,fn)=>{await fn();results.push({name,passed:true});console.log(`PASS ${name}`);};
const publish=(site,index)=>{
  const receipt=JSON.parse(execFileSync(process.execPath,[`${root}/publish.mjs`,site,fixture.repository,fixture.commits[index]],{encoding:'utf8'}));
  publications.push({site,version:receipt.version,deployment:receipt.platformDeployment});
  return receipt;
};
const beforeDomains=domains();
const control=await (await request(`https://${config.controlHost}/`)).json();
let browser;
try {
  await check('one wildcard attached; no per-Site Vercel domains',async()=>{
    assert.ok(beforeDomains.includes(`*.${config.siteBaseDomain}`));
    for(const site of [...sites,fresh]){
      assert.ok(!beforeDomains.includes(`${site}.${config.siteBaseDomain}`));
    }
  });
  const addresses=await resolve4(`${fresh}.${config.siteBaseDomain}`);
  assert.ok(addresses.length);
  let certificate;
  await check('unregistered random hostname resolves with a trusted wildcard certificate',async()=>{
    certificate=await new Promise((resolve,reject)=>{
      get(origin(fresh),{timeout:30000,agent:false},res=>{
        const cert=res.socket.getPeerCertificate();
        const trusted=res.socket.authorized;
        res.resume();
        res.on('end',()=>resolve({trusted,subjectAltName:cert.subjectaltname,validTo:cert.valid_to,status:res.statusCode}));
      }).on('timeout',function(){this.destroy(new Error('TLS probe timed out'));}).on('error',reject);
    });
    assert.equal(certificate.trusted,true);
    assert.ok(certificate.subjectAltName.split(', ').includes(`DNS:*.${config.siteBaseDomain}`));
    assert.equal(certificate.status,404);
    assert.deepEqual(await (await request(origin(fresh))).json(),{error:'unknown_site'});
  });
  await check('reserved labels reach the router but fail closed',async()=>{
    for(const site of ['api','auth','control']){
      const r=await request(origin(site));assert.equal(r.status,404);
      assert.deepEqual(await r.json(),{error:'unknown_host'});
    }
  });
  await check('fresh Site publication requires no new domain or platform deployment',async()=>{
    const receipt=publish(fresh,0);
    assert.equal(receipt.origin,origin(fresh));
    assert.equal(receipt.platformDeployment,control.deployment);
    assert.equal((await request(origin(fresh))).status,403);
    assert.deepEqual(domains(),beforeDomains);
  });
  await check('two sibling Sites publish and replay beneath the wildcard',async()=>{
    for(const site of sites){
      publish(site,0);publish(site,1);
      for(let i=0;i<2;i++)assert.equal((await call({op:'site.create',site})).origin,origin(site));
      await call({op:'grant.set',site,email,allowed:false});
      for(const path of ['/','/assets/style.css','/assets/private.txt'])assert.equal((await request(origin(site)+path)).status,403);
    }
    assert.ok(publications.every(p=>p.deployment===control.deployment));
  });
  const statuses=Object.fromEntries(await Promise.all(sites.map(async site=>[site,await call({op:'site.status',site})])));
  browser=await chromium.launch({executablePath:process.env.POC_BROWSER_PATH||chromium.executablePath(),headless:true});
  const context=await browser.newContext(); // No ignoreHTTPSErrors or DNS overrides.
  const pages={};
  await check('real browsers redeem one-use proofs and render private sibling Sites',async()=>{
    for(const site of sites){
      await call({op:'grant.set',site,email,allowed:true});
      const {proof,origin:issuedOrigin}=await call({op:'viewer.issue',site,verified_email:email},'issuer');
      assert.equal(issuedOrigin,origin(site));
      const page=await context.newPage();pages[site]=page;
      await page.goto(`${issuedOrigin}/_finite/login#${proof}`);
      await page.waitForURL(`${issuedOrigin}/`);
      await page.getByRole('heading',{name:'Content version 2'}).waitFor();
      assert.equal(await page.locator('body').evaluate(e=>getComputedStyle(e).backgroundColor),'rgb(244, 245, 239)');
      const replay=await request(`${issuedOrigin}/_finite/redeem`,{method:'POST',headers:{Origin:issuedOrigin,'Content-Type':'application/x-www-form-urlencoded'},body:`proof=${proof}`});
      assert.equal(replay.status,403);
    }
  });
  const alpha=pages['wild-alpha'], beta=pages['wild-beta'];
  const cookies=(await context.cookies()).filter(c=>c.name===COOKIE);
  await check('sibling origins have separate storage and host-only Secure HttpOnly cookies',async()=>{
    assert.equal(cookies.length,2);
    assert.deepEqual(cookies.map(c=>c.domain).sort(),sites.map(s=>new URL(origin(s)).hostname).sort());
    assert.ok(cookies.every(c=>c.secure&&c.httpOnly&&!c.domain.startsWith('.')));
    await alpha.evaluate(()=>localStorage.setItem('tenant-marker','alpha'));
    assert.equal(await beta.evaluate(()=>localStorage.getItem('tenant-marker')),null);
    const before=(await context.cookies()).filter(c=>c.name===COOKIE);
    await alpha.evaluate(({name,domain})=>{document.cookie=`${name}=${'a'.repeat(64)}; Domain=${domain}; Path=/; Secure`;},{name:COOKIE,domain:config.siteBaseDomain});
    assert.deepEqual((await context.cookies()).filter(c=>c.name===COOKIE),before,'Browser must reject parent-domain __Host- cookies');
  });
  await check('same-site sibling cannot read private content or redeem another Site proof',async()=>{
    assert.equal(await alpha.evaluate(async url=>{try{await fetch(url,{credentials:'include'});return false;}catch{return true;}},origin('wild-beta')),true);
    const {proof}=await call({op:'viewer.issue',site:'wild-beta',verified_email:email},'issuer');
    const denied=await alpha.evaluate(async({url,proof})=>{
      try{await fetch(url,{method:'POST',credentials:'include',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:`proof=${proof}`});return false;}catch{return true;}
    },{url:origin('wild-beta')+'/_finite/redeem',proof});
    assert.equal(denied,true);
    // CORS alone does not prove no mutation: the same proof must remain redeemable.
    const redeemed=await request(origin('wild-beta')+'/_finite/redeem',{method:'POST',headers:{Origin:origin('wild-beta'),'Content-Type':'application/x-www-form-urlencoded'},body:`proof=${proof}`});
    assert.equal(redeemed.status,303);
    const alphaCookie=cookies.find(c=>c.domain===new URL(origin('wild-alpha')).hostname);
    assert.equal((await request(origin('wild-beta'),{headers:{Cookie:`${COOKIE}=${alphaCookie.value}`}})).status,403);
  });
  await check('site hosts cannot impersonate the control plane',async()=>{
    const env=await secrets();
    const r=await request(origin('wild-alpha')+'/api/control',{method:'POST',headers:{Authorization:`Bearer ${env.POC_ADMIN_TOKEN}`,'Content-Type':'application/json','x-forwarded-host':config.controlHost},body:JSON.stringify({op:'site.status',site:'wild-beta'})});
    assert.equal(r.status,405);
  });
  await check('rollback changes only one sibling and retains current authorization',async()=>{
    const old=statuses['wild-alpha'].versions.find(v=>v.source_commit===fixture.commits[0]&&v.ready).id;
    const change={op:'version.activate',site:'wild-alpha',version:old,expectedVersion:statuses['wild-alpha'].active_version};
    await call(change);await call(change);
    assert.equal((await alpha.reload()).status(),200);
    await alpha.getByRole('heading',{name:'Content version 1'}).waitFor();
    await beta.reload();await beta.getByRole('heading',{name:'Content version 2'}).waitFor();
    await call({op:'grant.set',site:'wild-alpha',email,allowed:false});
    await call({op:'version.activate',site:'wild-alpha',version:statuses['wild-alpha'].active_version,expectedVersion:old});
    assert.equal((await alpha.reload()).status(),403);
    assert.equal((await alpha.goto(origin('wild-alpha')+'/assets/style.css')).status(),403);
    assert.equal((await beta.reload()).status(),200);
  });
  await check('all wildcard operations leave Vercel domain inventory and deployment unchanged',async()=>{
    assert.deepEqual(domains(),beforeDomains);
    for(const site of [...sites,fresh])assert.equal((await call({op:'site.status',site})).deployment,control.deployment);
  });
  await beta.screenshot({path:`${root}/evidence/wildcard-authorized.png`,fullPage:true});
  await writeFile(`${root}/evidence/wildcard-live.json`,JSON.stringify({at:new Date().toISOString(),project:config.project,deployment:control.deployment,wildcard:`*.${config.siteBaseDomain}`,origins:sites.map(origin),freshSite:fresh,addresses,certificate,domains:beforeDomains,publications,checks:results},null,2));
  console.log(`${results.length} wildcard checks passed.`);
} finally {
  await browser?.close();
  for(const site of [...sites,fresh]){
    try{
      await call({op:'grant.set',site,email,allowed:false});
      if(site===fresh)await call({op:'site.disable',site,disabled:true});
      else{
        const status=await call({op:'site.status',site});
        const newest=status.versions.find(v=>v.source_commit===fixture.commits[1]&&v.ready);
        if(newest&&status.active_version!==newest.id)await call({op:'version.activate',site,version:newest.id,expectedVersion:status.active_version});
      }
    }catch(error){console.error(`Cleanup ${site}: ${error.message}`);process.exitCode=1;}
  }
}
