#!/usr/bin/env node
// Native draft/session lifetime only, not accepted model-turn durability.
// No prompt.submit, provider credentials, model calls, or fabricated DB state.
// CADDY_BIN=<pinned caddy>/bin/caddy nix develop -c node <this-file>
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
import { startHermes, probeAuth, HERMES_REVISION } from './hosted-hermes-auth.mjs';
import { startCaddy } from './hosted-hermes-routing.mjs';

async function connect(baseUrl, mintTicket) {
  const { ticket } = await mintTicket();
  const ws = new WebSocket(`${baseUrl.replace(/^http/, 'ws')}/runtime-a/api/ws`,
    ['hermes-gateway-v1', `hermes-gateway-ticket.${ticket}`]);
  let sequence = 0;
  const pending = new Map();
  const closed = new Promise((resolve) => ws.addEventListener('close', resolve, { once: true }));
  ws.addEventListener('message', ({ data }) => {
    let frame;
    try { frame = JSON.parse(data); } catch { return; }
    const waiting = pending.get(frame.id);
    if (waiting) {
      pending.delete(frame.id);
      clearTimeout(waiting.timer);
      waiting.resolve(frame);
    }
  });
  ws.addEventListener('close', () => {
    for (const waiting of pending.values()) {
      clearTimeout(waiting.timer);
      waiting.reject(new Error('Native WS closed with an RPC outstanding'));
    }
    pending.clear();
  });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { ws.close(); reject(new Error('Native WS open timed out')); }, 10000);
    ws.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
    ws.addEventListener('error', () => { clearTimeout(timer); reject(new Error('Native WS open rejected')); }, { once: true });
  });
  assert.equal(ws.protocol, 'hermes-gateway-v1');
  return {
    closed,
    close: async () => { if (ws.readyState === WebSocket.OPEN) ws.close(); await closed; },
    rpc(method, params = {}) {
      return new Promise((resolve, reject) => {
        const id = ++sequence;
        const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, 15000);
        pending.set(id, { resolve, reject, timer });
        ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }));
      });
    },
  };
}

function result(frame, operation) {
  assert.equal(frame.error, undefined, `${operation} failed`);
  assert.ok(frame.result, `${operation} did not return a result`);
  return frame.result;
}

let hermes;
let caddy;
const clients = [];
try {
  hermes = await startHermes({ binary: process.env.HERMES_PROOF_BINARY, orphanReapGraceSeconds: 0,
    publicUrl: 'http://hermes-proof.invalid/runtime-a' });
  // The trusted-side native session stays connected to the same Hermes
  // process while the public ingress restarts. Authentication itself is
  // covered by the separate auth proof, including its Caddy-prefix variant.
  const auth = await probeAuth({ baseUrl: hermes.baseUrl, credentials: hermes.credentials, expiry: false });
  const routes = [{ prefix: '/runtime-a', upstream: new URL(hermes.baseUrl).host }];
  caddy = await startCaddy({ routes });
  const oldBaseUrl = caddy.baseUrl;
  const edgePort = caddy.edgePort;
  const initial = await connect(caddy.baseUrl, auth.mintTicket);
  clients.push(initial);
  const retained = result(await initial.rpc('session.create', { source: 'desktop', close_on_disconnect: false }), 'create retained draft');
  const disposable = result(await initial.rpc('session.create', { source: 'desktop', close_on_disconnect: true }), 'create disposable draft');
  assert.ok(retained.stored_session_id && retained.session_id);
  assert.ok(disposable.stored_session_id && disposable.session_id);
  console.log('Native retained and disposable drafts created; stopping real Caddy.');
  const stopped = await caddy.close();
  caddy = undefined;
  await Promise.race([initial.closed, delay(10000).then(() => { throw new Error('Ingress stop did not disconnect native WS'); })]);
  // Cross the pin's default 20-second orphan-reap grace using real wall time.
  // This distinguishes grace=0 behavior from an immediate lucky reconnect.
  console.log('Waiting 22 seconds offline, beyond the native default orphan grace.');
  await delay(22000);
  caddy = await startCaddy({ routes, edgePort });
  assert.equal(caddy.baseUrl, oldBaseUrl, 'Ingress restart must preserve the public address');
  const resumed = await connect(caddy.baseUrl, auth.mintTicket);
  clients.push(resumed);
  const restored = result(await resumed.rpc('session.resume', { session_id: retained.stored_session_id }), 'resume retained draft');
  assert.equal(restored.session_id, retained.session_id, 'Reconnect must reattach the existing live session');
  assert.equal(restored.stored_session_id, retained.stored_session_id, 'Stored identity must remain stable');
  assert.equal((await resumed.rpc('session.resume', { session_id: disposable.stored_session_id })).error?.code, 4007,
    'close_on_disconnect draft must have been removed');
  const closed = result(await resumed.rpc('session.close', { session_id: restored.session_id }), 'explicit close');
  assert.equal(closed.closed, true);
  assert.ok((await resumed.rpc('session.resume', { session_id: retained.stored_session_id })).error,
    'Explicitly closed unpersisted draft must no longer resume');
  assert.equal(auth.containsCredential(hermes.logs()), false, 'Native logs must not disclose retained credentials');
  console.log(JSON.stringify({ hermesRevision: HERMES_REVISION, orphanReapGraceSeconds: 0,
    sameIngressAddress: true, offlineSeconds: 22, caddyStop: stopped,
    retainedDraftReattachedSameLiveSession: true, disposableDraftRemoved: true, explicitCloseRemovedRetainedDraft: true,
    modelTurnDurability: 'NOT TESTED: requires real inference',
    limits: ['No provider or accepted model turn', 'No long-term idle TTL, cap saturation, or heap/process reclamation measurement',
      'Grace zero retains abandoned drafts until explicit close or independent native idle/cap cleanup'] }, null, 2));
} catch (error) {
  console.error(`Native lifetime proof failed: ${error.message}`);
  process.exitCode = 1;
} finally {
  for (const client of clients) await client.close();
  if (caddy) await caddy.close();
  if (hermes) await hermes.close();
}
