#!/usr/bin/env node
// Real browser enforcement of the Runner-rendered edge and native Hermes auth.
// Component proof only: no Core grant, agentd control loop, or dashboard fixture.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createHash, X509Certificate } from 'node:crypto';
import { once } from 'node:events';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { connect, getCACertificates, setDefaultCACertificates } from 'node:tls';
import { startHermes, HERMES_REVISION } from './hosted-hermes-auth.mjs';

const require = createRequire(new URL('../../finitecomputer-v2/apps/dashboard/package.json', import.meta.url));
const { chromium } = require('playwright');
for (const name of ['RUNNER_PROOF_BINARY', 'CADDY_BIN', 'HERMES_PROOF_BINARY', 'HERMES_PROOF_SOURCE', 'PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH']) {
  assert(process.env[name], `${name} must select the installed repository-pinned tools / local browser`);
}
const directory = await mkdtemp('/tmp/finite-caddy-browser-');
const originalCAs = getCACertificates('default');
let hermes, caddy, browser;
const servers = [];
async function pageServer() {
  const server = createServer((_request, response) => {
    response.setHeader('Content-Type', 'text/html');
    response.end('<!doctype html><title>Native Hermes CORS component proof</title>');
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  servers.push(server);
  return `http://127.0.0.1:${server.address().port}`;
}
try {
  const origin = await pageServer();
  const disallowedOrigin = await pageServer();
  const runtimeId = 'runtime_browser_proof';
  hermes = await startHermes({ binary: process.env.HERMES_PROOF_BINARY,
    source: process.env.HERMES_PROOF_SOURCE,
    publicUrl: `https://hermes-proof.invalid/runtimes/${runtimeId}/` });
  const skill = join(hermes.home, 'skills/finite-caddy-acceptance');
  await mkdir(skill, { recursive: true });
  await writeFile(join(skill, 'SKILL.md'), '---\nname: finite-caddy-acceptance\ndescription: Real agent-local Caddy acceptance fixture\n---\n# Acceptance skill\nRead-only transport proof.\n');
  // Reserve a fresh loopback port, then release it immediately before Caddy.
  const reservation = createServer();
  await new Promise((resolve) => reservation.listen(0, '127.0.0.1', resolve));
  const port = reservation.address().port;
  await new Promise((resolve) => reservation.close(resolve));
  const edgeOrigin = `https://localhost:${port}`;
  const baseUrl = `${edgeOrigin}/runtimes/${runtimeId}/`;
  const manifest = join(directory, 'manifest.json');
  const config = join(directory, 'caddy.json');
  await writeFile(manifest, JSON.stringify({ public_origin: edgeOrigin,
    listen: `127.0.0.1:${port}`, admin_socket: `${directory}/admin.sock`,
    allowed_origins: [origin], routes: [{ runtime_id: runtimeId, host_port: Number(new URL(hermes.baseUrl).port) }] }));
  await writeFile(config, execFileSync(process.env.RUNNER_PROOF_BINARY,
    ['render-hosted-hermes-caddy', '--manifest', manifest]));
  caddy = spawn(process.env.CADDY_BIN, ['run', '--config', config], { env: {
    PATH: process.env.PATH, HOME: directory, XDG_DATA_HOME: join(directory, 'data'), XDG_CONFIG_HOME: join(directory, 'config'),
  }, stdio: 'ignore' });
  let ca;
  for (let attempt = 0; attempt < 100; attempt++) {
    assert.equal(caddy.exitCode, null, 'Caddy exited before readiness');
    ca = await readFile(join(directory, 'data/caddy/pki/authorities/local/root.crt'), 'utf8').catch(() => null);
    if (ca) {
      setDefaultCACertificates([...originalCAs, ca]);
      if ((await fetch(`${baseUrl}api/status`).catch(() => null))?.ok) break;
    }
    await delay(100);
  }
  assert(ca, 'Local edge CA not ready');
  const login = await fetch(`${baseUrl}auth/password-login`, { method: 'POST',
    headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ provider: 'basic', ...hermes.credentials }) });
  assert.equal(login.status, 200);
  const accessCookie = login.headers.getSetCookie().find((value) => value.split('=', 1)[0].endsWith('hermes_session_at'));
  assert(accessCookie);
  const accessToken = accessCookie.split(';')[0].slice(accessCookie.indexOf('=') + 1);
  // Trust only this scratch CA's SPKI in a fresh browser profile, not the OS.
  const leaf = await new Promise((resolve, reject) => {
    const socket = connect({ host: '127.0.0.1', port, servername: 'localhost' }, () => {
      resolve(socket.getPeerCertificate().raw); socket.end();
    });
    socket.on('error', reject);
  });
  const spki = [ca, leaf].map((certificate) => createHash('sha256')
    .update(new X509Certificate(certificate).publicKey.export({ type: 'spki', format: 'der' })).digest('base64')).join(',');
  browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH,
    headless: true, args: [`--ignore-certificate-errors-spki-list=${spki}`] });
  const page = await browser.newPage();
  await page.goto(origin);
  const result = await page.evaluate(async ({ baseUrl, accessToken }) => {
    const read = (path, token) => fetch(new URL(path, baseUrl), { credentials: 'omit',
      headers: token ? { Authorization: `Bearer ${token}` } : {}, signal: AbortSignal.timeout(10000) });
    const status = await read('api/status');
    const anonymous = await read('api/auth/me');
    const invalid = await read('api/auth/me', 'not-a-native-session');
    const authorized = await read('api/auth/me', accessToken);
    const skillsAnonymous = await read('api/skills');
    const skills = await read('api/skills', accessToken);
    const entries = await skills.json();
    await Promise.all([status, anonymous, invalid, authorized, skillsAnonymous].map((response) => response.arrayBuffer()));
    return { status: status.status, anonymous: anonymous.status, invalid: invalid.status,
      authorized: authorized.status, skillsAnonymous: skillsAnonymous.status, skills: skills.status,
      fixtureSkillPresent: Array.isArray(entries) && entries.some((entry) => entry.name === 'finite-caddy-acceptance') };
  }, { baseUrl, accessToken });
  assert.deepEqual(result, { status: 200, anonymous: 401, invalid: 401, authorized: 200,
    skillsAnonymous: 401, skills: 200, fixtureSkillPresent: true });
  await page.goto(disallowedOrigin);
  const denied = await page.evaluate(async (baseUrl) => {
    try { await fetch(new URL('api/status', baseUrl), { signal: AbortSignal.timeout(10000) }); return false; }
    catch (error) { return error instanceof TypeError; }
  }, baseUrl);
  assert(denied, 'Browser must block the unlisted origin even for native public status');
  console.log(JSON.stringify({ status: 'PASS', hermesRevision: HERMES_REVISION,
    browser: browser.version(), checks: result, unlistedOriginBlockedByBrowser: denied,
    productionRenderer: true, limits: ['Component browser page, not dashboard/Core authorization',
      'Local browser Origin and loopback backend; production Origin/bind is covered by namespace proof',
      'No fleet, DNS, Kata, agentd pull or model-turn durability qualification'] }, null, 2));
} finally {
  if (browser) await browser.close();
  if (caddy && caddy.exitCode === null && caddy.signalCode === null) {
    const stopped = once(caddy, 'exit'); caddy.kill('SIGTERM');
    await Promise.race([stopped, delay(5000)]);
    if (caddy.exitCode === null && caddy.signalCode === null) { caddy.kill('SIGKILL'); await stopped; }
  }
  if (hermes) await hermes.close();
  for (const server of servers) await new Promise((resolve) => server.close(resolve));
  setDefaultCACertificates(originalCAs);
  await rm(directory, { recursive: true, force: true });
}
