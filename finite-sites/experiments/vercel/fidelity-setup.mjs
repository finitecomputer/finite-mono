// Prepare the declared disposable native/Git fixture. No production identity.
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {execFileSync,spawnSync} from 'node:child_process';
import {randomBytes} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {root,state,call} from './operator.mjs';
const repoRoot=fileURLToPath(new URL('../../../',import.meta.url));
let fixture;
try{fixture=JSON.parse(await readFile(`${state}/fidelity.json`));}
catch(error){if(error.code!=='ENOENT')throw error;fixture={site:'gamma',repo:'alexlwn123/finite-sites-poc-source',publisherToken:randomBytes(32).toString('hex'),identities:{}};}
for(const role of ['owner','viewer','stranger']){
 const home=`${state}/native/${role}`;await mkdir(home,{recursive:true,mode:0o700});
 fixture.identities[role]=JSON.parse(execFileSync(`${repoRoot}/target/debug/examples/sites_poc_sign`,[],{env:{...process.env,FINITE_HOME:home},encoding:'utf8'}));
}
await writeFile(`${state}/fidelity.json`,JSON.stringify(fixture,null,2),{mode:0o600});
await call({op:'publisher.set',site:fixture.site,pubkey:fixture.identities.owner.pubkey,allowed:true});
await call({op:'site.create',site:fixture.site});
await call({op:'project.bind',site:fixture.site,owner:fixture.identities.owner.pubkey,publisherToken:fixture.publisherToken,gitRemote:`https://github.com/${fixture.repo}.git`,branch:'main',deployPath:'site'});
const remote=JSON.parse(execFileSync('gh',['repo','view',fixture.repo,'--json','isPrivate,nameWithOwner'],{encoding:'utf8'}));
if(!remote.isPrivate||remote.nameWithOwner!==fixture.repo)throw new Error('Expected the declared private fixture repository');
const secret=spawnSync('gh',['secret','set','POC_PUBLISHER_TOKEN','--repo',fixture.repo],{input:fixture.publisherToken,encoding:'utf8'});
if(secret.status)throw new Error('Could not configure the scoped publisher credential');
try{await readFile(`${state}/managed-source/finite.toml`);}
catch(error){if(error.code!=='ENOENT')throw error;execFileSync('git',['clone',`git@github.com:${fixture.repo}.git`,`${state}/managed-source`],{stdio:'pipe'});}
console.log('Native identities, Site binding, and managed Git fixture ready. Existing owner bindings cannot be silently replaced.');
