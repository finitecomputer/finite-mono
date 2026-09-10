#!/usr/bin/env node
// Real native Hermes protocol proof. Run in the repo Nix shell. No provider
// requests, real user state, browser shim, or production mutations are involved.
// --via-caddy exercises /runtime-a through the sibling real Caddy fixture.
// --hold-browser keeps a loopback-only actual-browser test page alive.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { realpathSync } from 'node:fs';
import { mkdtemp, mkdir, readFile, writeFile, rm, readdir, chmod } from 'node:fs/promises';
import { createServer } from 'node:net';
import { createServer as createHttpServer } from 'node:http';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

export const HERMES_REVISION = '29112bef099274229cadff79cdff7bf7b99c4b77';
const secret = () => randomBytes(32).toString('base64url');

async function removeSyntheticHome(directory) {
  // Hermes copies bundled skills with their immutable Nix directory modes.
  // Change only real directories below our private scratch root; never follow
  // symlinks into packaged skills or another home.
  await chmod(directory, 0o700);
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (entry.isDirectory()) await removeSyntheticHome(join(directory, entry.name));
  }
  await rm(directory, { recursive: true, force: true });
}

async function availablePort() {
  const server = createServer();
  await new Promise((resolve, reject) => server.once('error', reject).listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}

export async function startHermes({ binary, publicUrl = 'http://hermes-proof.invalid', orphanReapGraceSeconds } = {}) {
  if (!binary) {
    const output = execFileSync('nix', ['build', '.#hermes-agent', '--no-link', '--print-out-paths'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
    binary = join(output.trim().split('\n').at(-1), 'bin/hermes');
  }
  // Refuse an accidentally selected old runtime, without importing or patching it.
  assert.match(await readFile(binary, 'utf8'), new RegExp(HERMES_REVISION), 'Hermes wrapper must identify the repo pin');
  const directory = await mkdtemp('/tmp/hosted-hermes-auth-');
  const home = join(directory, 'hermes-home');
  await mkdir(home, { mode: 0o700 });
  await writeFile(join(home, 'config.yaml'), 'dashboard:\n  basic_auth: {}\nplugins:\n  disabled: [dashboard_auth/nous]\n', { mode: 0o600 });
  const credentials = { username: `proof-${randomBytes(6).toString('hex')}`, password: secret() };
  const port = await availablePort();
  const logPath = join(directory, 'hermes.log');
  let output = '';
  const child = spawn(binary, ['serve', '--host', '127.0.0.1', '--port', String(port), '--no-open'], {
    cwd: directory,
    env: {
      PATH: process.env.PATH,
      HOME: directory,
      LANG: 'en_US.UTF-8',
      HERMES_HOME: home,
      HERMES_DASHBOARD_PUBLIC_URL: publicUrl,
      HERMES_DASHBOARD_BASIC_AUTH_USERNAME: credentials.username,
      HERMES_DASHBOARD_BASIC_AUTH_PASSWORD: credentials.password,
      HERMES_DASHBOARD_BASIC_AUTH_SECRET: secret(),
      HERMES_DASHBOARD_BASIC_AUTH_TTL_SECONDS: '3600',
      ...(orphanReapGraceSeconds === undefined ? {} : { HERMES_TUI_WS_ORPHAN_REAP_GRACE_S: String(orphanReapGraceSeconds) }),
      HERMES_NO_AUTO_UPDATE: '1',
      XDG_CACHE_HOME: join(directory, 'cache'),
      XDG_CONFIG_HOME: join(directory, 'config'),
      XDG_STATE_HOME: join(directory, 'state'),
      PYTHONDONTWRITEBYTECODE: '1',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });
  let spawnError;
  child.on('error', (error) => { spawnError = error; });
  const baseUrl = `http://127.0.0.1:${port}`;
  const close = async ({ keep = false } = {}) => {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill('SIGTERM');
      await Promise.race([new Promise((resolve) => child.once('exit', resolve)), delay(5000)]);
      if (child.exitCode === null && child.signalCode === null) {
        const exited = new Promise((resolve) => child.once('exit', resolve));
        child.kill('SIGKILL');
        await exited;
      }
    }
    await writeFile(logPath, output, { mode: 0o600 });
    if (!keep) await removeSyntheticHome(directory);
  };
  try {
    for (let attempt = 0; attempt < 120; attempt++) {
      if (spawnError || child.exitCode !== null) throw new Error('Hermes failed during startup; inspect only the isolated log');
      const response = await fetch(`${baseUrl}/api/auth/providers`, { signal: AbortSignal.timeout(1000) }).catch(() => null);
      if (response?.ok && (await response.json()).providers?.some((provider) => provider.name === 'basic')) {
        return { baseUrl, credentials, directory, home, logPath, close, logs: () => output };
      }
      await delay(500);
    }
    throw new Error('Hermes did not advertise its native basic auth provider');
  } catch (error) {
    await close({ keep: true });
    throw new Error(`${error.message}; isolated logs: ${logPath}`);
  }
}

// A server-side cookie jar deliberately never crosses into the WS client.
// No browser storage, proxy, or bespoke ticket implementation is involved.
export async function probeAuth({ baseUrl, credentials, expiry = true, onProgress = () => {} }) {
  const base = baseUrl.replace(/\/$/, '');
  const expectedPath = new URL(base).pathname.replace(/\/$/, '') || '/';
  const checks = [];
  const secrets = [credentials.password];
  let cookie = '';
  let loginCount = 0;
  const pass = (name, detail = {}) => { checks.push({ name, ...detail }); onProgress(name); };
  const request = (path, options = {}) => fetch(`${base}${path}`, { redirect: 'manual', signal: AbortSignal.timeout(10000), ...options });
  const rejectTicketMint = async (headers = {}) => {
    const response = await request('/api/auth/ws-ticket', { method: 'POST', headers });
    assert.equal(response.status, 401, 'Unauthenticated ticket mint must fail closed');
    assert.equal(Object.hasOwn(await response.json(), 'ticket'), false);
  };
  await rejectTicketMint();
  await rejectTicketMint({ Cookie: 'hermes_session_at=not-a-session' });
  pass('anonymous and forged-cookie ticket mint rejected');

  const protectedResponse = await request('/api/config');
  assert.equal(protectedResponse.status, 401, 'Anonymous config read must be rejected');
  assert.equal((await protectedResponse.text()).includes(credentials.password), false);
  pass('anonymous config read rejected without secret disclosure');

  const wrong = await request('/auth/password-login', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ provider: 'basic', username: credentials.username, password: secret() }),
  });
  assert.equal(wrong.status, 401);
  assert.equal(wrong.headers.getSetCookie().some((entry) => entry.includes('session_at=')), false);
  pass('wrong password rejected without session cookie');

  const login = await request('/auth/password-login', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ provider: 'basic', ...credentials }),
  });
  loginCount++;
  assert.equal(login.status, 200, 'Native password login must succeed');
  assert.equal((await login.json()).ok, true);
  const cookies = login.headers.getSetCookie();
  const access = cookies.find((entry) => entry.split('=', 1)[0].endsWith('hermes_session_at'));
  assert.ok(access, 'Native login must set an access-token cookie');
  assert.match(access, /; HttpOnly/i);
  assert.ok(access.includes(`Path=${expectedPath}`), 'Cookie must respect the route prefix');
  if (base.startsWith('https:')) assert.match(access, /; Secure/i);
  cookie = cookies.map((entry) => entry.split(';')[0]).join('; ');
  secrets.push(...cookies.map((entry) => entry.split(';')[0].slice(entry.indexOf('=') + 1)));
  pass('native password login returns server-held HttpOnly cookies', { cookiePath: expectedPath });

  const mint = async () => {
    const response = await request('/api/auth/ws-ticket', { method: 'POST', headers: { Cookie: cookie } });
    assert.equal(response.status, 200, 'Existing session must mint a WS ticket');
    const result = await response.json();
    assert.equal(typeof result.ticket, 'string');
    assert.equal(result.ttl_seconds, 30);
    secrets.push(result.ticket);
    return result;
  };
  const wsUrl = `${base.replace(/^http/, 'ws')}/api/ws`;
  const exchange = (ticket, { reject = false } = {}) => new Promise((resolve, rejectPromise) => {
    // The ticket subprotocol is native Hermes. Only its stable public protocol
    // may be reflected in the server handshake; the secret is never a URL.
    const protocols = ticket ? ['hermes-gateway-v1', `hermes-gateway-ticket.${ticket}`] : [];
    const ws = new WebSocket(wsUrl, protocols);
    let done = false;
    let received = false;
    const timer = setTimeout(() => finish(new Error('WS auth probe timed out')), 15000);
    function finish(error, value) {
      if (done) return;
      done = true;
      clearTimeout(timer);
      if (ws.readyState === WebSocket.OPEN) ws.close();
      if (error) rejectPromise(error); else resolve(value);
    }
    ws.addEventListener('open', () => {
      if (reject) return finish(new Error('An unauthorized WS upgrade was accepted'));
      if (ws.protocol !== 'hermes-gateway-v1') return finish(new Error('WS reflected an unexpected subprotocol'));
      ws.send(JSON.stringify({ jsonrpc: '2.0', id: 'proof-list', method: 'session.list', params: { limit: 10 } }));
    });
    ws.addEventListener('message', (event) => {
      received = true;
      if (reject) return finish(new Error('An unauthorized WS received a frame'));
      try {
        const message = JSON.parse(event.data);
        if (message.id === 'proof-list') {
          assert.equal(message.error, undefined, 'Native session.list returned an RPC error');
          assert.ok(Array.isArray(message.result?.sessions));
          finish(null, { sessionCount: message.result.sessions.length });
        }
      } catch { finish(new Error('WS returned an invalid session.list response')); }
    });
    ws.addEventListener('error', () => finish(reject && !received ? null : new Error('WS handshake/transport failed')));
    ws.addEventListener('close', () => {
      if (!done) finish(reject && !received ? null : new Error('WS closed before session.list completed'));
    });
  });
  await exchange(undefined, { reject: true });
  await exchange(secret(), { reject: true });
  pass('missing and invalid WS tickets rejected without frames');
  const first = await mint();
  const listed = await exchange(first.ticket);
  pass('ticket-only direct WS returns native session.list', listed);
  await exchange(first.ticket, { reject: true });
  pass('consumed ticket cannot be replayed');
  await exchange((await mint()).ticket);
  await exchange((await mint()).ticket);
  assert.equal(loginCount, 1);
  pass('fresh tickets reconnect twice from one retained session without relogin');
  if (expiry) {
    const expiring = await mint();
    onProgress('waiting 32 seconds for real native ticket expiration');
    await delay((expiring.ttl_seconds + 2) * 1000);
    await exchange(expiring.ticket, { reject: true });
    await exchange((await mint()).ticket);
    pass('unconsumed ticket expires; original session still mints working replacement');
  }
  await rejectTicketMint();
  pass('mint remains authenticated after successful client connections');
  return { checks, loginCount, mintTicket: mint, containsCredential: (text) => secrets.some((value) => value.length > 12 && text.includes(value)) };
}

// Optional, explicitly local test fixture for actual browser Origin/WS checks.
// It is not a product broker: there is no WorkOS/Core implementation here.
// Only short-lived native tickets reach the browser; the password and retained
// Hermes cookies stay in the proof process. The runtime contains no user data.
export async function startBrowserProof({ baseUrl, mintTicket }) {
  const wsUrl = `${baseUrl.replace(/\/$/, '').replace(/^http/, 'ws')}/api/ws`;
  const nonce = secret();
  let browserUrl;
  const server = createHttpServer(async (request, response) => {
    response.setHeader('Cache-Control', 'no-store');
    response.setHeader('Referrer-Policy', 'no-referrer');
    if (request.url === '/ticket' && request.method === 'POST') {
      if (request.headers.origin !== browserUrl) {
        response.writeHead(403).end();
        return;
      }
      try {
        const result = await mintTicket();
        response.setHeader('Content-Type', 'application/json');
        response.end(JSON.stringify(result));
      } catch { response.writeHead(502).end('Native ticket mint failed'); }
      return;
    }
    if (request.url !== '/' || request.method !== 'GET') {
      response.writeHead(404).end();
      return;
    }
    response.setHeader('Content-Type', 'text/html; charset=utf-8');
    response.setHeader('Content-Security-Policy', `default-src 'none'; script-src 'nonce-${nonce}'; connect-src 'self' ${new URL(wsUrl).origin}`);
    response.end(`<!doctype html><meta charset="utf-8"><title>Native Hermes browser proof</title>
<h1>Native Hermes browser proof</h1><p>Isolated synthetic runtime; local test fixture, not the product broker.</p>
<pre id="result">Connecting…</pre><button id="reconnect">Reconnect with a fresh ticket</button>
<script nonce="${nonce}">
const result = document.querySelector('#result');
async function connect() {
  result.textContent = 'Minting a fresh native ticket…';
  try {
    const response = await fetch('/ticket', {method: 'POST'});
    if (!response.ok) throw new Error('Ticket mint rejected');
    const {ticket} = await response.json();
    const socket = new WebSocket(${JSON.stringify(wsUrl)}, ['hermes-gateway-v1', 'hermes-gateway-ticket.' + ticket]);
    const timer = setTimeout(() => { socket.close(); result.textContent = 'FAIL: WebSocket timeout'; }, 15000);
    socket.onopen = () => socket.send(JSON.stringify({jsonrpc:'2.0', id:'browser-proof', method:'session.list', params:{limit:10}}));
    socket.onerror = () => { clearTimeout(timer); result.textContent = 'FAIL: browser WebSocket rejected'; };
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      if (message.id !== 'browser-proof') return;
      clearTimeout(timer);
      result.textContent = Array.isArray(message.result?.sessions) && socket.protocol === 'hermes-gateway-v1'
        ? JSON.stringify({status:'PASS', browserOrigin:location.origin, gateway:${JSON.stringify(wsUrl)}, protocol:socket.protocol, sessionCount:message.result.sessions.length, cookieOrPasswordInBrowser:false}, null, 2)
        : 'FAIL: native session.list response';
      socket.close();
    };
  } catch { result.textContent = 'FAIL: native browser auth flow'; }
}
document.querySelector('#reconnect').onclick = connect;
connect();
</script>`);
  });
  await new Promise((resolve, reject) => server.once('error', reject).listen(0, '127.0.0.1', resolve));
  browserUrl = `http://127.0.0.1:${server.address().port}`;
  return { browserUrl, close: () => new Promise((resolve) => server.close(resolve)) };
}

if (process.argv[1] && import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href) {
  let hermes;
  let browser;
  let caddy;
  let success = false;
  try {
    const viaCaddy = process.argv.includes('--via-caddy');
    const prefix = viaCaddy ? '/runtime-a' : '';
    hermes = await startHermes({ binary: process.env.HERMES_PROOF_BINARY, publicUrl: `http://hermes-proof.invalid${prefix}` });
    let baseUrl = hermes.baseUrl;
    if (viaCaddy) {
      const { startCaddy } = await import('./hosted-hermes-routing.mjs');
      caddy = await startCaddy({ routes: [{ prefix, upstream: new URL(hermes.baseUrl).host }] });
      baseUrl = `${caddy.baseUrl}${prefix}`;
    }
    const report = await probeAuth({ baseUrl, credentials: hermes.credentials, onProgress: (name) => console.log(`PASS/PROGRESS: ${name}`) });
    const logsContainCredentials = report.containsCredential(hermes.logs());
    console.log(JSON.stringify({ hermesRevision: HERMES_REVISION, transport: 'loopback HTTP and native WS; no browser/Desktop exercised', prefix, checks: report.checks, loginCount: report.loginCount, rawHermesLogsContainCredentials: logsContainCredentials }, null, 2));
    if (process.argv.includes('--hold-browser')) {
      browser = await startBrowserProof({ baseUrl, mintTicket: report.mintTicket });
      console.log(`Browser proof ready: ${browser.browserUrl}`);
      await new Promise((resolve) => { process.once('SIGINT', resolve); process.once('SIGTERM', resolve); });
    }
    success = true;
  } catch (error) {
    console.error(`Auth proof failed: ${error.message}`);
    if (hermes) console.error(`Inspect isolated logs only: ${hermes.logPath}`);
    process.exitCode = 1;
  } finally {
    if (browser) await browser.close();
    if (caddy) await caddy.close();
    if (hermes) await hermes.close({ keep: !success });
  }
}
