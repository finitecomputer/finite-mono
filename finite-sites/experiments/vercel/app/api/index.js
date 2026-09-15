import {database} from '../lib/database.js';
import { get } from '@vercel/blob';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import {nativeApi,exchangeNative,issueNative,redeemNative} from '../lib/native.js';
import { control } from '../lib/control.js';
import { COOKIE, hash, token, validToken, validPath, resolveHost, blobKey, contentType, requireThat } from '../lib/model.js';

export default async function handler(req, res) {
  for (const key of ['Cache-Control','CDN-Cache-Control','Vercel-CDN-Cache-Control']) res.setHeader(key,'private, no-store');
  res.setHeader('X-Content-Type-Options','nosniff');
  res.setHeader('Referrer-Policy','same-origin');
  const json = (status, body) => { res.statusCode=status; res.setHeader('Content-Type','application/json'); res.end(JSON.stringify(body)); };
  try {
    const route = resolveHost(req.headers.host);
    requireThat(route,404,'unknown_host');
    const url = new URL(req.url, `https://${route.host}`);
    const sql = database();
    if (route.control) {
      if (req.method === 'GET' && url.pathname === '/') return json(200,{ experiment:'finite-sites-single-project', hosting:'private Blob + managed Postgres', deployment:process.env.VERCEL_URL });
      if(url.pathname.startsWith('/api/v2/'))return json(200,await nativeApi(sql,url.pathname,req.method,req.method==='GET'?'':await readBody(req,3000000),req.headers.authorization,`https://${route.host}`));
      if(url.pathname==='/internal/v1/native-viewer-sessions'){
        requireThat(req.method==='POST',405,'method_not_allowed');
        return json(200,await exchangeNative(sql,await readBody(req,16384),(req.headers.authorization||'').replace(/^Bearer /,'')));
      }
      requireThat(url.pathname === '/api/control',404,'not_found');
      requireThat(req.method === 'POST',405,'method_not_allowed');
      // This API accepts bearer credentials, never ambient cookies. Cross-origin
      // browser calls cannot pass the JSON preflight; Origin is checked as well.
      requireThat(!req.headers.origin || req.headers.origin === `https://${route.host}`,403,'invalid_origin');
      const body = await readBody(req, 3000000);
      const result = await control(sql, JSON.parse(body), (req.headers.authorization || '').replace(/^Bearer /,''));
      return json(200,result);
    }
    const rows = await sql`SELECT id,hostname,disabled,active_version FROM poc2_sites WHERE id=${route.site} AND hostname=${route.host}`;
    requireThat(rows.length,404,'unknown_site');
    const site = rows[0];
    requireThat(!site.disabled,403,'access_denied');
    if(url.pathname==='/_finite/auth/native-session'){
      requireThat(req.method==='POST',405,'method_not_allowed');
      requireThat(!req.headers.origin||req.headers.origin===url.origin,403,'invalid_origin');
      const result=await issueNative(sql,site,req.headers.authorization,await readBody(req,4096));
      const redemption=new URL(result.redeem_url);
      const session=await redeemNative(sql,site,redemption.searchParams.get('native_token'),redemption.searchParams.get('return_to'));
      res.statusCode=303;res.setHeader('Location',redemption.searchParams.get('return_to'));
      res.setHeader('Set-Cookie',`${COOKIE}=${session}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=3600`);return res.end();
    }
    if(url.pathname==='/_finite/auth'){
      requireThat(req.method==='GET',405,'method_not_allowed');
      const session=await redeemNative(sql,site,url.searchParams.get('native_token'),url.searchParams.get('return_to'));
      res.statusCode=303;res.setHeader('Location',url.searchParams.get('return_to'));res.setHeader('Referrer-Policy','no-referrer');
      res.setHeader('Set-Cookie',`${COOKIE}=${session}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=3600`);return res.end();
    }
    if (url.pathname === '/_finite/login' && req.method === 'GET') {
      const nonce=token();
      res.setHeader('Content-Type','text/html; charset=utf-8');
      res.setHeader('Content-Security-Policy',`default-src 'none'; script-src 'nonce-${nonce}'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'`);
      return res.end(`<!doctype html><meta charset="utf-8"><title>Finite experiment sign-in</title><h1>Finite Sites experiment</h1><p>Use the local console to sign in with the synthetic viewer.</p><form method="post" action="/_finite/redeem"><input name="proof" type="hidden"></form><script nonce="${nonce}">const proof=location.hash.slice(1);history.replaceState(null,'','/_finite/login');if(/^[a-f0-9]{64}$/.test(proof)){document.querySelector('input').value=proof;document.querySelector('form').submit();}</script>`);
    }
    if (url.pathname === '/_finite/redeem') {
      requireThat(req.method === 'POST',405,'method_not_allowed');
      requireThat(req.headers.origin === `https://${route.host}`,403,'invalid_origin');
      const raw = await readBody(req,256);
      const proof = new URLSearchParams(raw).get('proof');
      requireThat(validToken(proof),403,'invalid_proof');
      const session=token();
      const redeemed = await sql`WITH consumed AS (
        UPDATE poc2_handoffs h SET consumed_at=now()
        WHERE h.token_hash=${hash(proof)} AND h.site_id=${site.id} AND h.hostname=${route.host}
        AND h.expires_at>now() AND h.consumed_at IS NULL
        AND EXISTS (SELECT 1 FROM poc2_grants g WHERE g.site_id=h.site_id AND g.email=h.email)
        RETURNING h.site_id,h.hostname,h.email
      ) INSERT INTO poc2_sessions (token_hash,site_id,hostname,email,expires_at)
        SELECT ${hash(session)},site_id,hostname,email,now()+interval '1 hour' FROM consumed RETURNING site_id`;
      requireThat(redeemed.length,403,'invalid_or_used_proof');
      res.statusCode=303; res.setHeader('Location','/');
      res.setHeader('Set-Cookie',`${COOKIE}=${session}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=3600`);
      return res.end();
    }
    requireThat(['GET','HEAD'].includes(req.method),405,'method_not_allowed');
    const cookies=(req.headers.cookie || '').split(';').map(c=>c.trim()).filter(c=>c.startsWith(`${COOKIE}=`));
    requireThat(cookies.length === 1 && validToken(cookies[0].slice(COOKIE.length+1)),403,'access_denied');
    const authorized = await sql`SELECT v.site_id FROM poc2_sessions v JOIN poc2_grants g ON v.site_id=g.site_id AND v.email=g.email
      WHERE v.token_hash=${hash(cookies[0].slice(COOKIE.length+1))} AND v.site_id=${site.id} AND v.hostname=${route.host} AND v.expires_at>now()
      UNION ALL SELECT v.site_id FROM poc2_native_sessions v JOIN poc2_native_grants g ON v.site_id=g.site_id AND v.pubkey=g.pubkey
      WHERE v.token_hash=${hash(cookies[0].slice(COOKIE.length+1))} AND v.site_id=${site.id} AND v.hostname=${route.host} AND v.expires_at>now()`;
    requireThat(authorized.length,403,'access_denied');
    let path=decodeURIComponent(url.pathname).slice(1);
    if (!path || path.endsWith('/')) path+='index.html';
    requireThat(validPath(path),404,'not_found');
    const files = await sql`SELECT path,sha256,size FROM poc2_files WHERE site_id=${site.id} AND version_id=${site.active_version} AND path=${path} AND uploaded`;
    requireThat(files.length,404,'not_found');
    const file=files[0];
    res.setHeader('Content-Type',contentType(file.path));
    res.setHeader('X-Finite-Version',site.active_version);
    res.setHeader('X-Finite-Deployment',process.env.VERCEL_URL || 'local');
    // HEAD/conditional/range requests follow the SAME gate. Private responses
    // never use CDN caching or 304, and Range currently receives the full file.
    if (req.method === 'HEAD') return res.end();
    const blob=await get(blobKey(site.id,site.active_version,file),{access:'private'});
    requireThat(blob?.statusCode === 200,503,'content_unavailable');
    await pipeline(Readable.fromWeb(blob.stream),res);
  } catch (error) {
    if (res.headersSent) return res.destroy();
    json(error.status || 503,{error:error.status ? error.message : 'service_unavailable'});
  }
}
async function readBody(req, limit) {
  const chunks=[];let size=0;
  for await(const chunk of req){size+=chunk.length;requireThat(size<=limit,413,'body_too_large');chunks.push(chunk);}
  return Buffer.concat(chunks).toString();
}
