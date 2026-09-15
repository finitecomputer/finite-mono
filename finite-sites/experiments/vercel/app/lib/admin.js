import { requireThat, siteHost, token, validSite } from './model.js';

// Public synthetic metadata only. Privileged administration stays in the local
// operator console; this host cannot issue sessions or reach control mutations.
export async function adminCatalog(req, res, url, sql) {
  requireThat(url.pathname === '/', 404, 'not_found');
  requireThat(['GET', 'HEAD'].includes(req.method), 405, 'method_not_allowed');
  const rows = await sql`SELECT id FROM poc2_sites
    WHERE NOT disabled AND active_version IS NOT NULL
      AND hostname = id || '.' || ${process.env.POC_SITE_BASE_DOMAIN}
    ORDER BY id LIMIT 201`;
  const sites = rows.filter(row => validSite(row.id)).slice(0, 200);
  const nonce = token();
  res.setHeader('Content-Type', 'text/html; charset=utf-8');
  res.setHeader('X-Robots-Tag', 'noindex, nofollow');
  res.setHeader('Content-Security-Policy', `default-src 'none'; style-src 'nonce-${nonce}'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'`);
  if (req.method === 'HEAD') return res.end();
  const escape = value => value.replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  res.end(`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Sites · Finite Admin</title>
<style nonce="${nonce}">
*{box-sizing:border-box}body{margin:0;background:#f4f5ef;color:#18382c;font:16px/1.5 system-ui,sans-serif}
main{max-width:880px;margin:0 auto;padding:64px 24px}header{border-bottom:1px solid #cad1c5;padding-bottom:28px;margin-bottom:28px}
.eyebrow{font-size:12px;text-transform:uppercase;letter-spacing:.14em;font-weight:650}h1{font-size:44px;letter-spacing:-.04em;margin:12px 0 8px;font-weight:600}
p{color:#536252;margin:0}ul{list-style:none;padding:0;margin:16px 0}li{border-top:1px solid #d5dacd}li:last-child{border-bottom:1px solid #d5dacd}
a{display:flex;justify-content:space-between;align-items:center;gap:20px;padding:22px 4px;color:inherit;text-decoration:none}a:hover{background:#e9edde}a:focus-visible{outline:2px solid #245f42;outline-offset:3px}
strong{font-size:19px;font-weight:600;display:block}.hostname{display:block;color:#536252;font-size:14px;overflow-wrap:anywhere}.arrow{font-size:23px}footer{margin-top:28px;font-size:13px;color:#536252}
@media(max-width:480px){main{padding:36px 20px}h1{font-size:36px}}
</style></head><body><main><header><div class="eyebrow">Finite / Admin</div><h1>Sites</h1><p>The active sites in this experiment.</p></header>
<section aria-label="Active sites"><p>${sites.length} active ${sites.length === 1 ? 'site' : 'sites'}${rows.length > 200 ? ' · Showing the first 200' : ''}</p>
${sites.length ? `<ul>${sites.map(({id}) => { const host = escape(siteHost(id)); return `<li><a href="https://${host}/"><span><strong>${escape(id)}</strong><span class="hostname">${host}</span></span><span class="arrow" aria-hidden="true">↗</span></a></li>`; }).join('')}</ul>` : '<p>No active sites yet.</p>'}
</section><footer>Read-only experiment catalog. Site content still requires Finite access.</footer></main></body></html>`);
}
