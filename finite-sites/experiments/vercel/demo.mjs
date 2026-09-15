import { createServer } from 'node:http';
import { call } from './operator.mjs';

const origin = 'http://127.0.0.1:4319';
const email = 'viewer@example.invalid';
const siteOrigin = 'https://finite-sites-poc-alpha.vercel.app';
const html = `<!doctype html><meta charset="utf-8"><title>Finite Sites / Experiment Console</title>
<style>body{font:18px/1.6 system-ui;background:#f4f5ef;color:#20352b;max-width:700px;margin:70px auto;padding:24px}h1{font-size:42px;font-weight:500}button,a{font:inherit}button{padding:12px 20px;background:#20352b;color:white;border:0;border-radius:4px;cursor:pointer}form{display:inline-block;margin:6px}small{display:block;margin-top:28px}</style>
<p>FINITE / EXPERIMENT</p><h1>Private sites, hosted by Vercel.</h1>
<p>This local console acts as a synthetic verified-email issuer for <b>viewer@example.invalid</b>. It holds the experiment operator credentials on this machine.</p>
<ol><li>Grant access and open the private site.</li><li>Return here and revoke access.</li><li>Refresh the site or open its asset URL. Both should be denied.</li></ol>
<form method="post" action="/grant"><button>Grant access</button></form>
<form method="post" action="/login" target="_blank"><button>Open private site</button></form>
<form method="post" action="/revoke"><button>Revoke access</button></form>
<p><a href="${siteOrigin}" target="_blank">Site</a> · <a href="${siteOrigin}/assets/private.txt" target="_blank">Private asset</a></p>
<small>Disposable data only. No production identity, DNS, Git remote, or Sites service is changed. Stop this console with Ctrl-C.</small>`;
createServer(async (req,res) => {
  res.setHeader('Cache-Control','no-store');
  res.setHeader('Referrer-Policy','same-origin');
  res.setHeader('X-Content-Type-Options','nosniff');
  if (req.headers.host !== '127.0.0.1:4319') { res.writeHead(403); return res.end(); }
  if (req.method === 'GET' && req.url === '/') { res.setHeader('Content-Type','text/html; charset=utf-8'); return res.end(html); }
  if (req.method !== 'POST' || req.headers.origin !== origin || !['/grant','/revoke','/login'].includes(req.url)) { res.writeHead(403); return res.end('Denied'); }
  try {
    if (req.url === '/login') {
      const { proof } = await call({ op:'viewer.issue',site:'alpha',verified_email:email },'issuer');
      res.writeHead(303,{ Location:`${siteOrigin}/_finite/login#${proof}` });
    } else {
      await call({ op:'grant.set',site:'alpha',email,allowed:req.url === '/grant' });
      res.writeHead(303,{Location:'/'});
    }
    res.end();
  } catch {
    res.writeHead(403,{'Content-Type':'text/plain'});
    res.end('Access was not granted. Return to the console and grant the synthetic viewer first.');
  }
}).listen(4319,'127.0.0.1',() => console.log(`Experiment console: ${origin}`));
