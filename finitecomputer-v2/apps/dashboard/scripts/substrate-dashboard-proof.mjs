// Actual Next routes with the existing dev account mode and real local services.
// This exercises account routing, not third-party WorkOS login.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { copyFile, mkdtemp, open, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { chromium } from 'playwright';
import { proveFreshBrainApproval } from './substrate-brain-proof.mjs';

export async function withDashboard({ ownerToken, brainProof = false, brainChat = false }, work) {
  const response = await fetch('http://127.0.0.1:18420/api/core/v1/me', { headers: { authorization: `Bearer ${ownerToken}` }, signal: AbortSignal.timeout(10000) });
  assert.equal(response.status, 200, 'Core account lookup failed');
  const account = await response.json();
  const tsconfig = new URL('../tsconfig.devfinity-substrate-proof.json', import.meta.url);
  await copyFile(new URL('../tsconfig.json', import.meta.url), tsconfig);
  const port = 18427;
  const base = `http://127.0.0.1:${port}`;
  const directory = await mkdtemp(join(tmpdir(), 'finite-substrate-dashboard-'));
  const log = await open(join(directory, 'next.log'), 'w', 0o600);
  const brainOrigin = 'http://127.0.0.1:18430';
  const brain = (brainProof || brainChat) ? spawn(process.env.FC_TEST_SUBSTRATE_BRAIN_BINARY, [], {
    env: { ...process.env, FINITE_BRAIN_ADDR: brainChat ? '0.0.0.0:18430' : '127.0.0.1:18430',
      FINITE_BRAIN_PUBLIC_BASE_URL: brainOrigin, FINITE_BRAIN_DB: join(directory, 'brain.sqlite3') },
    stdio: ['ignore', log.fd, log.fd],
  }) : null;
  const server = spawn(process.execPath, ['node_modules/next/dist/bin/next', 'dev', '--hostname', '127.0.0.1', '--port', String(port)], {
    cwd: new URL('..', import.meta.url),
    env: { ...process.env,
      ...(brain ? { FC_BRAIN_UPSTREAM_URL: 'http://127.0.0.1:18430', FC_BRAIN_PUBLIC_ORIGIN: brainOrigin } : {}),
      FC_CORE_BASE_URL: 'http://127.0.0.1:18420',
      FC_HOSTED_WEB_DEVICE_URL: 'http://127.0.0.1:18428',
      FINITECHAT_HOSTED_API_TOKEN: 'disposable-control-proof',
      FC_WORKOS_AUTH_ENABLED: '0', FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: '1',
      FC_DASHBOARD_DEV_WORKOS_USER_ID: account.workos_user_id,
      FC_DASHBOARD_DEV_EMAIL: account.email,
      FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: ownerToken,
      FC_DASHBOARD_RUNTIME_MODE: 'canary',
      WORKOS_COOKIE_PASSWORD: 'disposable-substrate-cookie-password-at-least-32-characters',
      FC_DASHBOARD_BASE_URL: base, FC_DASHBOARD_PUBLIC_URL: base,
      NEXT_PUBLIC_APP_URL: base, NEXT_DIST_DIR: '.next-substrate-proof',
      NEXT_TSCONFIG_PATH: 'tsconfig.devfinity-substrate-proof.json',
      STRIPE_SECRET_KEY: '', STRIPE_WEBHOOK_SECRET: '',
    },
    stdio: ['ignore', log.fd, log.fd],
  });
  try {
    let ready = false;
    const deadline = Date.now() + 120000;
    while (Date.now() < deadline) {
      assert.equal(server.exitCode, null, 'Actual dashboard exited');
      ready = await fetch(`${base}/healthz`, { signal: AbortSignal.timeout(2000) }).then(r => r.ok).catch(() => false);
      if (ready) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.ok(ready, 'Actual dashboard did not become ready');
    if (brain) {
      assert.equal(brain.exitCode, null, 'Disposable Brain service exited');
      if (brainProof) await proveFreshBrainApproval(base, brainOrigin, account.workos_user_id);
    }
    return await work(base, directory, brainChat ? { origin: brainOrigin, user: account.workos_user_id, projects: account.projects } : undefined);
  } catch (error) {
    console.error(`Private actual-dashboard diagnostics: ${directory}`);
    throw error;
  } finally {
    for (const child of [server, brain]) {
      if (child && child.exitCode === null && child.signalCode === null) {
        const exited = once(child, 'exit');
        child.kill('SIGTERM');
        const timer = setTimeout(() => child.kill('SIGKILL'), 5000);
        await exited;
        clearTimeout(timer);
      }
    }
    await log.close();
    await rm(tsconfig, { force: true });
  }
}

if (process.argv[2] === 'create') {
  let input = '';
  for await (const chunk of process.stdin) input += chunk;
  const options = JSON.parse(input);
  const result = await withDashboard({ ...options, brainProof: Boolean(process.env.FC_TEST_SUBSTRATE_BRAIN_BINARY) }, async (base, directory) => {
    const browser = await chromium.launch({ headless: true,
      ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
        ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH }
        : { channel: 'chrome' }),
    });
    const page = await browser.newPage();
    try {
      page.setDefaultTimeout(60000);
      await page.goto(`${base}/dashboard?new=1`, { timeout: 90000 });
      await page.getByLabel('Launch Code', { exact: true }).fill(options.launchCode);
      await page.getByRole('button', { name: 'Continue with code', exact: true }).click();
      await page.getByRole('button', { name: 'Continue', exact: true }).click();
      await page.getByLabel('Agent name', { exact: true }).fill(options.displayName);
      const submitted = page.waitForResponse(response => response.request().method() === 'POST' && new URL(response.url()).pathname === '/dashboard/agent-creation-requests');
      await page.getByRole('button', { name: 'Launch agent', exact: true }).click();
      const response = await submitted;
      assert.equal(response.status(), 303, 'Actual dashboard creation form failed');
      const redirect = new URL(response.headers().location, base);
      assert.ok(!redirect.searchParams.has('agentCreationError'), 'Actual dashboard rejected agent creation');
      const requestId = redirect.searchParams.get('creation');
      assert.ok(requestId, 'Dashboard did not return a creation request');
      return { requestId };
    } catch (error) {
      await writeFile(join(directory, 'page.txt'), await page.locator('body').innerText().catch(() => ''), { mode: 0o600 });
      await page.screenshot({ path: join(directory, 'page.png'), fullPage: true }).catch(() => {});
      throw error;
    } finally {
      await browser.close();
    }
  });
  console.log(JSON.stringify({ ...result, brainApprovalVerified: Boolean(process.env.FC_TEST_SUBSTRATE_BRAIN_BINARY) }));
}
