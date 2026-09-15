import { next } from '@vercel/functions';

export const config = { runtime: 'nodejs', matcher: '/:path*' };
const COOKIE = '__Host-finite-poc';
const tokenPattern = /^[a-f0-9]{64}$/;
const headers = {
  'Cache-Control': 'private, no-store', 'CDN-Cache-Control': 'no-store',
  'Vercel-CDN-Cache-Control': 'no-store', 'Referrer-Policy': 'no-referrer',
  'X-Content-Type-Options': 'nosniff', 'X-Finite-Gate': 'poc-v1',
};
const deny = (status = 403) => new Response('Finite Sites: access required.', { status, headers });

export default async function middleware(request) {
  try {
    const control = process.env.POC_CONTROL_URL;
    if (!control || new URL(control).protocol !== 'https:' || !process.env.POC_GATE_TOKEN || !process.env.POC_SITE_ID) return deny(503);
    const call = async body => fetch(control, {
      method: 'POST', redirect: 'error', cache: 'no-store', signal: AbortSignal.timeout(5000),
      headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${process.env.POC_GATE_TOKEN}` },
      body: JSON.stringify({ ...body, site: process.env.POC_SITE_ID }),
    });
    const url = new URL(request.url);
    if (url.pathname === '/_finite/login' && request.method === 'GET') {
      const nonce = crypto.randomUUID();
      return new Response(`<!doctype html><meta charset="utf-8"><title>Finite experiment sign-in</title>
        <h1>Finite Sites experiment</h1><p>Sign in using the disposable operator demo. This is not production Finite login.</p>
        <form method="post" action="/_finite/redeem"><input name="proof" type="hidden"></form>
        <script nonce="${nonce}">const proof=location.hash.slice(1);history.replaceState(null,'','/_finite/login');
        if(/^[a-f0-9]{64}$/.test(proof)){document.querySelector('input').value=proof;document.querySelector('form').submit();}</script>`, {
        headers: { ...headers, 'Content-Type': 'text/html; charset=utf-8', 'Referrer-Policy': 'same-origin',
          'Content-Security-Policy': `default-src 'none'; script-src 'nonce-${nonce}'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'` },
      });
    }
    if (url.pathname === '/_finite/redeem') {
      // Proofs travel in POST bodies, never URLs, history or referrers.
      if (request.method !== 'POST') return deny(405);
      if (request.headers.get('origin') !== url.origin) return deny();
      const raw = await request.text();
      if (raw.length > 256) return deny(413);
      const proof = new URLSearchParams(raw).get('proof');
      if (!proof || !tokenPattern.test(proof)) return deny();
      const response = await call({ op: 'gate.redeem', proof });
      if (!response.ok) return deny(response.status === 503 ? 503 : 403);
      const result = await response.json();
      if (!tokenPattern.test(result.session)) return deny(503);
      return new Response(null, { status: 303, headers: {
        ...headers, Location: '/',
        'Set-Cookie': `${COOKIE}=${result.session}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=3600`,
      } });
    }
    if (!['GET', 'HEAD'].includes(request.method)) return deny(405);
    const cookies = (request.headers.get('cookie') ?? '').split(';').map(value => value.trim());
    const matching = cookies.filter(value => value.startsWith(`${COOKIE}=`));
    if (matching.length > 1) return deny();
    const session = matching[0]?.slice(COOKIE.length + 1);
    const response = await call({ op: 'gate.authorize', session });
    if (!response.ok) return deny(response.status === 503 ? 503 : 403);
    if ((await response.json()).allowed !== true) return deny();
    return next({ headers });
  } catch {
    return deny(503);
  }
}
