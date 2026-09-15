import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { call } from './operator.mjs';
import {nativeShare,nativeLogin} from './native-client.mjs';
const origin='http://127.0.0.1:4319', email='viewer@example.invalid';
const config=JSON.parse(await readFile(new URL('config.json',import.meta.url),'utf8'));
const html=`<!doctype html><meta charset="utf-8"><title>Finite Sites / One Project Experiment</title>
<style>body{font:18px/1.6 system-ui;background:#f4f5ef;color:#20352b;max-width:900px;margin:50px auto;padding:24px}h1{font-size:42px;font-weight:500}button,a{font:inherit}button{padding:10px 15px;background:#20352b;color:white;border:0;border-radius:4px;cursor:pointer}form{display:inline-block;margin:5px}small{display:block;margin-top:28px}.sites{display:grid;grid-template-columns:1fr 1fr;gap:24px}section{border:1px solid #bdc8b9;padding:20px;border-radius:8px}pre{font-size:12px;white-space:pre-wrap;overflow-wrap:anywhere}h2{text-transform:capitalize}</style>
<p>FINITE / EXPERIMENT</p><h1>Finite Sites. One Vercel project.</h1>
<p>Each site has its own hostname and permissions. Content is stored privately; publishing and rollback change its active version without deploying the platform.</p>
<p>Wildcard routing is live at <strong>*.${config.siteBaseDomain}</strong>. Wild-alpha and Wild-beta use that single wildcard, with no individual Vercel domain entries.</p>
<section data-site="gamma"><h2>Managed Git + Finite identity</h2><p>Publish with a Git push. Sign in with a native Finite identity. Recover content and editable source together.</p><form method="post" action="/gamma/grant"><button>Grant native viewer</button></form><form method="post" action="/gamma/login" target="_blank"><button>Open with Finite identity</button></form><form method="post" action="/gamma/revoke"><button>Revoke native viewer</button></form><p><a href="https://github.com/alexlwn123/finite-sites-poc-source" target="_blank">Private source repository</a> · <a href="https://gamma.${config.siteBaseDomain}" target="_blank">Site URL</a></p><small>Uses isolated prototype Finite identities and the real signed-request protocol. Production account sign-in is not connected.</small></section><br><div class="sites">${['wild-alpha','wild-beta'].map(site=>`<section data-site="${site}"><h2>${site}</h2><p><a href="https://${site}.${config.siteBaseDomain}" target="_blank">Open URL</a></p>${[['grant','Grant access'],['login','Open private site'],['revoke','Revoke access'],['v1','Use version 1'],['v2','Use version 2']].map(([op,label])=>`<form method="post" action="/${site}/${op}" ${op==='login'?'target="_blank"':''}><button>${label}</button></form>`).join('')}<p id="${site}-summary">Loading state…</p><details><summary>Technical state</summary><pre id="${site}"></pre></details></section>`).join('')}</div>
<small>Synthetic viewer: ${email}. Operator credentials stay on this machine. Project: ${config.project}. Wild-alpha and Wild-beta use local source fixtures; Gamma uses private managed Git. All Sites use the wildcard.</small>
<script>fetch('/status').then(r=>r.json()).then(data=>{for(const site of ['wild-alpha','wild-beta']){document.getElementById(site+'-summary').textContent='Active: version '+data[site].demoVersion+' · '+data[site].versions.filter(v=>v.ready).length+' ready versions';document.getElementById(site).textContent=JSON.stringify(data[site],null,2)}});</script>`;
createServer(async(req,res)=>{
  res.setHeader('Cache-Control','no-store');res.setHeader('Referrer-Policy','same-origin');res.setHeader('X-Content-Type-Options','nosniff');
  if(req.headers.host!=='127.0.0.1:4319'){res.writeHead(403);return res.end();}
  if(req.method==='GET'&&req.url==='/'){res.setHeader('Content-Type','text/html; charset=utf-8');return res.end(html);}
  try {
    if(req.method==='GET'&&req.url==='/status'){
      const fixture=JSON.parse(await readFile(new URL('.local-state/fixture.json',import.meta.url),'utf8'));
      const statuses=await Promise.all(['wild-alpha','wild-beta'].map(async site=>{const status=await call({op:'site.status',site});const active=status.versions.find(v=>v.id===status.active_version);return [site,{...status,demoVersion:fixture.commits.indexOf(active?.source_commit)+1}]}));
      res.setHeader('Content-Type','application/json');return res.end(JSON.stringify(Object.fromEntries(statuses)));
    }
    const match=/^\/(wild-alpha|wild-beta|gamma)\/(grant|revoke|login|v1|v2)$/.exec(req.url);
    if(req.method!=='POST'||req.headers.origin!==origin||!match){res.writeHead(403);return res.end('Denied');}
    const [,site,op]=match;
    if(site==='gamma'){
      if(op==='login')res.writeHead(303,{Location:await nativeLogin()});
      else if(op==='grant'||op==='revoke'){await nativeShare(op==='grant');res.writeHead(303,{Location:'/'});}
      else throw new Error('Use managed Git to publish this Site.');
    }else if(op==='login'){
      const {proof,origin:siteOrigin}=await call({op:'viewer.issue',site,verified_email:email},'issuer');
      res.writeHead(303,{Location:`${siteOrigin}/_finite/login#${proof}`});
    }else{
      if(op==='v1'||op==='v2'){
        const fixtures=JSON.parse(await readFile(new URL('.local-state/fixture.json',import.meta.url),'utf8'));
        const status=await call({op:'site.status',site});
        const version=status.versions.find(v=>v.source_commit===fixtures.commits[op==='v1'?0:1]&&v.ready);
        if(!version)throw new Error('Publish both fixture versions first.');
        await call({op:'version.activate',site,version:version.id,expectedVersion:status.active_version});
      }else await call({op:'grant.set',site,email,allowed:op==='grant'});
      res.writeHead(303,{Location:'/'});
    }
    res.end();
  }catch(error){res.writeHead(403,{'Content-Type':'text/plain'});res.end(error.message);}
}).listen(4319,'127.0.0.1',()=>console.log(`Experiment console: ${origin}`));
