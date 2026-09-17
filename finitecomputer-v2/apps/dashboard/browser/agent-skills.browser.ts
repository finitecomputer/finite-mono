/** Dashboard rendering against the local design fixture. Owner grants and native
 * responses are intercepted; this is UI/transport-contract proof, not live auth. */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import http from "node:http";
import { test } from "node:test";
import { chromium, type Browser, type Route } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

test("agent Skills loads independently of Chat, refreshes manually, and clears private results on access or agent changes", { timeout: 120_000 }, async () => {
  const reservation = http.createServer();
  reservation.listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const address = reservation.address();
  assert(address && typeof address !== "string");
  const port = address.port;
  await new Promise<void>(resolve => reservation.close(() => resolve()));
  const base = `http://127.0.0.1:${port}`;
  const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
    env: { ...process.env, FC_WEB_DESIGN_PORT: String(port), FC_WEB_DESIGN_SECOND_AGENT: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  server.stdout.on("data", chunk => { output += chunk; });
  server.stderr.on("data", chunk => { output += chunk; });
  let browser: Browser | undefined;
  let releaseHeld: (() => void) | undefined;
  try {
    await waitFor(async () => {
      assert(server.exitCode === null, output);
      try { return (await fetch(`${base}/healthz`)).ok; } catch { return false; }
    });
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    let ownerStatus = 200;
    let nativeStatus = 200;
    let legacyResponse = false;
    let nativeReads = 0;
    const ownerMethods: string[] = [];
    let data = [
      { name: "zeta", description: "Find research", category: "research", enabled: false },
      { name: "alpha", description: "Write code", category: "software-development", enabled: true },
      { name: "plain", description: "General help", category: null, enabled: true },
    ];
    let holdNext = false;
    let held = false;
    await page.route("**/api/chat/**", route => route.fulfill({ status: 503, json: { error: "Chat unavailable" } }));
    await page.route("**/api/agents/*/hermes-access", async route => {
      ownerMethods.push(route.request().method());
      const runtime = route.request().url().split("/agents/")[1].split("/")[0];
      await route.fulfill({ status: ownerStatus, json: ownerStatus === 200
        ? { baseUrl: `https://skills.fixture.test/${runtime}/`, accessToken: "synthetic-browser-test", expiresAt: 100 }
        : { error: "private upstream error" } });
    });
    const fulfill = (route: Route, status: number, json: unknown) => route.fulfill({ status, json,
      headers: { "access-control-allow-origin": base, "access-control-allow-headers": "authorization" } });
    await page.route("https://skills.fixture.test/**", async route => {
      if (route.request().method() === "OPTIONS") { await fulfill(route, 200, {}); return; }
      nativeReads++;
      assert(route.request().url().endsWith("/api/skills?inventory=true"));
      assert.equal(route.request().method(), "GET");
      assert.equal(route.request().headers().authorization, "Bearer synthetic-browser-test");
      assert.equal(route.request().headers().cookie, undefined);
      if (holdNext) {
        holdNext = false; held = true;
        await new Promise<void>(resolve => { releaseHeld = resolve; });
        await fulfill(route, 200, { inventory_version: 1, skills: [{ name: "old-agent-secret", description: "Late response", category: null, enabled: true }] }).catch(() => {});
        return;
      }
      const records = route.request().url().includes("runtime_web_design_second")
        ? [{ name: "fern-only", description: "Second agent", category: null, enabled: true }] : data;
      await fulfill(route, nativeStatus, nativeStatus === 200 ? (legacyResponse ? records : { inventory_version: 1, skills: records }) : { error: "private diagnostics" });
    });
    await page.goto(`${base}/dashboard/skills?machine=runtime_web_design`);
    await page.getByText("3 skills discovered for Moss.", { exact: true }).waitFor();
    await page.getByText("Disabled", { exact: true }).waitFor();
    assert.deepEqual(await page.locator("main h2").allTextContents(), ["General (1)", "Research (1)", "Software Development (1)"]);
    const artifacts = process.env.FC_BROWSER_ARTIFACT_DIR;
    if (artifacts) {
      await mkdir(artifacts, { recursive: true });
      await page.screenshot({ path: path.join(artifacts, "skills-desktop.png"), fullPage: true });
    }
    const initialReads = nativeReads;
    await page.getByRole("searchbox", { name: "Search skills" }).fill("Write code");
    assert.deepEqual(await page.locator("main article h3").allTextContents(), ["alpha"]);
    await page.getByRole("searchbox", { name: "Search skills" }).fill("no matches");
    await page.getByText("No skills match", { exact: false }).waitFor();
    await page.getByRole("button", { name: "Clear search", exact: true }).click();
    assert.equal(nativeReads, initialReads, "search does not refetch");
    const refresh = page.getByRole("button", { name: "Refresh", exact: true });
    nativeStatus = 503;
    await refresh.click();
    await page.getByRole("alert").filter({ hasText: "last successful list" }).waitFor();
    assert.equal(await page.locator("main article").count(), 3);
    assert.equal(nativeReads, initialReads + 1);
    ownerStatus = 403;
    await refresh.click();
    await page.getByText("Skills are unavailable for Moss.", { exact: true }).waitFor();
    assert.equal(await page.locator("main article").count(), 0);
    assert.equal(await page.locator("main time").count(), 0);
    assert.equal(nativeReads, initialReads + 1, "denied grants never contact native API");
    ownerStatus = 200; nativeStatus = 404;
    await refresh.click();
    await page.getByRole("alert").filter({ hasText: "does not support" }).waitFor();
    nativeStatus = 200; legacyResponse = true;
    await refresh.click();
    await page.getByRole("alert").filter({ hasText: "needs a Skills listing update" }).waitFor();
    assert.equal(await page.locator("main article").count(), 0);
    legacyResponse = false; data = [];
    await refresh.click();
    await page.getByText("No skills were discovered for Moss.", { exact: true }).waitFor();
    holdNext = true;
    await refresh.click();
    await waitFor(async () => held);
    await page.getByRole("button", { name: /^Moss,/ }).click();
    await page.getByRole("menuitem", { name: "Fern", exact: true }).click();
    await page.waitForURL(`${base}/dashboard/machines/runtime_web_design_second`);
    await page.getByRole("button", { name: /^Fern,/ }).waitFor();
    await page.getByRole("navigation", { name: "Agent navigation", exact: true }).getByRole("link", { name: "Skills", exact: true }).click();
    await page.getByText("1 skill discovered for Fern.", { exact: true }).waitFor();
    releaseHeld?.();
    await page.getByRole("heading", { name: "fern-only", exact: true }).waitFor();
    assert.equal(await page.getByText("old-agent-secret", { exact: true }).count(), 0);
    await page.setViewportSize({ width: 390, height: 844 });
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth));
    if (artifacts) await page.screenshot({ path: path.join(artifacts, "skills-mobile.png"), fullPage: true });
    assert(ownerMethods.every(method => method === "POST"), "the page never enables access");
    assert.deepEqual(errors, []);
  } finally {
    releaseHeld?.();
    await browser?.close();
    server.kill("SIGTERM");
    if (server.exitCode === null) await once(server, "exit");
  }
});

async function waitFor(check: () => Promise<boolean>) {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error("Timed out waiting for the design fixture");
}
