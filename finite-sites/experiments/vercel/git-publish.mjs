// Copied into the private Git fixture. Requires only Node and Git, never builds
// customer code. POC_PUBLISHER_TOKEN authorizes exactly one prebound Site.
import {execFileSync} from 'node:child_process';
import {readFileSync,mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createHash} from 'node:crypto';
const site=process.env.POC_SITE,endpoint=process.env.POC_CONTROL_URL,credential=process.env.POC_PUBLISHER_TOKEN;
if(!site||!endpoint||!credential)throw new Error('Missing scoped publisher configuration');
const hash=b=>createHash('sha256').update(b).digest('hex');
const git=args=>execFileSync('git',args,{maxBuffer:16*1024*1024});
const commit=git(['rev-parse','HEAD']).toString().trim(),deployPath='site';
const entries=git(['ls-tree','-rz','--full-tree',commit,'--',deployPath]).toString().split('\0').filter(Boolean);
if(!entries.length||entries.length>200)throw new Error('Manifest limit');
let total=0;
const files=entries.map(entry=>{
 const i=entry.indexOf('\t'),[mode,kind,id]=entry.slice(0,i).split(' '),path=entry.slice(i+1).slice(deployPath.length+1);
 if(!['100644','100755'].includes(mode)||kind!=='blob'||!path||!(/^[a-zA-Z0-9_./@+ -]+$/).test(path)||path.startsWith('_finite')||path.split('/').some(p=>!p||p==='..'||(p.startsWith('.')&&p!=='.well-known')))throw new Error('Invalid committed file');
 const bytes=git(['cat-file','blob',id]);total+=bytes.length;
 if(bytes.length>1048576||total>8*1048576)throw new Error('Content limit');
 return {path,sha256:hash(bytes),size:bytes.length,bytes};
});
if(!files.some(f=>f.path==='index.html'))throw new Error('Missing index.html');
const api=async body=>{
 const response=await fetch(endpoint,{method:'POST',redirect:'error',signal:AbortSignal.timeout(60000),headers:{Authorization:`Bearer ${credential}`,'Content-Type':'application/json'},body:JSON.stringify({site,...body})});
 if(!response.ok)throw new Error(`Publisher request failed (${response.status}): ${(await response.json()).error}`);
 return response.json();
};
const directory=mkdtempSync(join(tmpdir(),'sites-source-'));
try{
 const sourcePath=join(directory,'source.bundle');git(['bundle','create',sourcePath,'HEAD']);git(['bundle','verify',sourcePath]);
 const source=readFileSync(sourcePath);if(source.length>2*1048576)throw new Error('Source bundle limit: 2 MiB');
 const before=await api({op:'site.status'});
 const {version}=await api({op:'version.begin',commit,deployPath,files:files.map(({path,sha256,size})=>({path,sha256,size}))});
 for(const file of files)await api({op:'file.put',version,path:file.path,base64:file.bytes.toString('base64')});
 await api({op:'source.put',version,sha256:hash(source),base64:source.toString('base64')});
 await api({op:'version.complete',version});
 await api({op:'version.activate',version,expectedVersion:before.active_version});
 console.log(JSON.stringify({site,commit,version,sourceSha256:hash(source),platformDeployment:before.deployment}));
}finally{rmSync(directory,{recursive:true,force:true});}
