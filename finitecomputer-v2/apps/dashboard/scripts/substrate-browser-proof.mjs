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

export async function proveBrowser({ baseUrl, grantEndpoint, ownerToken, dashboardBase, brainChat }) {
  if (process.env.FC_TEST_SUBSTRATE_DASHBOARD && !dashboardBase) {
    const { withDashboard } = await import('./substrate-dashboard-proof.mjs');
    return withDashboard({ ownerToken, brainChat: Boolean(process.env.FC_TEST_SUBSTRATE_BRAIN_BINARY) }, (base, _directory, brainChat) => proveBrowser({ baseUrl, grantEndpoint, ownerToken, dashboardBase: base, brainChat }));
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
    const filename = `${marker}.txt`;
    const generatedPath = `/data/agent/${filename}`;
    await page.locator('.finite-chat__composer textarea').fill(`Use the terminal tool to write exactly ${marker} followed by a newline into ${generatedPath}. Then reply with only MEDIA:${generatedPath} on its own line.`);
    await page.getByRole('button', { name: 'Send message', exact: true }).click();
    const generated = page.locator('.finite-chat__message--agent').getByRole('link', { name: filename, exact: true });
    await generated.waitFor({ timeout: 90000 });
    await page.getByRole('button', { name: 'Stop response', exact: true }).waitFor({ state: 'hidden' });
    const verifyGenerated = async () => {
      const href = await generated.getAttribute('href');
      assert.ok(href?.startsWith(`/api/agents/${runtimeId}/hermes-file?`));
      assert.equal(new URL(href, base).searchParams.get('path'), generatedPath);
      const file = await page.evaluate(async url => {
        const response = await fetch(url);
        return { status: response.status, text: await response.text(), disposition: response.headers.get('content-disposition') };
      }, href);
      assert.equal(file.status, 200, 'Generated file download');
      assert.equal(file.text, `${marker}\n`, 'Agent-created file bytes');
      if (dashboardBase) assert.ok(file.disposition?.startsWith('attachment;'), 'Actual Next route must download documents');
    };
    await verifyGenerated();
    // Automatic titles can finish asynchronously. Persist an explicit name
    // through the product UI before testing reload, rather than racing that job.
    const title = `Saved ${marker}`;
    await page.locator('.finite-chat__thread-row.is-active').hover();
    await page.locator('.finite-chat__thread-row.is-active button[title="Rename chat"]').click();
    await page.getByRole('dialog').getByLabel('Name', { exact: true }).fill(title);
    await page.getByRole('dialog').getByRole('button', { name: 'Save', exact: true }).click();
    await page.getByRole('dialog').waitFor({ state: 'hidden' });
    await page.getByRole('button', { name: title, exact: true }).waitFor();
    await page.reload();
    await page.getByRole('button', { name: title, exact: true }).click();
    await reply.waitFor({ timeout: 30000 });
    await colors.waitFor();
    await uploadedImage.waitFor();
    await page.waitForFunction(() => document.querySelector('img[alt="quadrants.png"]')?.naturalWidth > 0);
    await generated.waitFor();
    await verifyGenerated();
    if (brainChat) {
      const { brainProofClient } = await import('./substrate-brain-proof.mjs');
      const { provider, json, signed } = brainProofClient(brainChat.origin, brainChat.user);
      const human = await json(await provider('identifyMember'), 'native Brain human identity');
      const brainId = `native-${marker}`;
      const resultPath = `/data/agent/${brainId}.json`;
      // fbrain accepts loopback HTTP, but its public WebPKI roots exclude our
      // local CA. Relay only this disposable service; keep its signer untouched.
      const relay = `const http=require('node:http'),{spawn}=require('node:child_process'),fs=require('node:fs');
const server=http.createServer((req,res)=>{const up=http.request({hostname:'host.docker.internal',port:18430,path:req.url,method:req.method,headers:req.headers},r=>{res.writeHead(r.statusCode,r.headers);r.pipe(res)});up.on('error',()=>{res.writeHead(502);res.end()});req.pipe(up)});
server.listen(18430,'127.0.0.1',()=>{const fd=fs.openSync(${JSON.stringify(resultPath)},'w');const child=spawn('fbrain',${JSON.stringify(['brain', 'create', brainId, '--kind', 'organization', '--name', brainId, '--server', brainChat.origin, '--json'])},{stdio:['ignore',fd,'inherit']});fs.closeSync(fd);child.on('error',e=>{console.error(e);server.close();process.exitCode=1});child.on('exit',code=>{server.close();process.exitCode=code??1})});`;
      const command = `node -e '${relay.replaceAll("'", "'\\''")}'`;
      await page.locator('.finite-chat__composer textarea').fill(`Use the terminal tool to run exactly this command: ${command}. Do not set or override any identity, requester, session, or configuration environment variables. If it succeeds reply only with MEDIA:${resultPath} on its own line; otherwise report the error.`);
      await page.getByRole('button', { name: 'Send message', exact: true }).click();
      const result = page.locator('.finite-chat__message--agent').getByRole('link', { name: `${brainId}.json`, exact: true });
      await result.waitFor({ timeout: 90000 });
      await page.getByRole('button', { name: 'Stop response', exact: true }).waitFor({ state: 'hidden' });
      // Read independently as the human: the model's prose/file is not proof of server state.
      const metadata = await signed('GET', `/v1/brains/${brainId}/metadata`);
      assert.ok(metadata.admins.includes(human.npub), 'Native requester must receive Brain administration');
      const project = brainChat.projects.find(project => project.runtime?.id === runtimeId);
      assert.ok(project, 'Core must identify the exact runtime Project');
      const state = await json(await fetch('http://127.0.0.1:18428/v1/app/agent-bindings/open', {
        method: 'POST', headers: { authorization: 'Bearer disposable-control-proof',
          'x-finite-workos-user-id': brainChat.user, 'content-type': 'application/json' },
        body: JSON.stringify({ project_id: project.project.id }), signal: AbortSignal.timeout(10000),
      }), 'canonical hosted agent binding');
      assert.equal(state.hosted_agent_binding.project_id, project.project.id);
      const agent = state.hosted_agent_binding.agent_npub;
      assert.ok(agent, 'Binding must identify the exact runtime agent');
      assert.notEqual(agent, human.npub);
      assert.deepEqual([...metadata.admins].sort(), [human.npub, agent].sort(), 'Brain must retain the exact human and agent admins');
      assert.ok((await signed('GET', '/v1/brains')).brains.some(brain => brain.brainId === brainId));
      console.error('native browser -> terminal fbrain -> real Brain creation and human access passed');
    }
    assert.equal(legacyCalls, 0, 'Native UI must not invoke Finite chat');
    assert.deepEqual(errors, []);
    console.error('Real native browser turn, generated-file bytes, and reload history passed');
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
