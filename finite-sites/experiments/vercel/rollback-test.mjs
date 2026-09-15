import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { call, state, vc } from './operator.mjs';

const fixture = JSON.parse(await readFile(join(state,'fixture.json'),'utf8'));
const receipts = await Promise.all(fixture.commits.map(commit => readFile(join(state,'sites/alpha',`receipt-${commit}.json`),'utf8').then(JSON.parse)));
const [v1,v2] = receipts;
const alpha = v1.origin;
const beta = 'https://finite-sites-poc-beta.vercel.app';
const email = 'viewer@example.invalid';
const convergence = [];
async function login(site,origin) {
  await call({op:'grant.set',site,email,allowed:true});
  const {proof} = await call({op:'viewer.issue',site,verified_email:email},'issuer');
  const response = await fetch(origin+'/_finite/redeem',{method:'POST',redirect:'manual',headers:{Origin:origin,'Content-Type':'application/x-www-form-urlencoded'},body:new URLSearchParams({proof})});
  assert.equal(response.status,303);
  return response.headers.get('set-cookie').split(';')[0];
}
async function page(origin,cookie,status,version) {
  const response = await fetch(origin,{headers:{Cookie:cookie},redirect:'manual'});
  assert.equal(response.status,status);
  const body = await response.text();
  if(version) assert.ok(body.includes(`Content version ${version}`));
}
async function waitForVersion(origin,cookie,version) {
  const start=performance.now();
  // Vercel's completed control-plane operation can precede the new alias at
  // this edge. Bound content convergence separately; never retry a denial test.
  for(let attempt=0;attempt<30;attempt++) {
    const response=await fetch(origin,{headers:{Cookie:cookie},redirect:'manual'});
    assert.equal(response.status,200);
    if((await response.text()).includes(`Content version ${version}`)) {
      convergence.push({version,ms:Math.round(performance.now()-start),attempts:attempt+1});
      return;
    }
    await new Promise(resolve=>setTimeout(resolve,1000));
  }
  throw new Error(`Alias did not converge to version ${version} within 30 checks`);
}
const alphaCookie = await login('alpha',alpha);
const betaCookie = await login('beta',beta);
await waitForVersion(alpha,alphaCookie,2);
await page(beta,betaCookie,200,1);
await page(beta,alphaCookie,403);
await call({op:'grant.set',site:'alpha',email,allowed:false});
vc(['rollback',v1.deployment,'--yes','--cwd',join(state,'sites/alpha'),'--scope','alexlwn123-s-team']);
try {
  await page(alpha,alphaCookie,403);
  await page(beta,betaCookie,200,1);
  await call({op:'grant.set',site:'alpha',email,allowed:true});
  await waitForVersion(alpha,alphaCookie,1);
  await call({op:'grant.set',site:'alpha',email,allowed:false});
  await page(alpha,alphaCookie,403);
} finally {
  // Put the experiment back on its latest gate/content after proving rollback.
  vc(['promote',v2.deployment,'--yes','--cwd',join(state,'sites/alpha'),'--scope','alexlwn123-s-team']);
  await call({op:'grant.set',site:'alpha',email,allowed:false});
  await call({op:'grant.set',site:'beta',email,allowed:false});
}
await page(alpha,alphaCookie,403);
const result={passed:true,at:new Date().toISOString(),from:v2.deployment,to:v1.deployment,restored:v2.deployment,
  convergence,
  checks:['alpha v2 and beta v1 independent','alpha session rejected on beta','revocation survives rollback','beta stays available during alpha rollback','explicit regrant reads v1','revocation survives promotion back to v2']};
await writeFile(join(state,'rollback-evidence.json'),JSON.stringify(result,null,2));
console.log(JSON.stringify(result,null,2));
