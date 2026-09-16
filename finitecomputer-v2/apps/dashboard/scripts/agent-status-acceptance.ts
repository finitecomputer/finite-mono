// Invoked by the ignored Rust gate, which owns real Core/Postgres/agentd/Hermes.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { createServer } from "node:net";
import { chromium } from "playwright";
import { chromiumLaunchOptions } from "./playwright-browser";

async function main() {
const reservation = createServer();
reservation.listen(0, "127.0.0.1"); await once(reservation, "listening");
const address = reservation.address();
assert(address && typeof address === "object");
const port = address.port;
await new Promise<void>((resolve) => reservation.close(() => resolve()));
const base = `http://127.0.0.1:${port}`;
const next = spawn(process.execPath, ["node_modules/next/dist/bin/next", "dev", "--hostname", "127.0.0.1", "--port", String(port)], {
  env: { ...process.env, NEXT_DIST_DIR: ".next-iroh-acceptance", FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
    FC_DASHBOARD_DEV_EMAIL: "runtime-auth@finite.test", FC_DASHBOARD_DEV_WORKOS_USER_ID: "runtime-auth-user",
    FC_WORKOS_AUTH_ENABLED: "0" }, stdio: ["ignore", "pipe", "pipe"], detached: true,
});
let logs = "";
next.stdout.on("data", (b) => { logs += String(b); });
next.stderr.on("data", (b) => { logs += String(b); });
const browser = await chromium.launch({ ...chromiumLaunchOptions(), headless: true });
try {
  for (let n = 0; ; n++) {
    if (next.exitCode !== null || n === 100) throw new Error(`Dashboard failed to start: ${logs}`);
    if (await fetch(`${base}/iroh/client.js`).then((r) => r.ok).catch(() => false)) break;
    await new Promise((r) => setTimeout(r, 500));
  }
  const page = await browser.newPage();
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const api = `/api/agents/${process.env.FINITE_TEST_PROJECT_ID}/transport`;
  await page.goto(`${base}/dashboard/machines/${process.env.FINITE_TEST_RUNTIME_ID}/status`);
  await page.getByRole("button", { name: "Enable hosted access" }).waitFor();
  // Actual dashboard route -> verified Core JWT -> default-off denial.
  const denied = await page.evaluate(async (api) => {
    const binding = await fetch(api).then((r) => r.json());
    return fetch(api, { method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ generation: binding.generation, endpointId: binding.endpointId, peerId: "a".repeat(64) })
    }).then((r) => r.status);
  }, api);
  assert.equal(denied, 404);
  await page.getByRole("button", { name: "Enable hosted access" }).click();
  await page.getByRole("button", { name: "Disable hosted access" }).waitFor();
  await page.getByRole("button", { name: "Check agent status" }).click();
  await page.getByText("Live Hermes status received", { exact: true }).waitFor({ timeout: 45_000 });
  const status = JSON.parse(await page.getByTestId("agent-status").innerText());
  assert.equal(typeof status, "object");
  assert.equal(typeof status.version, "string");
  assert.equal(typeof status.gateway_running, "boolean");
  // Repeat a full read: new WASM peer/admission, no persistent identity.
  await page.getByRole("button", { name: "Check agent status" }).click();
  await page.getByText("Live Hermes status received", { exact: true }).waitFor({ timeout: 45_000 });
  if (process.env.FINITE_TEST_STATUS_SCREENSHOT) {
    await page.screenshot({ path: process.env.FINITE_TEST_STATUS_SCREENSHOT, fullPage: true });
  }
  // Keep a separately admitted real browser peer to prove revocation at the
  // runtime, independently of the UI disabling its button.
  await page.evaluate(async (api) => {
    const url = "/iroh/client.js";
    const wasm = await import(url); await wasm.default();
    const binding = await fetch(api).then((r) => r.json());
    const peer = await wasm.BrowserPeer.create(binding.relayUrl);
    const response = await fetch(api, { method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ generation: binding.generation, endpointId: binding.endpointId, peerId: peer.id() }) });
    if (!response.ok) throw new Error("Held peer admission failed");
    Object.assign(window, { heldStatusPeer: peer, heldStatusEndpoint: binding.endpointId });
    await new Promise((r) => setTimeout(r, 6_000));
    const result = JSON.parse(await peer.request(binding.endpointId, "GET", "/api/status", "{}", new Uint8Array()));
    if (result.status !== 200) throw new Error("Held peer did not reach Hermes");
  }, api);
  await page.getByRole("button", { name: "Disable hosted access" }).click();
  await page.getByRole("button", { name: "Enable hosted access" }).waitFor();
  assert(await page.getByRole("button", { name: "Check agent status" }).isDisabled());
  assert.equal(await page.getByTestId("agent-status").count(), 0);
  const binding = await page.evaluate(async (api) => fetch(api).then((r) => r.json()), api);
  assert.equal(binding.enabled, false);
  await new Promise((r) => setTimeout(r, 6_000));
  const revoked = await page.evaluate(async () => {
    const state = window as unknown as { heldStatusPeer: {
      request(...args: unknown[]): Promise<string>; close(): Promise<void>; free(): void;
    }; heldStatusEndpoint: string };
    try {
      await state.heldStatusPeer.request(state.heldStatusEndpoint, "GET", "/api/status", "{}", new Uint8Array());
      return false;
    } catch { return true; }
    finally { await state.heldStatusPeer.close(); state.heldStatusPeer.free(); }
  });
  assert(revoked, "Disabled access still reached native Hermes");
  assert.deepEqual(errors, []);
  console.log("REAL DASHBOARD/WASM PASS: default-off denial, enable, two live Hermes status reads, disable, revoked peer denied");
} catch (error) {
  console.error(logs.slice(-8000)); throw error;
} finally {
  await browser.close();
  if (next.pid) process.kill(-next.pid, "SIGTERM");
  await once(next, "exit").catch(() => {});
}

}
main().catch((error) => { console.error(error); process.exitCode = 1; });
