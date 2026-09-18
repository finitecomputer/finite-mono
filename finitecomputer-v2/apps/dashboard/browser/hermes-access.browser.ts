/** Real dashboard UI with intercepted owner API responses; no live access changes. */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import http from "node:http";
import { test } from "node:test";
import { chromium, type Browser } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

test("owners explicitly change web access with fresh generations, independently of Chat", { timeout: 120_000 }, async () => {
  const reservation = http.createServer().listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const address = reservation.address();
  assert(address && typeof address !== "string");
  await new Promise<void>(resolve => reservation.close(() => resolve()));
  const base = `http://127.0.0.1:${address.port}`;
  const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
    env: { ...process.env, FC_WEB_DESIGN_PORT: String(address.port), FC_WEB_DESIGN_SECOND_AGENT: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  server.stdout.on("data", chunk => { output += chunk; });
  server.stderr.on("data", chunk => { output += chunk; });
  let browser: Browser | undefined;
  try {
    for (let attempt = 0; attempt < 600; attempt++) {
      assert.equal(server.exitCode, null, output);
      if (await fetch(`${base}/healthz`).then(r => r.ok).catch(() => false)) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    let access = { runtimeId: "runtime_web_design", enrolled: true, enabled: false,
      generation: 3, appliedGeneration: 3, applyStatus: "applied" };
    const writes: unknown[] = [];
    let writeStatus = 200;
    let readStatus = 200;
    await page.route("**/api/chat/**", route => route.fulfill({ status: 503, json: {} }));
    await page.route("**/api/agents/*/hermes-access", async route => {
      const method = route.request().method();
      if (method === "POST") return route.fulfill({ status: 409, json: {} });
      if (method === "PUT") {
        const body = route.request().postDataJSON();
        writes.push(body);
        if (writeStatus !== 200) return route.fulfill({ status: writeStatus, json: {} });
        assert.equal(body.expectedGeneration, access.generation);
        access = { ...access, enabled: body.enabled, generation: access.generation + 1, applyStatus: "pending" };
        return route.fulfill({ json: access });
      }
      return route.fulfill({ status: readStatus, json: access });
    });
    await page.goto(`${base}/dashboard/skills?machine=runtime_web_design`);
    await page.getByRole("link", { name: "Manage agent web access" }).waitFor();
    await page.getByRole("button", { name: "Refresh", exact: true }).click();
    assert.equal(writes.length, 0, "Skills loading and Refresh never enable access");
    await page.getByRole("link", { name: "Manage agent web access" }).click();
    const card = page.locator("#web-access");
    const enable = card.getByRole("button", { name: "Turn on web access", exact: true });
    await enable.waitFor();
    await card.getByText("Web access is off.", { exact: false }).waitFor();
    assert.equal(writes.length, 0, "Connections reads never enable access");
    await enable.click();
    await card.getByText("Turning on web access.", { exact: false }).waitFor();
    assert.equal(await card.getByText("Connected", { exact: true }).count(), 0);
    assert.deepEqual(writes, [{ enabled: true, expectedGeneration: 3 }]);
    access = { ...access, appliedGeneration: access.generation, applyStatus: "applied" };
    await card.getByRole("button", { name: "Refresh status" }).click();
    await card.getByText("Web access is on for your account.", { exact: true }).waitFor();
    await card.getByRole("button", { name: "Turn off web access" }).click();
    await card.getByText("Turning off web access.", { exact: false }).waitFor();
    assert.deepEqual(writes[1], { enabled: false, expectedGeneration: 4 });
    access = { ...access, appliedGeneration: access.generation, applyStatus: "applied" };
    await card.getByRole("button", { name: "Refresh status" }).click();
    await card.getByText("Web access is off.", { exact: false }).waitFor();

    // Another tab changed the generation: require a read, never replay the write.
    writeStatus = 409;
    access = { ...access, generation: 8, appliedGeneration: 8 };
    await enable.click();
    await card.getByRole("alert").waitFor();
    assert(await enable.isDisabled());
    assert.equal(writes.length, 3);
    writeStatus = 200;
    await card.getByRole("button", { name: "Refresh status" }).click();
    await card.getByText("Web access is off.", { exact: false }).waitFor();
    await enable.click();
    await card.getByText("Turning on web access.", { exact: false }).waitFor();
    assert.deepEqual(writes[3], { enabled: true, expectedGeneration: 8 });

    readStatus = 403;
    await card.getByRole("button", { name: "Refresh status" }).click();
    await card.getByRole("alert").waitFor();
    assert(await enable.isDisabled());
    assert.equal(await card.getByText("Connected", { exact: true }).count(), 0);
    readStatus = 200;
    access = { runtimeId: "runtime_web_design_second", enrolled: false, enabled: false,
      generation: 0, appliedGeneration: 0, applyStatus: "applied" };
    await page.goto(`${base}/dashboard/machines/runtime_web_design_second/connections`);
    await card.getByText("Web access is not available for this agent yet.").waitFor();
    assert(await enable.isDisabled());
    assert.equal(writes.length, 4);
    await page.setViewportSize({ width: 390, height: 844 });
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth));
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    server.kill("SIGTERM");
    if (server.exitCode === null) await once(server, "exit");
  }
});
