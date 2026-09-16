#!/usr/bin/env node
// Real pinned Hermes + actual Runner-rendered Caddy JSON. Run the sibling .sh.
// Linux network namespace has only loopback: 0.0.0.0 never exposes a host port.
// This uses Node's explicit Origin header, not a browser or the dashboard UI.
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { once } from 'node:events';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { networkInterfaces } from 'node:os';
import { request as httpsRequest } from 'node:https';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { getCACertificates, setDefaultCACertificates } from 'node:tls';
import { startHermes, probeAuth, HERMES_REVISION } from './hosted-hermes-auth.mjs';

const [manifestPath, configPath] = process.argv.slice(2);
assert(manifestPath && configPath, 'Pass the manifest and its unmodified Runner-rendered JSON');
assert.equal(process.platform, 'linux');
assert.deepEqual(Object.keys(networkInterfaces()), ['lo'], 'Only loopback may exist in this network namespace');
const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
const rendered = JSON.parse(await readFile(configPath, 'utf8'));
assert.equal(rendered.apps.pki.certificate_authorities.local.install_trust, false,
  'The proof must never install its CA in a system trust store');
assert.equal(manifest.routes.length, 1);
const route = manifest.routes[0];
const prefix = `/runtimes/${route.runtime_id}`;
const baseUrl = `${manifest.public_origin}${prefix}`;
// A nonloopback public URL engages native auth even for the loopback negative
// control. Caddy uses localhost solely for its temporary local TLS certificate;
// localhost is also a native accepted loopback Host. No Origin is rewritten.
const nativePublicUrl = `https://hermes-proof.invalid${prefix}`;
const productionOrigin = 'https://finite.computer';
const originalCAs = getCACertificates('default');
const canary = randomBytes(32).toString('base64url');
let hermes;
let caddy;
const reports = [];

async function startRenderedCaddy() {
  const directory = await mkdtemp('/tmp/finite-native-caddy-');
  let logs = '';
  const child = spawn(process.env.CADDY_BIN, ['run', '--config', configPath], {
    env: { PATH: process.env.PATH, HOME: directory, XDG_DATA_HOME: join(directory, 'data'),
      XDG_CONFIG_HOME: join(directory, 'config') }, stdio: ['ignore', 'pipe', 'pipe'],
  });
  const exited = once(child, 'exit');
  child.stdout.on('data', (chunk) => { logs += chunk; });
  child.stderr.on('data', (chunk) => { logs += chunk; });
  const close = async () => {
    let forced = false;
    if (child.exitCode === null && child.signalCode === null) {
      child.kill('SIGTERM');
      await Promise.race([exited, delay(5000)]);
      if (child.exitCode === null && child.signalCode === null) { forced = true; child.kill('SIGKILL'); await exited; }
    }
    await rm(directory, { recursive: true, force: true });
    return { exitCode: child.exitCode, signal: child.signalCode, forced };
  };
  try {
    for (let attempt = 0; attempt < 100; attempt++) {
      if (child.exitCode !== null || child.signalCode !== null) throw new Error('Rendered Caddy exited before readiness');
      const ca = await readFile(join(directory, 'data/caddy/pki/authorities/local/root.crt'), 'utf8').catch(() => null);
      if (ca) {
        // Trust this temporary CA in this Node process only; never install it.
        setDefaultCACertificates([...originalCAs, ca]);
        const response = await fetch(`${manifest.public_origin}/unpublished`, { signal: AbortSignal.timeout(1000) }).catch(() => null);
        if (response?.status === 404) return { close, logs: () => logs };
      }
      await delay(100);
    }
    throw new Error('Rendered Caddy TLS readiness timed out (logs withheld)');
  } catch (error) { await close(); throw error; }
}

try {
  for (const bindHost of ['127.0.0.1', '0.0.0.0']) {
    hermes = await startHermes({ binary: process.env.HERMES_PROOF_BINARY, source: process.env.HERMES_PROOF_SOURCE,
      bindHost, port: route.host_port, publicUrl: nativePublicUrl });
    caddy = await startRenderedCaddy();
    assert.equal((await fetch(`${manifest.public_origin}${prefix}-neighbor/api/config`)).status, 404, 'Neighboring prefix must not reach Hermes');
    // Fetch controls Host itself; node:https permits an explicit HTTP authority
    // while keeping TLS SNI/certificate verification bound to localhost.
    const wrongHostStatus = await new Promise((resolve, reject) => {
      const request = httpsRequest(`${baseUrl}/api/config`, { servername: new URL(baseUrl).hostname,
        headers: { Host: 'wrong-host.invalid' } }, (response) => {
        response.resume(); response.once('end', () => resolve(response.statusCode));
      });
      request.once('error', reject); request.end();
    });
    assert.equal(wrongHostStatus, 404, 'Unpublished HTTP Host must not reach Hermes');
    const auth = await probeAuth({ baseUrl, credentials: hermes.credentials,
      expiry: bindHost === '0.0.0.0',
      ...(bindHost === '0.0.0.0' ? { wsOrigin: productionOrigin, restOrigin: productionOrigin, allowedRestOrigin: true }
        : { rejectedOrigin: productionOrigin }),
      requestHeaders: { 'X-Forwarded-Prefix': '/forged-client-prefix' },
      onProgress: (name) => console.log(`PASS/PROGRESS (${bindHost}): ${name}`),
    });
    assert.equal(auth.containsCredential(hermes.logs()), false, 'Native Hermes logs disclosed a full credential');
    assert.equal(auth.containsCredential(caddy.logs()), false, 'Caddy logs disclosed a full credential');
    reports.push({ bindHost, checks: auth.checks, loginCount: auth.loginCount,
      wrongHostRejected: true, neighboringPrefixRejected: true, clientSuppliedPrefixOverwritten: true });
    if (bindHost === '0.0.0.0') {
      await hermes.close(); hermes = undefined;
      const failed = await fetch(`${baseUrl}/api/ws?ticket=${canary}`, { headers: {
        Authorization: `Bearer ${canary}`, 'Sec-WebSocket-Protocol': `hermes-gateway-v1, hermes-gateway-ticket.${canary}`,
      } });
      assert.equal(failed.status, 502, 'Closed native upstream must cause a real proxy error');
      await failed.arrayBuffer();
      await delay(100);
      assert.equal(caddy.logs().includes(canary), false, 'Caddy proxy-error log disclosed the synthetic credential');
      assert.match(caddy.logs(), /"level":"error"/, 'Must actually exercise error logging');
      reports.at(-1).proxyErrorLogsExcludeCredentialCanary = true;
    }
    const stopped = await caddy.close(); caddy = undefined;
    assert.deepEqual(stopped, { exitCode: 0, signal: null, forced: false }, 'Caddy must exit before upstream address reuse');
    if (hermes) { await hermes.close(); hermes = undefined; }
  }
  console.log(JSON.stringify({ status: 'PASS', hermesRevision: HERMES_REVISION, productionOrigin, nativePublicUrl,
    nodeVersion: process.version, caddyVersion: execFileSync(process.env.CADDY_BIN, ['version'], { encoding: 'utf8' }).trim(),
    hermesPackaging: 'Pinned upstream minimal Python environment; no bundled frontend or optional integrations',
    productionRenderer: true, transport: 'real TLS/WSS with temporary process-only CA trust', reports,
    limits: ['Explicit Node Origin header; not actual browser or dashboard authentication',
      'Guest bind semantics in isolated loopback-only Linux namespace; not Kata/CNI host-port allocation',
      'CORS headers verified over HTTP; actual browser enforcement remains separate', 'No Core grant or credential lifecycle integration',
      'No production DNS/certificate, Desktop, or accepted model-turn durability claim'] }, null, 2));
} catch (error) {
  console.error(`Native rendered-Caddy proof failed: ${error.message}`);
  process.exitCode = 1;
} finally {
  if (caddy) await caddy.close();
  if (hermes) await hermes.close();
  setDefaultCACertificates(originalCAs);
}
