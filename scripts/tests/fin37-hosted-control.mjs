// Real local components; only the external WorkOS boundary uses devfinity's
// signed local identity fixture. No Core/agentd/Hermes handler is mocked.
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, rmSync, openSync, closeSync, renameSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { createServer } from 'node:net';
import { randomBytes } from 'node:crypto';
import assert from 'node:assert/strict';

const root = resolve(import.meta.dirname, '../..');
const state = mkdtempSync(join(tmpdir(), 'fin37-proof-'));
const children = [];
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(WORKOS_|FC_CORE_|FC_DASHBOARD_|FINITE_AGENTD_|FINITECHAT_|HERMES_)/.test(key)) delete env[key];
const hermes = process.env.FIN37_HERMES_ENV;
const caddy = process.env.FINITE_AGENTD_TEST_CADDY;
assert(hermes && caddy, 'Set FIN37_HERMES_ENV and FINITE_AGENTD_TEST_CADDY to pinned Nix artifacts');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const privateFile = (path, value) => writeFileSync(path, value, { mode: 0o600 });
async function port() { const s = createServer(); await new Promise(r => s.listen(0, '127.0.0.1', r)); const p = s.address().port; await new Promise(r => s.close(r)); return p; }
function start(name, command, args, extra = {}, cwd = root) {
  const fd = openSync(join(state, `${name}.log`), 'w', 0o600);
  const child = spawn(command, args, { cwd, env: { ...env, ...extra }, stdio: ['ignore', fd, fd], detached: true });
  closeSync(fd); children.push({ name, child }); return child;
}
async function ready(url, init = {}, timeout = 60000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const dead = children.find(({ child }) => child.exitCode !== null);
    assert(!dead, `${dead?.name} exited; inspect ${state}`);
    try { const response = await fetch(url, { ...init, signal: AbortSignal.timeout(1500) }); if (response.ok) return; } catch {}
    await sleep(100);
  }
  throw new Error(`Timed out waiting for ${url}; logs: ${state}`);
}
let pgStarted = false;
let passed = false;
try {
  const [pgPort, authPort, corePort, controlPort, tlsPort, hermesPort, healthPort, dashboardPort] = await Promise.all(Array.from({ length: 8 }, port));
  execFileSync('initdb', ['-D', join(state, 'postgres'), '-A', 'trust', '-U', 'proof', '--no-locale', '--encoding=UTF8'], { env, stdio: 'ignore' });
  execFileSync('pg_ctl', ['-D', join(state, 'postgres'), '-l', join(state, 'postgres.log'), '-o', `-h 127.0.0.1 -p ${pgPort} -k ${state}`, '-w', 'start'], { env, stdio: 'ignore' });
  pgStarted = true;
  const pgEnv = { ...env, PGHOST: '127.0.0.1', PGPORT: String(pgPort), PGUSER: 'proof', PGDATABASE: 'postgres' };
  const sql = statement => execFileSync('psql', ['-X', '-v', 'ON_ERROR_STOP=1', '-At'], { env: pgEnv, input: statement, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trim();
  start('identity-fixture', join(root, 'target/debug/devfinity'), ['workos-fixture', '--listen', `127.0.0.1:${authPort}`, '--state-dir', join(state, 'workos')]);
  await ready(`http://127.0.0.1:${authPort}/sso/jwks/client_devfinity`);
  const ownerJwt = readFileSync(join(state, 'workos/dashboard-customer.jwt'), 'utf8').trim();
  const bobJwt = readFileSync(join(state, 'workos/bob.jwt'), 'utf8').trim();
  const managementToken = randomBytes(32).toString('hex');
  const hermesToken = randomBytes(32).toString('hex');
  const agentHome = join(state, 'agent'), hermesHome = join(state, 'hermes');
  mkdirSync(agentHome); mkdirSync(hermesHome); mkdirSync(join(state, 'targets'));
  privateFile(join(state, 'token'), managementToken);
  privateFile(join(state, 'control.json'), JSON.stringify({ listen: `127.0.0.1:${controlPort}`, runtime_id: 'runtime_proof', token_file: join(state, 'token') }));
  const launcher = join(state, 'hermes-launch');
  writeFileSync(launcher, `#!/bin/sh\nexec '${hermes}/bin/hermes' serve --host 127.0.0.1 --port ${hermesPort} --isolated\n`, { mode: 0o700 });
  start('agentd', join(root, 'target/debug/finite-agentd'), ['serve'], {
    PATH: `${hermes}/bin:${env.PATH}`, FINITECHAT_HOME: agentHome, HERMES_HOME: hermesHome,
    FINITE_AGENTD_CONTROL_CONFIG: join(state, 'control.json'), FINITECHAT_BIN: '/usr/bin/false',
    FINITE_SERVER_URL: 'http://127.0.0.1:1', FINITECHAT_WORKSPACE: join(state, 'workspace'),
    FINITE_AGENTD_PREPARE_COMMAND: join(root, 'finitechat/containers/agent/run_hermes_gateway.sh'),
    FINITE_HERMES_CONFIG_RECONCILER: join(root, 'finitechat/containers/agent/reconcile_hermes_config.py'),
    FINITE_AGENTD_HERMES_COMMAND: launcher,
    FINITE_AGENTD_HEALTH_PYTHON: `${hermes}/bin/python3`,
    FINITE_AGENTD_HEALTH_SCRIPT: join(root, 'finitechat/containers/agent/health_server.py'),
    FINITE_AGENT_HTTP_HOST: '127.0.0.1', FINITE_AGENT_HTTP_PORT: String(healthPort),
    HERMES_DASHBOARD_SESSION_TOKEN: hermesToken,
  });
  await ready(`http://127.0.0.1:${controlPort}/v1/runtimes/runtime_proof/connections`, { headers: { authorization: `Bearer ${managementToken}` } });
  await ready(`http://127.0.0.1:${hermesPort}/api/sessions?limit=1`, { headers: { 'x-hermes-session-token': hermesToken } });
  const caddyRoot = join(state, 'caddy-storage');
  writeFileSync(join(state, 'Caddyfile'), `{\n admin off\n auto_https disable_redirects\n skip_install_trust\n storage file_system {\n root ${caddyRoot}\n }\n log default {\n output discard\n }\n}\nhttps://localhost:${tlsPort} {\n bind 127.0.0.1\n tls internal\n reverse_proxy 127.0.0.1:${controlPort}\n}\n`);
  start('caddy', caddy, ['run', '--config', join(state, 'Caddyfile'), '--adapter', 'caddyfile'], { XDG_DATA_HOME: join(state, 'caddy-data'), XDG_CONFIG_HOME: join(state, 'caddy-config') });
  const ca = join(caddyRoot, 'pki/authorities/local/root.crt');
  for (let i = 0; i < 100 && !existsSync(ca); i++) await sleep(100);
  assert(existsSync(ca));
  const targetPath = join(state, 'targets/runtime_proof.json');
  const target = { source_host_id: 'proof-host', source_machine_id: 'proof-machine', endpoint: `https://localhost:${tlsPort}/`, token: managementToken };
  privateFile(targetPath, JSON.stringify(target));
  start('core', join(root, 'target/debug/finite-saas-core'), ['serve'], {
    FC_CORE_DATABASE_URL: `postgres://proof@127.0.0.1:${pgPort}/postgres`, FC_CORE_BIND: `127.0.0.1:${corePort}`,
    FC_CORE_API_TOKEN: randomBytes(32).toString('hex'), FC_CORE_RUNNER_API_TOKEN: randomBytes(32).toString('hex'), FC_FINITE_PRIVATE_USAGE_API_TOKEN: randomBytes(32).toString('hex'),
    WORKOS_CLIENT_ID: 'client_devfinity', WORKOS_ISSUER: `http://127.0.0.1:${authPort}`,
    WORKOS_API_BASE_URL: `http://127.0.0.1:${authPort}`, WORKOS_API_KEY: readFileSync(join(state, 'workos/workos-fixture-api-key'), 'utf8').trim(),
    FC_WORKOS_OPERATOR_ORG_ID: 'org_devfinity_operator', FC_CORE_AGENT_CONTROL_DIRECTORY: join(state, 'targets'), FC_CORE_AGENT_CONTROL_CA_FILE: ca,
  });
  const coreUrl = `http://127.0.0.1:${corePort}`;
  await ready(`${coreUrl}/healthz`);
  // Synthetic initial ownership state in real migrated Postgres. Deliberately
  // create NO chat identity, membership, room, hosted device or command binding.
  sql(`INSERT INTO users VALUES ('proof-owner','devfinity@finite.computer','linked','user_devfinity',now(),now());
    INSERT INTO customer_orgs VALUES ('proof-org','proof-owner','Proof','grandfathered',now(),now());
    INSERT INTO projects (id,customer_org_id,owner_user_id,display_name,created_at,updated_at) VALUES ('proof-project','proof-org','proof-owner','Proof',now(),now());
    INSERT INTO agent_runtimes (id,project_id,source_host_id,source_machine_id,source_import_key,host_facts,created_at,updated_at) VALUES ('runtime_proof','proof-project','proof-host','proof-machine','proof', '{"display_name":"Proof","hostname":null,"runtime_host":"proof-host","runtime_status":"unknown","active_inference_profile":null,"hermes_available":true,"published_app_urls":[]}',now(),now());
    INSERT INTO project_runtime_links VALUES ('proof-link','proof-project','runtime_proof',true,now());`);
  const controlUrl = `${coreUrl}/api/core/v1/me/runtime-agent-control/proof-project`;
  const ownerHeaders = { authorization: `Bearer ${ownerJwt}`, 'content-type': 'application/json' };
  assert.equal((await fetch(controlUrl)).status, 401);
  assert.equal((await fetch(controlUrl, { headers: { authorization: `Bearer ${bobJwt}` } })).status, 404);
  await ready(controlUrl, { headers: ownerHeaders });
  start('dashboard', 'node', ['node_modules/next/dist/bin/next', 'dev', '--hostname', '127.0.0.1', '--port', String(dashboardPort)], {
    FC_CORE_BASE_URL: coreUrl, FC_CONNECTIONS_TRANSPORT: 'https', FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: '1',
    FC_DASHBOARD_DEV_EMAIL: 'devfinity@finite.computer', FC_DASHBOARD_DEV_WORKOS_USER_ID: 'user_devfinity', FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: ownerJwt,
    WORKOS_ENABLED: '0',
  }, join(root, 'finitecomputer-v2/apps/dashboard'));
  const dashboardUrl = `http://127.0.0.1:${dashboardPort}/api/connections/machines/proof-project`;
  await ready(dashboardUrl, {}, 120000);
  privateFile(join(hermesHome, 'google_token.json'), 'synthetic-credential');
  const disconnected = await fetch(dashboardUrl, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ action: 'google_disconnect' }) });
  assert.equal(disconnected.status, 200, await disconnected.text());
  assert(!existsSync(join(hermesHome, 'google_token.json')));
  const inference = await fetch(dashboardUrl, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ action: 'inference', profile: 'openrouter', apiKey: 'sk-or-v1-synthetic-local-proof-only', model: 'anthropic/claude-sonnet-4.6' }) });
  assert.equal(inference.status, 200, await inference.text());
  assert(readFileSync(join(hermesHome, '.env'), 'utf8').includes('sk-or-v1-synthetic-local-proof-only'));
  await ready(`http://127.0.0.1:${hermesPort}/api/sessions?limit=1`, { headers: { 'x-hermes-session-token': hermesToken } });
  assert(!existsSync(join(agentHome, 'config.json')));
  assert.equal(sql('SELECT count(*) FROM chat_identities'), '0');
  assert.equal(sql('SELECT count(*) FROM project_room_memberships'), '0');
  // Current authority and placement are queried again, not retained in a cache.
  sql("UPDATE users SET workos_user_id='revoked-owner' WHERE id='proof-owner'");
  assert.equal((await fetch(controlUrl, { headers: ownerHeaders })).status, 404);
  sql("UPDATE users SET workos_user_id='user_devfinity' WHERE id='proof-owner'");
  privateFile(targetPath, JSON.stringify({ ...target, source_host_id: 'old-host' }));
  assert.equal((await fetch(controlUrl, { headers: ownerHeaders })).status, 503);
  privateFile(targetPath, JSON.stringify(target));
  const rotated = randomBytes(32).toString('hex');
  privateFile(join(state, 'next-token'), rotated); renameSync(join(state, 'next-token'), join(state, 'token'));
  assert.equal((await fetch(controlUrl, { headers: ownerHeaders })).status, 503);
  privateFile(targetPath, JSON.stringify({ ...target, token: rotated }));
  assert.equal((await fetch(controlUrl, { headers: ownerHeaders })).status, 200);
  rmSync(targetPath);
  assert.equal((await fetch(dashboardUrl)).status, 503);
  assert(!existsSync(join(agentHome, 'config.json')), 'no fallback through chat');
  assert(readFileSync(join(state, 'agentd.log'), 'utf8').includes('chat preparation unavailable'));
  console.log('PASS: dashboard → real Core + Postgres ownership → verified HTTPS/Caddy → integrated agentd → pinned Hermes settings validation and restart; chat unavailable throughout.');
  console.log('PASS: no chat membership records, wrong owner rejected, live ownership revocation, stale host rejection, credential rotation/revocation, and no chat fallback.');
  passed = true;
} finally {
  for (const { child } of children.reverse()) {
    if (child.exitCode === null) { try { process.kill(-child.pid, 'SIGTERM'); } catch {} }
  }
  await sleep(2000);
  for (const { child } of children) if (child.exitCode === null) { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
  if (pgStarted) { try { execFileSync('pg_ctl', ['-D', join(state, 'postgres'), '-m', 'fast', '-w', 'stop'], { env, stdio: 'ignore' }); } catch {} }
  if (passed) rmSync(state, { recursive: true });
  else console.error(`Private local proof logs retained at ${state}`);
}
