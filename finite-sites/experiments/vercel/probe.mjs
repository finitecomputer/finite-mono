import { strict as assert } from 'node:assert';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { call, state, vc } from './operator.mjs';

const [site = 'alpha', version = '1', ...extraOrigins] = process.argv.slice(2);
if (!/^[a-z][a-z0-9-]+$/.test(site)) throw new Error('Invalid site');
const origin = `https://finite-sites-poc-${site}.vercel.app`;
const origins = [...new Set([origin, ...extraOrigins])];
const email = 'viewer@example.invalid';
const outcomes = [];
async function check(label, url, expected, options = {}, marker) {
  const start = performance.now();
  let response;
  if (new URL(url).origin !== origin) {
    // Keep Vercel Deployment Protection enabled. Authenticate through its CLI
    // to prove Finite's independent gate still runs behind that outer layer.
    const config = Object.entries(options.headers ?? {}).map(([key,value]) => `header = ${JSON.stringify(`${key}: ${value}`)}`).join('\n');
    const raw = vc(['curl', new URL(url).pathname, '--deployment', new URL(url).origin, '--', '--include', '--silent',
      ...(options.method === 'HEAD' ? ['--head'] : []), '--config', '-'], config, join(state, 'sites', site));
    const boundary = raw.search(/\r?\n\r?\n/);
    const headerText = raw.slice(0,boundary);
    const body = raw.slice(boundary).replace(/^\r?\n\r?\n/, '');
    const lines = headerText.split(/\r?\n/);
    const status = Number(lines.shift().match(/HTTP\/\S+ (\d+)/)?.[1]);
    const headers = new Headers(lines.map(line => { const i = line.indexOf(':'); return [line.slice(0,i),line.slice(i+1).trim()]; }));
    response = new Response(body, { status, headers });
  } else {
    response = await fetch(url, { ...options, redirect: 'manual', signal: AbortSignal.timeout(15000) });
  }
  const text = await response.text();
  // No response body or cookies in failure output: a broken auth boundary
  // must not leak credentials while the test is reporting its failure.
  assert.equal(response.status, expected, `${label}: unexpected status`);
  assert.match(response.headers.get('cache-control') ?? '', /no-store/, `${label}: caching enabled`);
  assert.equal(response.headers.get('x-finite-gate'), 'poc-v1', `${label}: gate did not execute`);
  if (marker) assert.ok(text.includes(marker), `${label}: expected content missing`);
  if (expected === 403) assert.ok(!text.includes('PRIVATE-ASSET') && !text.includes('Content version'), `${label}: content leaked`);
  outcomes.push({ label, status: response.status, ms: Math.round(performance.now() - start), cache: response.headers.get('x-vercel-cache') });
  return response;
}
await call({ op: 'grant.set', site, email, allowed: true });
for (const host of origins) {
  for (const path of ['/', '/index.html', '/assets/private.txt', '/assets/style.css', '/llms.txt', '/missing', '/assets%2fprivate.txt']) {
    await check(`anonymous ${host}${path}`, host + path, 403);
  }
  await check('forged middleware header', host + '/assets/private.txt', 403, { headers: { 'x-middleware-subrequest': 'middleware:middleware:middleware:middleware:middleware' } });
  await check('forged viewer header', host + '/', 403, { headers: { 'x-finite-user-email': email, 'x-finite-authorized': 'true' } });
}
const { proof } = await call({ op: 'viewer.issue', site, verified_email: email }, 'issuer');
const redeem = await check('redeem one-use proof', origin + '/_finite/redeem', 303, {
  method: 'POST', headers: { Origin: origin, 'Content-Type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams({ proof }),
});
const setCookie = redeem.headers.get('set-cookie');
assert.ok(setCookie?.includes('Secure') && setCookie.includes('HttpOnly') && setCookie.includes('SameSite=Lax'));
const cookie = setCookie.split(';')[0];
await writeFile(join(state, `session-${site}.json`), JSON.stringify({ cookie, origin }), { mode: 0o600 });
await check('proof replay rejected', origin + '/_finite/redeem', 403, {
  method: 'POST', headers: { Origin: origin, 'Content-Type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams({ proof }),
});
for (let i = 0; i < 3; i++) {
  await check('authorized HTML', origin + '/', 200, { headers: { Cookie: cookie } }, `Content version ${version}`);
  await check('authorized asset', origin + '/assets/private.txt', 200, { headers: { Cookie: cookie } }, `PRIVATE-ASSET-V${version}`);
}
// Deliberately supply the cookie to alternate deployment hosts: browser cookie
// scoping alone must not be the only thing protecting retained deployments.
for (const host of extraOrigins) await check('authorized alternate deployment', host + '/assets/private.txt', 200, { headers: { Cookie: cookie } }, 'PRIVATE-ASSET');
await call({ op: 'grant.set', site, email, allowed: false });
for (const host of origins) {
  for (const path of ['/', '/assets/private.txt', '/assets/style.css', '/llms.txt']) {
    await check(`revoked ${host}${path}`, host + path, 403, { headers: { Cookie: cookie } });
  }
  await check('revoked HEAD', host + '/assets/private.txt', 403, { method: 'HEAD', headers: { Cookie: cookie } });
  await check('revoked conditional request', host + '/assets/private.txt', 403, { headers: { Cookie: cookie, 'If-None-Match': '*' } });
}
await writeFile(join(state, `probe-${site}-v${version}.json`), JSON.stringify({ at: new Date().toISOString(), origins, outcomes }, null, 2));
console.log(JSON.stringify({ passed: outcomes.length, outcomes }, null, 2));
