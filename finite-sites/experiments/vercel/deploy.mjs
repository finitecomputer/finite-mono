// Deploy ONLY shared platform code. Site publication never invokes this script.
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { root, state, secrets, vc } from './operator.mjs';
const config=JSON.parse(await readFile(join(root,'config.json'),'utf8'));
const env=await secrets();
const app=join(root,'app');
for (const [key,value] of Object.entries({
  POC_ADMIN_TOKEN:env.POC_ADMIN_TOKEN, POC_ISSUER_TOKEN:env.POC_ISSUER_TOKEN,
  POC_CONTROL_HOST:config.controlHost, POC_SITE_BASE_DOMAIN:config.siteBaseDomain,
  POC_SITE_HOSTS:JSON.stringify(config.siteHosts),
})) {
  // Preview keeps Vercel protection, but needs the same synthetic state for probes.
  if (value) vc(['env','add',key,'production','--force'],value,app);
}
// Register production domains so Vercel protection distinguishes them from previews.
for (const name of [config.controlHost,...Object.values(config.siteHosts)]) {
  const current=JSON.parse(vc(['api',`/v9/projects/${config.projectId}/domains`],undefined,app));
  if (!current.domains.some(d=>d.name===name)) vc(['api',`/v10/projects/${config.projectId}/domains`,'--method','POST','--input','-'],JSON.stringify({name}),app);
}
const output=vc(['deploy','--yes','--prod'],undefined,app);
let deployment;
try { deployment=JSON.parse(output).deployment.url; }
catch { deployment=output.trim().split('\n').findLast(line=>/^https:\/\//.test(line)); }
if (!deployment) throw new Error('Inspect project: deployment response did not include a URL');
for (const hostname of [config.controlHost,...Object.values(config.siteHosts)]) {
  vc(['alias','set',deployment,hostname],undefined,app);
}
const receipt={project:config.project,projectId:config.projectId,deployment,controlHost:config.controlHost,siteHosts:config.siteHosts,deployedAt:new Date().toISOString()};
await writeFile(join(state,'platform.json'),JSON.stringify(receipt,null,2));
await writeFile(join(state,'operator.env'),Object.entries({...env,POC_CONTROL_URL:`https://${config.controlHost}/api/control`}).map(([k,v])=>`${k}=${v}`).join('\n')+'\n',{mode:0o600});
console.log(JSON.stringify(receipt,null,2));
