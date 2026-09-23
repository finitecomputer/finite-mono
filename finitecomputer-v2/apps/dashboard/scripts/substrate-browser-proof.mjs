// Dashboard UI -> Core grants -> native Hermes HTTP/WebSocket. By default the
// shell uses account/project fixtures; DASHBOARD=1 exercises actual Next routes.
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import http from 'node:http';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { chromium } from 'playwright';

export async function proveBrowser({ baseUrl, grantEndpoint, ownerToken, dashboardBase }) {
  if (process.env.FC_TEST_SUBSTRATE_DASHBOARD && !dashboardBase) {
    const { withDashboard } = await import('./substrate-dashboard-proof.mjs');
    return withDashboard({ ownerToken }, base => proveBrowser({ baseUrl, grantEndpoint, ownerToken, dashboardBase: base }));
  }
  const reservation = http.createServer().listen(dashboardBase ? 0 : Number(process.env.FC_TEST_SUBSTRATE_BROWSER_PORT || 0), '127.0.0.1');
  await once(reservation, 'listening');
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  const base = dashboardBase || `http://127.0.0.1:${port}`;
  const server = dashboardBase ? null : spawn(process.execPath, ['--import', 'tsx', 'scripts/web-design-fixture.ts', 'serve'], {
    cwd: new URL('..', import.meta.url),
    env: { ...process.env, FC_WEB_DESIGN_PORT: String(port), FC_WEB_DESIGN_NATIVE_HERMES: '1' },
    stdio: 'ignore',
  });
  let browser;
  let page;
  const network = [];
  try {
    for (let i = 0; i < 600; i++) {
      assert.equal(server?.exitCode ?? null, null, 'Dashboard server exited');
      if (await fetch(`${base}/healthz`).then(r => r.ok).catch(() => false)) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    browser = await chromium.launch({ headless: true,
      ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
        ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH }
        : { channel: 'chrome' }),
    });
    page = await browser.newPage({ ignoreHTTPSErrors: true }); // Disposable local CA only.
    page.on('requestfailed', request => network.push({ path: new URL(request.url()).pathname, error: request.failure()?.errorText }));
    page.on('response', response => { if (response.status() >= 400) network.push({ path: new URL(response.url()).pathname, status: response.status() }); });
    page.on('websocket', socket => {
      socket.on('socketerror', () => network.push({ websocket: 'error' }));
      socket.on('framereceived', ({ payload }) => {
        try { const frame = JSON.parse(String(payload)); if (frame.error) network.push({ rpcError: frame.error }); } catch {}
      });
    });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    let legacyCalls = 0;
    await page.route('**/api/chat/**', route => { legacyCalls++; return route.abort(); });
    if (!dashboardBase) await page.route('**/api/agents/*/hermes-access', async route => {
      const response = await fetch(grantEndpoint, {
        method: 'POST', headers: { authorization: `Bearer ${ownerToken}` },
        redirect: 'error', signal: AbortSignal.timeout(15000),
      });
      assert.equal(response.status, 200, 'Real Core grant');
      const grant = await response.json();
      assert.equal(grant.baseUrl, baseUrl);
      await route.fulfill({ json: grant });
    });
    // The fixture shell has no Next route server; forward downloads to the
    // real authorized native endpoint, just as the product route does.
    if (!dashboardBase) await page.route('**/api/agents/*/hermes-file?*', async route => {
      const response = await fetch(grantEndpoint, { method: 'POST', headers: { authorization: `Bearer ${ownerToken}` }, redirect: 'error', signal: AbortSignal.timeout(15000) });
      assert.equal(response.status, 200);
      const grant = await response.json();
      const url = new URL('api/files/download', baseUrl);
      url.searchParams.set('path', new URL(route.request().url()).searchParams.get('path'));
      const file = await fetch(url, { headers: { authorization: `Bearer ${grant.accessToken}` }, redirect: 'error', signal: AbortSignal.timeout(15000) });
      assert.equal(file.status, 200);
      await route.fulfill({ body: Buffer.from(await file.arrayBuffer()), contentType: file.headers.get('content-type') || 'application/octet-stream' });
    });
    const runtimeId = dashboardBase ? new URL(grantEndpoint).pathname.split('/')[6] : 'runtime_web_design';
    assert.ok(runtimeId);
    await page.goto(`${base}/dashboard/machines/${runtimeId}/chat`);
    await page.getByRole('button', { name: 'New chat', exact: true }).click();
    const marker = `browser-proof-${randomBytes(8).toString('hex')}`;
    await page.locator('.finite-chat__composer textarea').fill(`Reply with exactly ${marker}, without using tools.`);
    await page.getByRole('button', { name: 'Send message', exact: true }).click();
    const reply = page.locator('.finite-chat__message--agent').getByText(marker, { exact: true });
    await reply.waitFor({ timeout: 90000 });
    await page.getByRole('button', { name: 'Stop response', exact: true }).waitFor({ state: 'hidden' });
    await page.locator('input[type="file"]').setInputFiles({ name: 'quadrants.png', mimeType: 'image/png', buffer: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAIAAAAlC+aJAAAAbUlEQVR4nO3PwQkAUQhDQftv2q3hH9wQGHhnzczOnHZ7/YfyCwDKyy8AKC+/AKC8/AKA8vILAMrLLwAoL78AoLz8AoDy8gsAyssvACjv/sHxh905DQAAAAAAAAAAAAAAAAAAAAAAAAAAAADguQ/7Be0ehXGAqQAAAABJRU5ErkJggg==', 'base64') });
    await page.locator('.finite-chat__composer textarea').fill('Name the four quadrant colors in this attached image, clockwise from top left. Reply only with the four color names separated by commas.');
    await page.getByRole('button', { name: 'Send message', exact: true }).click();
    const colors = page.locator('.finite-chat__message--agent').getByText(/^red, green, yellow, blue[.!]?$/i);
    await colors.waitFor({ timeout: 90000 });
    await page.getByRole('button', { name: 'Stop response', exact: true }).waitFor({ state: 'hidden' });
    const uploadedImage = page.getByRole('img', { name: 'quadrants.png', exact: true });
    await uploadedImage.waitFor();
    await uploadedImage.evaluate(img => { if (!img.complete) return new Promise((resolve, reject) => { img.onload = resolve; img.onerror = reject; }); });
    assert.ok(await uploadedImage.evaluate(img => img.naturalWidth > 0), 'Uploaded image did not render');
    const selected = page.locator('.finite-chat__thread-open[aria-current="page"]');
    await page.waitForFunction(() => {
      const title = document.querySelector('.finite-chat__thread-open[aria-current="page"]')?.textContent?.trim();
      return title && title !== 'New chat';
    });
    const title = (await selected.innerText()).trim();
    await page.reload();
    await page.getByRole('button', { name: title, exact: true }).click();
    await reply.waitFor({ timeout: 30000 });
    await colors.waitFor();
    await uploadedImage.waitFor();
    await page.waitForFunction(() => document.querySelector('img[alt="quadrants.png"]')?.naturalWidth > 0);
    assert.equal(legacyCalls, 0, 'Native UI must not invoke Finite chat');
    assert.deepEqual(errors, []);
    console.error('Real native browser turn and reload history passed');
  } catch (error) {
    const directory = await mkdtemp(join(tmpdir(), 'finite-substrate-browser-'));
    await writeFile(join(directory, 'diagnostic.json'), JSON.stringify({ network, body: await page?.locator('body').innerText().catch(() => '') }), { mode: 0o600 });
    await page?.screenshot({ path: join(directory, 'page.png'), fullPage: true }).catch(() => {});
    console.error(`Private browser diagnostics: ${directory}`);
    throw error;
  } finally {
    await browser?.close();
    if (server && server.exitCode === null) {
      const stopped = once(server, 'exit');
      server.kill('SIGTERM');
      await stopped;
    }
  }
}
