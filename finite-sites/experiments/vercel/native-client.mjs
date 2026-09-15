import {readFile} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {root,state,call} from './operator.mjs';
const repo=fileURLToPath(new URL('../../../',import.meta.url));
export async function fixture(){return JSON.parse(await readFile(`${state}/fidelity.json`));}
export function sign(role,method,url,body=''){
 return JSON.parse(execFileSync(`${repo}/target/debug/examples/sites_poc_sign`,[method,url],{env:{...process.env,FINITE_HOME:`${state}/native/${role}`},input:body,encoding:'utf8'}));
}
export function fsite(role,args){return JSON.parse(execFileSync(`${repo}/target/debug/fsite`,[...args,'--output','json'],{env:{...process.env,FINITE_HOME:`${state}/native/${role}`,FINITE_SITES_API:'https://finite-sites-poc.vercel.app'},encoding:'utf8',stdio:['pipe','pipe','pipe']}));}
export async function nativeShare(allowed){const f=await fixture();return fsite('owner',['project','share',f.site,allowed?'--add-npub':'--remove-npub',f.identities.viewer.npub]);}
export async function nativeLogin(role='viewer'){
 const f=await fixture(),status=await call({op:'site.status',site:f.site});
 const body=JSON.stringify({purpose:'finite_site_view_session',return_to:'/',client:'finite-dashboard',nonce:randomUUID()});
 const proof=sign(role,'POST',status.origin+'/_finite/auth/native-session',body);
 // Existing dashboard/Hosted Device wire shape. Only this loopback bridge holds
 // the prototype exchange credential; no account cookies/production keys copied.
 const result=await callExchange({output_url:status.origin+'/',authorization:proof.authorization,signed_body:body});
 return result.redeem_url;
}
export async function callExchange(body){
 const {secrets}=await import('./operator.mjs');const env=await secrets();
 const r=await fetch('https://finite-sites-poc.vercel.app/internal/v1/native-viewer-sessions',{method:'POST',headers:{Authorization:`Bearer ${env.POC_ISSUER_TOKEN}`,'Content-Type':'application/json'},body:JSON.stringify(body)});
 if(!r.ok)throw new Error(`Native viewer exchange denied (${r.status})`);return r.json();
}
