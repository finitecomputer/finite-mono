#!/usr/bin/env node
// Real Caddy / real subprocess proof. This is not a production routing service.
// Run with the repository's Nix Node and CADDY_BIN pointing to pinned Caddy.
// Resolve/build Caddy with:
// nix build --no-link --impure --expr 'let f = builtins.getFlake (toString ./.);
//   in f.inputs.nixpkgs.legacyPackages.aarch64-darwin.caddy'
// Then: nix develop --command env CADDY_BIN=<output>/bin/caddy node <this-file>
// Exit 1 is the intentionally BLOCKED architecture verdict; exit 2 is a broken
// proof. Changing a negative control so it stops catching disclosure is not a fix.
// No credential canary is printed or written to disk. Temporary files contain
// only Caddy configuration/logs and are removed on successful cleanup.
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { execFile, fork, spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, open, readFile, rm, writeFile } from 'node:fs/promises';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const self = fileURLToPath(import.meta.url);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const deadline = (promise, ms = 5000, label = 'operation') => {
  let timer;
  return Promise.race([
    promise,
    new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label} timed out`)), ms); }),
  ]).finally(() => clearTimeout(timer));
};

function adminRequest(port, method, pathname, payload) {
  return new Promise((resolve, reject) => {
    const request = http.request({ host: '127.0.0.1', port, method, path: pathname,
      headers: payload ? { 'content-type': 'application/json' } : {}, timeout: 5000 }, (response) => {
      response.resume();
      response.on('end', () => resolve(response.statusCode));
    });
    request.on('error', reject);
    request.on('timeout', () => request.destroy(new Error('Caddy admin request timed out')));
    request.end(payload);
  });
}

async function freePort() {
  const server = net.createServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

export async function startCaddy({ routes = [], binary = process.env.CADDY_BIN, disableUpstreamKeepalive = false,
  forbiddenLogValues = [] } = {}) {
  assert(binary, 'CADDY_BIN must identify the repository-pinned Caddy executable');
  const edgePort = await freePort();
  const adminPort = await freePort();
  assert.notEqual(edgePort, adminPort);
  const directory = await mkdtemp(path.join(os.tmpdir(), 'finite-hermes-caddy-'));
  const logPath = path.join(directory, 'caddy.log');
  const log = await open(logPath, 'w', 0o600);
  const config = (entries) => ({
    admin: { listen: `127.0.0.1:${adminPort}` },
    logging: { logs: { default: { level: 'ERROR' } } },
    apps: { http: { servers: { proof: {
      listen: [`127.0.0.1:${edgePort}`],
      automatic_https: { disable: true },
      routes: [
        ...entries.map(({ prefix, upstream }) => {
          assert.match(prefix, /^\/[a-z0-9-]+$/);
          assert.match(upstream, /^127\.0\.0\.1:[1-9][0-9]*$/);
          return { match: [{ path: [prefix, `${prefix}/*`] }], terminal: true, handle: [{
            handler: 'subroute', routes: [{ handle: [
              { handler: 'rewrite', strip_path_prefix: prefix },
              { handler: 'reverse_proxy', upstreams: [{ dial: upstream }],
                ...(disableUpstreamKeepalive ? { transport: { protocol: 'http', keep_alive: { enabled: false } } } : {}),
                headers: { request: { set: { 'X-Forwarded-Prefix': [prefix] } } } },
            ] }],
          }] };
        }),
        { handle: [{ handler: 'static_response', status_code: 404, body: 'No published runtime route' }] },
      ],
    } } } },
  });
  const configPath = path.join(directory, 'caddy.json');
  await writeFile(configPath, JSON.stringify(config(routes)), { mode: 0o600 });
  const child = spawn(binary, ['run', '--config', configPath], {
    env: { ...process.env, XDG_CONFIG_HOME: directory, XDG_DATA_HOME: directory },
    stdio: ['ignore', log.fd, log.fd],
  });
  const exited = once(child, 'exit');
  let closed = false;
  let stopResult;
  const close = async () => {
    if (closed) return stopResult;
    closed = true;
    let forced = false;
    if (child.exitCode === null && child.signalCode === null) {
      child.kill('SIGTERM');
      try { await deadline(exited, 5000, 'Caddy shutdown'); }
      catch { forced = true; child.kill('SIGKILL'); await exited; }
    }
    stopResult = { exitCode: child.exitCode, signal: child.signalCode, forced };
    await log.close();
    const logText = await readFile(logPath, 'utf8');
    await rm(directory, { recursive: true, force: true });
    assert(!forbiddenLogValues.some((value) => value && logText.includes(value)),
      'Ephemeral Caddy logs contained a full canary credential (value withheld)');
    return stopResult;
  };
  try {
    for (let attempt = 0; attempt < 100; attempt++) {
      if (child.exitCode !== null || child.signalCode !== null) throw new Error('Caddy exited before readiness');
      try {
        if (await adminRequest(adminPort, 'GET', '/config/') === 200) break;
      } catch { /* process not yet listening */ }
      if (attempt === 99) throw new Error('Caddy admin readiness timed out');
      await sleep(50);
    }
  } catch (error) { await close(); throw error; }
  return {
    baseUrl: `http://127.0.0.1:${edgePort}`, edgePort, logPath,
    async updateRoutes(entries) {
      const status = await adminRequest(adminPort, 'POST', '/load', JSON.stringify(config(entries)));
      assert.equal(status, 200, 'Caddy must acknowledge configuration acceptance');
    },
    close,
  };
}

function backendChild() {
  let canary = '';
  let identity = '';
  const held = new Map();
  const heldSockets = new Set();
  const sockets = new Set();
  const emit = (event) => process.send?.(event);
  const server = http.createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = Buffer.concat(chunks).toString();
    const sawCanary = body.includes(canary) || Object.values(request.headers).some((value) => String(value).includes(canary));
    emit({ event: 'request', identity, url: request.url, prefix: request.headers['x-forwarded-prefix'], sawCanary, peerPort: request.socket.remotePort });
    const finish = () => {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ identity, url: request.url, prefix: request.headers['x-forwarded-prefix'], sawCanary }));
    };
    if (request.url === '/hold' && identity === 'original-runtime') { held.set(identity, finish); heldSockets.add(request.socket); emit({ event: 'holding' }); }
    else finish();
  });
  server.on('connection', (socket) => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)); });
  server.on('upgrade', (request, socket) => {
    const accept = createHash('sha1').update(request.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    emit({ event: 'upgrade', identity, url: request.url, prefix: request.headers['x-forwarded-prefix'] });
    socket.on('data', (data) => { if ((data[0] & 0x0f) === 8) socket.end(Buffer.from([0x88, 0])); });
  });
  process.on('message', async (message) => {
    if (message.command === 'start') {
      ({ canary, identity } = message);
      server.listen(message.port || 0, '127.0.0.1');
      await once(server, 'listening');
      emit({ event: 'ready', port: server.address().port });
    } else if (message.command === 'release') {
      held.get(identity)?.(); held.delete(identity);
    } else if (message.command === 'releaseListener') {
      server.close();
      emit({ event: 'listenerReleased' });
    } else if (message.command === 'dropHeld') {
      for (const socket of heldSockets) socket.destroy();
    } else if (message.command === 'stop') {
      for (const socket of sockets) socket.destroy();
      server.close(() => process.exit(0));
    }
  });
}

async function startBackend({ identity, canary, port }) {
  const child = fork(self, ['--backend'], { stdio: ['ignore', 'ignore', 'ignore', 'ipc'] });
  const events = [];
  child.on('message', (event) => events.push(event));
  const next = async (type, after = 0) => {
    for (let attempt = 0; attempt < 100; attempt++) {
      const event = events.slice(after).find((candidate) => candidate.event === type);
      if (event) return event;
      if (child.exitCode !== null) throw new Error('Backend exited before expected event');
      await sleep(20);
    }
    throw new Error(`Backend ${type} event timed out`);
  };
  child.send({ command: 'start', identity, canary, port });
  const ready = await next('ready');
  return { port: ready.port, events, next,
    release() { child.send({ command: 'release' }); },
    async releaseListener() { child.send({ command: 'releaseListener' }); await next('listenerReleased'); },
    dropHeld() { child.send({ command: 'dropHeld' }); },
    async close({ crash = false } = {}) {
      if (child.exitCode !== null || child.signalCode !== null) return;
      const exited = once(child, 'exit');
      if (crash) child.kill('SIGKILL'); else child.send({ command: 'stop' });
      await deadline(exited, 5000, 'backend shutdown');
    },
  };
}

async function proveAcceptedRetry(canary, { disableUpstreamKeepalive = false, stopCaddy = false } = {}) {
  let original;
  let trap;
  let caddy;
  try {
    original = await startBackend({ identity: 'original-runtime', canary });
    const route = { prefix: '/runtime-a', upstream: `127.0.0.1:${original.port}` };
    caddy = await startCaddy({ routes: [route], disableUpstreamKeepalive, forbiddenLogValues: [canary] });
    await (await fetch(`${caddy.baseUrl}/runtime-a/warm`)).arrayBuffer();
    const pending = fetch(`${caddy.baseUrl}/runtime-a/hold`, {
      headers: { authorization: `Bearer ${canary}` }, signal: AbortSignal.timeout(12000),
    }).then(async (response) => ({ status: response.status, body: await response.json().catch(() => null) }))
      .catch(() => ({ status: 0, body: null }));
    await original.next('holding');
    const warm = original.events.find((event) => event.url === '/warm');
    const held = original.events.find((event) => event.url === '/hold');
    if (!disableUpstreamKeepalive) assert.equal(warm.peerPort, held.peerPort, 'Must actually exercise a reused upstream connection');
    await caddy.updateRoutes([]);
    const caddyExit = stopCaddy ? await caddy.close() : undefined;
    await original.releaseListener();
    trap = await startBackend({ identity: 'wrong-runtime', canary, port: original.port });
    original.dropHeld();
    const response = await deadline(pending, 15000, 'accepted request resolution');
    await sleep(100);
    const result = { upstreamConnectionReused: warm.peerPort === held.peerPort,
      reloadAcknowledged: true, caddyExitedBeforeReuse: stopCaddy,
      clientStatus: response.status, wrongTargetReceivedCredential: trap.events.some((event) => event.sawCanary) };
    if (stopCaddy) {
      result.caddyExit = caddyExit;
      caddy = await startCaddy({ routes: [], forbiddenLogValues: [canary] });
      assert.equal((await fetch(`${caddy.baseUrl}/runtime-a/hold`, { headers: { authorization: `Bearer ${canary}` } })).status, 404);
      assert.equal(trap.events.some((event) => event.sawCanary), false);
      result.freshProcessHasNoOldRoute = true;
    }
    return result;
  } finally {
    await original?.close();
    await trap?.close();
    await caddy?.close();
  }
}

async function rawPartialRequest(port, body, suffix) {
  const socket = net.createConnection({ host: '127.0.0.1', port });
  await once(socket, 'connect');
  let raw = '';
  socket.on('data', (data) => { raw += data.toString(); });
  socket.on('error', () => {});
  const closed = once(socket, 'close');
  // The accepted TCP connection starts an HTTP message, but the headers are
  // deliberately incomplete until after /load acknowledges route withdrawal.
  socket.write(`POST /runtime-a/${suffix} HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\nConnection: close\r\nContent-Length: ${Buffer.byteLength(body)}\r\n`);
  await sleep(100);
  return {
    async finish() {
      if (!socket.destroyed) socket.end(`Authorization: Bearer ${body}\r\n\r\n${body}`);
      try { await deadline(closed, 5000, 'partial request completion'); } catch { socket.destroy(); }
      return { status: Number(raw.match(/^HTTP\/1\.[01] (\d+)/)?.[1] || 0), connectionClosed: socket.destroyed };
    },
    close() { socket.destroy(); },
  };
}

export async function runRoutingProof() {
  const canary = randomBytes(32).toString('hex');
  const backends = [];
  let caddy;
  let partial;
  let ws;
  const results = {};
  try {
    const { stdout } = await promisify(execFile)(process.env.CADDY_BIN, ['version']);
    results.caddyVersion = stdout.trim();
    assert.match(results.caddyVersion, /^v?2\.11\.4(?:\s|$)/, 'Proof must use reviewed Caddy 2.11.4');
    results.nodeVersion = process.version;
    let original = await startBackend({ identity: 'original-runtime', canary });
    backends.push(original);
    const upstream = `127.0.0.1:${original.port}`;
    const route = { prefix: '/runtime-a', upstream };
    caddy = await startCaddy({ routes: [route], forbiddenLogValues: [canary] });
    const forwardedResponse = await fetch(`${caddy.baseUrl}/runtime-a/api/arbitrary?unchanged=1`, {
      method: 'POST', headers: { authorization: `Bearer ${canary}`, 'x-forwarded-prefix': '/forged' }, body: canary,
    });
    assert.equal(forwardedResponse.status, 200, 'initial routing response status');
    const forwardedText = await forwardedResponse.text();
    assert(forwardedText.length, 'initial routing response must have a JSON body');
    const forwarded = JSON.parse(forwardedText);
    assert.deepEqual(forwarded, { identity: 'original-runtime', url: '/api/arbitrary?unchanged=1', prefix: '/runtime-a', sawCanary: true });
    results.prefixAndCredentialForwarding = 'passed; prefix stripped, query preserved, injected prefix overrides client header';

    ws = new WebSocket(`${caddy.baseUrl.replace('http:', 'ws:')}/runtime-a/api/ws`);
    await deadline(new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = () => reject(new Error('WebSocket open failed')); }));
    const upgrade = await original.next('upgrade');
    assert.equal(upgrade.url, '/api/ws');
    assert.equal(upgrade.prefix, '/runtime-a');
    const wsClosed = new Promise((resolve) => { ws.onclose = (event) => resolve({ code: event.code, wasClean: event.wasClean }); });
    const holdingResponse = fetch(`${caddy.baseUrl}/runtime-a/hold`, { signal: AbortSignal.timeout(10000) });
    await original.next('holding');
    await caddy.updateRoutes([]);
    const forbidden = await fetch(`${caddy.baseUrl}/runtime-a/auth/password-login`, { method: 'POST', body: canary });
    assert.equal(forbidden.status, 404);
    original.release();
    const heldHttpResponse = await holdingResponse;
    const heldHttpText = await heldHttpResponse.text();
    results.alreadyAcceptedHttpAtWithdrawal = { status: heldHttpResponse.status,
      completedFromOriginal: heldHttpText.length > 0 && JSON.parse(heldHttpText).identity === 'original-runtime' };
    results.defaultWebSocketReload = await deadline(wsClosed, 5000, 'WebSocket reload close');
    await original.close();
    let trap = await startBackend({ identity: 'wrong-runtime', canary, port: original.port });
    backends.push(trap);
    assert.equal((await fetch(`${caddy.baseUrl}/runtime-a/auth/password-login`, { method: 'POST', body: canary })).status, 404);
    await sleep(100);
    assert.equal(trap.events.filter((event) => event.sawCanary).length, 0);
    results.withdrawThenReuseForNewConnections = 'passed: wrong runtime received no canary';
    await trap.close();

    // Adversary 1: late headers on a connection accepted before withdrawal.
    original = await startBackend({ identity: 'original-runtime', canary, port: original.port });
    backends.push(original);
    await caddy.updateRoutes([route]);
    partial = await rawPartialRequest(caddy.edgePort, canary, 'late-headers');
    await caddy.updateRoutes([]);
    await original.close();
    trap = await startBackend({ identity: 'wrong-runtime', canary, port: original.port });
    backends.push(trap);
    const lateResult = await partial.finish();
    await sleep(100);
    results.oldConnectionAfterAcknowledgedWithdrawal = { ...lateResult, wrongTargetReceivedCredential: trap.events.some((event) => event.sawCanary) };
    await trap.close();

    // Adversary 2: unexpected backend exit; publication is still present.
    original = await startBackend({ identity: 'original-runtime', canary, port: original.port });
    backends.push(original);
    await caddy.updateRoutes([route]);
    await original.close({ crash: true });
    trap = await startBackend({ identity: 'wrong-runtime', canary, port: original.port });
    backends.push(trap);
    await fetch(`${caddy.baseUrl}/runtime-a/auth/password-login`, { method: 'POST', body: canary });
    await trap.next('request');
    assert.equal(trap.events.some((event) => event.sawCanary), true, 'Negative control must detect stale-route credential disclosure');
    results.unexpectedExitAndAddressReuse = { wrongTargetReceivedCredential: true };
    results.acceptedKeepaliveRequestAfterWithdrawal = await proveAcceptedRetry(canary);
    assert.equal(results.acceptedKeepaliveRequestAfterWithdrawal.wrongTargetReceivedCredential, true,
      'Adversarial negative control must reproduce retry disclosure, or the claimed failure is not proven');
    results.keepaliveOffDiagnostic = await proveAcceptedRetry(canary, { disableUpstreamKeepalive: true });
    results.fullCaddyExitBeforeReuseDiagnostic = await proveAcceptedRetry(canary, { stopCaddy: true });
    results.scope = 'Real Caddy and independent Node HTTP processes on macOS. No Kata allocator, guest lifecycle, native Hermes, or production TLS claim.';
    results.verdict = 'blocked: simple dynamic-port publication is unsafe without an independently proven address-reuse fence';
    return results;
  } finally {
    partial?.close();
    if (ws && ws.readyState !== WebSocket.CLOSED) ws.close();
    for (const backend of backends) await backend.close();
    await caddy?.close();
  }
}

if (process.argv[2] === '--backend') backendChild();
else if (process.argv[1] && path.resolve(process.argv[1]) === self) {
  runRoutingProof().then((result) => {
    console.log(JSON.stringify(result, null, 2));
    process.exitCode = result.verdict.startsWith('blocked:') ? 1 : 0;
  }).catch((error) => { console.error(`Routing proof error: ${error.stack}`); process.exitCode = 2; });
}
