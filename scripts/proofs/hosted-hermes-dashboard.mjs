#!/usr/bin/env node
// Invoked by Core's ignored hosted_browser_composition test. Secrets travel only
// through inherited pipes/environment. No product fixture routes or auth bypass
// are compiled into Core. Next uses its existing development-account adapter;
// Core still verifies real signed JWTs against the isolated WorkOS fixture.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createHash, randomBytes, X509Certificate } from 'node:crypto';
import { once } from 'node:events';
import { mkdtemp, mkdir, readFile, writeFile, rm, open, chmod, readdir } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { setTimeout as delay } from 'node:timers/promises';
import { connect } from 'node:tls';

const require = createRequire(new URL('../../finitecomputer-v2/apps/dashboard/package.json', import.meta.url));
const { chromium } = require('playwright');
const ts = require('typescript');
for (const name of ['RUNNER_PROOF_BINARY', 'AGENTD_PROOF_BINARY', 'CADDY_BIN', 'HERMES_PROOF_BINARY', 'HERMES_PROOF_SOURCE', 'PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH']) assert(process.env[name], `${name} required`);
const lines = createInterface({ input: process.stdin })[Symbol.asyncIterator]();
const initial = JSON.parse((await lines.next()).value);
const directory = await mkdtemp('/tmp/finite-dashboard-proof-');
const children = [], logs = [];
let browser, failed = false;
const dashboardDir = resolve('finitecomputer-v2/apps/dashboard');
const checks = {};
async function freePort() {
  const server = createServer(); server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const port = server.address().port; await new Promise(r => server.close(r)); return port;
}
async function waitFor(check, label, timeout = 45000) {
  const started = Date.now();
  while (Date.now() - started < timeout) { if (await check()) return; await delay(200); }
  throw new Error(`${label} did not become ready`);
}
async function start(name, program, args, env, cwd) {
  const log = await open(join(directory, `${name}.log`), 'w', 0o600); logs.push(log);
  const child = spawn(program, args, { env, cwd, stdio: ['ignore', log.fd, log.fd] });
  children.push(child); return child;
}
async function removeScratch(path) {
  await chmod(path, 0o700);
  for (const entry of await readdir(path, { withFileTypes: true })) {
    if (entry.isDirectory()) await removeScratch(join(path, entry.name));
  }
  await rm(path, { recursive: true, force: true });
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, 'exit'); child.kill('SIGTERM');
  await Promise.race([exited, delay(6000)]);
  if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await exited; }
}
try {
  const ownerPort = await freePort(), otherPort = await freePort(), signedOutPort = await freePort();
  const dashboardOrigin = `http://127.0.0.1:${ownerPort}`;
  const edgePort = await freePort(), bridgePort = await freePort();
  const origin = `https://hermes-proof.test:${edgePort}`;
  const baseUrl = `${origin}/runtimes/${initial.runtimeId}/`;
  const manifest = join(directory, 'manifest.json');
  await writeFile(manifest, JSON.stringify({ public_origin: origin, listen: `127.0.0.1:${edgePort}`,
    admin_socket: `${directory}/admin.sock`, allowed_origins: [dashboardOrigin],
    routes: [{ runtime_id: initial.runtimeId, host_port: 8642 }] }));
  const config = JSON.parse(execFileSync(process.env.RUNNER_PROOF_BINARY, ['render-hosted-hermes-caddy', '--manifest', manifest]));
  // Only test PKI differs: retain the rendered HTTP routing/CORS verbatim,
  // issue a temporary cert for a synthetic hostname, never install OS trust.
  config.apps.tls = { automation: { policies: [{ subjects: ['hermes-proof.test'], issuers: [{ module: 'internal' }] }] } };
  const configPath = join(directory, 'caddy.json'); await writeFile(configPath, JSON.stringify(config));
  await start('caddy', process.env.CADDY_BIN, ['run', '--config', configPath], {
    PATH: process.env.PATH, HOME: directory, XDG_DATA_HOME: join(directory, 'data'), XDG_CONFIG_HOME: join(directory, 'config'),
  });
  let ca;
  await waitFor(async () => {
    ca = await readFile(join(directory, 'data/caddy/pki/authorities/local/root.crt'), 'utf8').catch(() => null);
    return Boolean(ca);
  }, 'Caddy CA');
  const leaf = await new Promise((resolveLeaf, reject) => {
    const socket = connect({ host: '127.0.0.1', port: edgePort, servername: 'hermes-proof.test', ca }, () => {
      resolveLeaf(socket.getPeerCertificate().raw); socket.end();
    }); socket.on('error', reject);
  });
  process.stdout.write(`${JSON.stringify({ ca, port: edgePort })}\n`);
  const { coreUrl } = JSON.parse((await lines.next()).value);

  const home = join(directory, 'home'); await mkdir(home);
  await writeFile(join(home, 'config.json'), JSON.stringify({ account_id: 'proof-agent', device_id: 'proof-device' }));
  await writeFile(join(home, 'config.yaml'), 'plugins:\n  disabled: [dashboard_auth/nous]\n');
  const skillDir = join(home, 'skills/finite-shared-access-proof'); await mkdir(skillDir, { recursive: true });
  await writeFile(join(skillDir, 'SKILL.md'), '---\nname: finite-shared-access-proof\ndescription: Real protected agent-local skills proof\n---\n# Shared access proof\nNo inference is required.\n');
  const scripts = {
    sleeper: '#!/bin/sh\nexec sleep 300\n',
    prepare: '#!/bin/sh\nexit 0\n',
    bridge: '#!/bin/sh\nexit 1\n',
    hermes: '#!/bin/sh\nexec "$HERMES_PROOF_BINARY" "$@"\n',
  };
  for (const [name, source] of Object.entries(scripts)) await writeFile(join(directory, name), source, { mode: 0o700 });
  // Fail before starting if another service owns native Hermes's fixed port.
  const reservation = createServer(); reservation.listen(8642, '0.0.0.0'); await once(reservation, 'listening'); await new Promise(r => reservation.close(r));
  const agentEnv = {
    ...process.env, PATH: `${directory}:${process.env.PATH}`, FINITECHAT_HOME: home, HERMES_HOME: home,
    PYTHONPATH: process.env.HERMES_PROOF_SOURCE, HERMES_BUNDLED_PLUGINS: `${process.env.HERMES_PROOF_SOURCE}/plugins`,
    FINITE_CORE_URL: initial.runtimeUrl, FINITE_CORE_CREDENTIAL: initial.bootstrap,
    FINITECHAT_BIN: join(directory, 'bridge'), FINITE_AGENTD_PREPARE_COMMAND: join(directory, 'prepare'),
    FINITE_AGENTD_HERMES_COMMAND: join(directory, 'sleeper'), FINITE_AGENTD_HEALTH_PYTHON: join(directory, 'sleeper'),
    FINITE_AGENTD_SIMPLEX_SCRIPT: join(directory, 'sleeper'), FINITE_AGENTD_BRIDGE_ADDR: `127.0.0.1:${bridgePort}`,
    FINITE_AGENTD_BRIDGE_READY_TIMEOUT_SECS: '300',
  };
  await start('agentd', process.env.AGENTD_PROOF_BINARY, ['serve'], agentEnv);
  async function dashboard(name, port, token, id, email) {
    await start(name, process.execPath, ['node_modules/next/dist/bin/next', 'start', '--hostname', '127.0.0.1', '--port', String(port)], {
      ...process.env, FC_CORE_BASE_URL: coreUrl, FC_CORE_API_TOKEN: '',
      FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: token ? '1' : '0', FC_DASHBOARD_DEV_EMAIL: email,
      FC_DASHBOARD_DEV_WORKOS_USER_ID: id, FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: token,
      FC_WORKOS_AUTH_ENABLED: '0', FC_WORKOS_OPERATOR_ORG_ID: '', FC_DASHBOARD_DEV_ADMIN_EMAILS: '',
      WORKOS_COOKIE_PASSWORD: randomBytes(32).toString('hex'), NEXT_PUBLIC_WORKOS_REDIRECT_URI: `http://127.0.0.1:${port}/callback`,
    }, dashboardDir);
    await waitFor(async () => (await fetch(`http://127.0.0.1:${port}/favicon.svg`).catch(() => null))?.ok, name);
  }
  await dashboard('dashboard-owner', ownerPort, initial.owner, 'runtime-auth-user', 'runtime-auth@finite.test');
  await dashboard('dashboard-other', otherPort, initial.other, 'other-user', 'other@finite.test');
  await dashboard('dashboard-signed-out', signedOutPort, '', '', '');
  const spki = [ca, leaf].map(cert => createHash('sha256').update(new X509Certificate(cert).publicKey.export({ type: 'spki', format: 'der' })).digest('base64')).join(',');
  browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH, headless: true,
    args: [`--ignore-certificate-errors-spki-list=${spki}`, '--host-resolver-rules=MAP hermes-proof.test 127.0.0.1'] });
  const page = await browser.newPage();
  // Use a real dashboard document and execute the exact shipped helper module.
  await page.goto(`${dashboardOrigin}/`, { waitUntil: 'domcontentloaded' });
  assert((await page.locator('body').innerText()).trim());
  assert.equal(await page.locator('[data-nextjs-dialog]').count(), 0);
  const source = await readFile(join(dashboardDir, 'src/lib/hosted-hermes-status.ts'), 'utf8');
  const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
  await page.addScriptTag({ type: 'module', content: `${js}\nglobalThis.proof={readHostedHermesAccess,changeHostedHermesAccess,readHostedHermesJson};` });
  await page.waitForFunction(() => Boolean(globalThis.proof));
  await page.evaluate(({ runtimeId, baseUrl }) => { globalThis.runtimeId = runtimeId; globalThis.baseUrl = baseUrl; }, { runtimeId: initial.runtimeId, baseUrl });
  const route = `/api/agents/${initial.runtimeId}/hermes-access`;
  const readAccess = () => page.evaluate(() => globalThis.proof.readHostedHermesAccess(globalThis.runtimeId, new AbortController().signal));
  const change = enabled => page.evaluate(async enabled => {
    const p = globalThis.proof, signal = new AbortController().signal;
    return p.changeHostedHermesAccess(await p.readHostedHermesAccess(globalThis.runtimeId, signal), enabled, signal);
  }, enabled);
  const applied = enabled => waitFor(async () => {
    const access = await readAccess(); return access.enabled === enabled && access.applyStatus === 'applied' && access.appliedGeneration === access.generation;
  }, `applied ${enabled}`);
  const initialAccess = await readAccess(); assert(initialAccess.enrolled && !initialAccess.enabled);
  await change(true); await applied(true);
  checks.enableApplied = true;
  const skills = await page.evaluate(() => globalThis.proof.readHostedHermesJson(globalThis.runtimeId, 'api/skills', new AbortController().signal));
  assert(Array.isArray(skills) && skills.some(skill => skill.name === 'finite-shared-access-proof'));
  checks.realSkillListed = true;
  const nativeDenial = await page.evaluate(async () => {
    const read = token => fetch(`${globalThis.baseUrl}api/skills`, { headers: token ? { authorization: `Bearer ${token}` } : {}, credentials: 'omit' });
    return [(await read()).status, (await read('invalid-native-session')).status];
  }); assert.deepEqual(nativeDenial, [401, 401]); checks.nativeAnonymousAndInvalidDenied = true;
  assert.equal(await page.evaluate(async () => (await fetch('/api/agents/runtime_wrong/hermes-access', { method: 'POST' })).status), 404);
  checks.wrongAgentDenied = true;
  for (const [name, port] of [['otherOwnerDenied', otherPort], ['signedOutDenied', signedOutPort]]) {
    const deniedPage = await browser.newPage(); await deniedPage.goto(`http://127.0.0.1:${port}/`);
    const status = await deniedPage.evaluate(async route => (await fetch(route, { method: 'POST' })).status, route);
    assert([401, 403, 404].includes(status), `${name}: ${status}`); checks[name] = true;
    const corsDenied = await deniedPage.evaluate(async url => { try { await fetch(`${url}api/status`); return false; } catch { return true; } }, baseUrl);
    assert(corsDenied); checks.unlistedOriginBlocked = true; await deniedPage.close();
  }
  // Retain a grant only in test-page memory to prove actual expiry; the shipped
  // helper itself retains no token across operations.
  await page.evaluate(async route => { globalThis.oldGrant = await (await fetch(route, { method: 'POST' })).json(); }, route);
  await delay(61000);
  assert.equal(await page.evaluate(async () => (await fetch(`${globalThis.baseUrl}api/skills`, { credentials: 'omit', headers: { authorization: `Bearer ${globalThis.oldGrant.accessToken}` } })).status), 401);
  const renewed = await page.evaluate(() => globalThis.proof.readHostedHermesJson(globalThis.runtimeId, 'api/skills', new AbortController().signal));
  assert(renewed.some(skill => skill.name === 'finite-shared-access-proof')); checks.expiryAndAutomaticRenewal = true;
  await page.evaluate(async route => { globalThis.beforeDisable = await (await fetch(route, { method: 'POST' })).json(); }, route);
  await change(false);
  assert.equal(await page.evaluate(async route => (await fetch(route, { method: 'POST' })).status, route), 409);
  await applied(false); checks.disableAppliedAndGrantDenied = true;
  await change(true); await applied(true);
  assert.equal(await page.evaluate(async () => (await fetch(`${globalThis.baseUrl}api/skills`, { credentials: 'omit', headers: { authorization: `Bearer ${globalThis.beforeDisable.accessToken}` } })).status), 401);
  const reenabled = await page.evaluate(() => globalThis.proof.readHostedHermesJson(globalThis.runtimeId, 'api/skills', new AbortController().signal));
  assert(reenabled.some(skill => skill.name === 'finite-shared-access-proof')); checks.reenableRotatesAndPreservesSkill = true;
  await change(false); await applied(false);
  process.stdout.write(`${JSON.stringify({ status: 'PASS', enrollment: initial.enrollment, checks, browser: browser.version(), limits: ['Synthetic WorkOS account adapter; no live OAuth flow', 'Gateway/Chat children are lifecycle fixtures', 'Local test CA/DNS; rendered HTTP routes unchanged', 'Enrollment uses Core store; no Runner upgrade delivery, fleet, Kata, or model-turn qualification'] })}\n`);
} catch (error) {
  // Print no native response, headers, tokens or credential-bearing process logs.
  failed = true;
  console.error(`Hosted browser proof failed: ${error.message}; private diagnostics: ${directory}`);
  throw error;
} finally {
  if (browser) await browser.close();
  for (const child of children.reverse()) await stop(child);
  for (const log of logs) await log.close();
  if (!failed) await removeScratch(directory);
}
