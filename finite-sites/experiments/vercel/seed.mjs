// Four content publications, zero platform deployments. Synthetic Git only.
import {readFile,writeFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {state,root,call} from './operator.mjs';
function run(script,...args){const result=spawnSync(process.execPath,[`${root}/${script}`,...args],{encoding:'utf8'});if(result.status!==0)throw new Error(result.stderr);return JSON.parse(result.stdout);}
const fixture=run('fixtures.mjs');
for(const site of ['alpha','beta'])await call({op:'site.create',site});
const before=await call({op:'site.status',site:'alpha'}),publications=[];
for(const [site,index] of [['alpha',0],['beta',0],['alpha',1],['beta',1]]){
 const betaBefore=await call({op:'site.status',site:'beta'});
 const receipt=run('publish.mjs',site,fixture.repository,fixture.commits[index]);
 const betaAfter=await call({op:'site.status',site:'beta'});
 if(site==='alpha'&&betaBefore.active_version!==betaAfter.active_version)throw new Error('Alpha publication changed Beta');
 if(receipt.platformDeployment!==before.deployment)throw new Error('Platform deployment changed');
 publications.push({site,fixtureVersion:index+1,version:receipt.version,deployment:receipt.platformDeployment});
 console.log(`Published ${site} version ${index+1}`);
}
const beta=await call({op:'site.status',site:'beta'});
await call({op:'version.activate',site:'beta',version:beta.versions.find(v=>v.source_commit===fixture.commits[0]).id,expectedVersion:beta.active_version});
const after=await call({op:'site.status',site:'alpha'});
if(before.deployment!==after.deployment)throw new Error('Platform deployment changed during publication');
for(const site of ['alpha','beta'])await call({op:'grant.set',site,email:'viewer@example.invalid',allowed:false});
await writeFile(`${root}/evidence/single-project-publication.json`,JSON.stringify({at:new Date().toISOString(),beforeDeployment:before.deployment,afterDeployment:after.deployment,publications,final:{alpha:2,beta:1}},null,2));
console.log('Alpha v2, Beta v1; both private. Platform deployment unchanged.');
