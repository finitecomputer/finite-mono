/** Existing roster flow, with synthetic plugin responses and no production access. */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { test } from "node:test";
import { chromium, type Browser } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

test("admin roster renders dated scoped labels in existing cells, reloads, and clears lost access", { timeout: 180_000 }, async () => {
  const reservation = http.createServer().listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const address = reservation.address();
  assert(address && typeof address !== "string");
  const base = `http://127.0.0.1:${address.port}`;
  await new Promise<void>(resolve => reservation.close(() => resolve()));
  const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
    env: { ...process.env, FC_WEB_DESIGN_PORT: String(address.port), FC_WEB_DESIGN_PREVIEWS: "0" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  server.stdout.on("data", chunk => { output += chunk; });
  server.stderr.on("data", chunk => { output += chunk; });
  let browser: Browser | undefined;
  try {
    const deadline = Date.now() + 60_000;
    while (!await fetch(`${base}/healthz`).then(r => r.ok).catch(() => false)) {
      assert.equal(server.exitCode, null, output);
      assert(Date.now() < deadline, output);
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    let allowed = true;
    await page.route("**/api/chat/**", route => route.fulfill({ status: 503, json: {} }));
    await page.route("**/api/agents/*/hermes-access", route => route.fulfill({ status: allowed ? 200 : 403, json: allowed
      ? { baseUrl: "https://roster.fixture.test/", accessToken: "synthetic-test", expiresAt: 100 }
      : { error: "private-denial" } }));
    const key = "npub1ccz8l9zpa47k6vz9gphftsrumpw80rjt3nhnefat4symjhrsnmjs38mnyd";
    const blank = { role: "member", kind: null, name: null, email: null, ownerEmail: null, folders: [] };
    const payload = { version: 1, brainId: "synthetic", identities: [
      { ...blank, npub: key, kind: "agent", name: "Example Agent", lifecycle: "retired", ownerEmail: "owner@example.test" },
      { ...blank, npub: key.replace("ccz8", "cdz8"), kind: "human", email: "human@example.test" },
      { ...blank, npub: key.replace("ccz8", "cez8"), operatorAnnotation: { kind: "historical_identity", label: "Previous example Agent", observedAt: "2026-10-01T12:00:00Z" } },
      { ...blank, npub: key.replace("ccz8", "cfz8"), operatorAnnotation: { kind: "invitation_key", label: "Example invitation", observedAt: "2026-10-02T12:00:00Z" } },
      { ...blank, npub: key.replace("ccz8", "cgz8") },
    ] };
    await page.route("https://roster.fixture.test/**", route => {
      const headers = { "access-control-allow-origin": base, "access-control-allow-headers": "authorization" };
      if (route.request().method() === "OPTIONS") return route.fulfill({ status: 200, headers });
      assert.equal(route.request().method(), "GET", "roster reads never change access or identities");
      assert.equal(route.request().headers().authorization, "Bearer synthetic-test");
      const url = new URL(route.request().url());
      assert(["/api/plugins/finite-brain/overview", "/api/plugins/finite-brain/identities/synthetic"].includes(url.pathname), url.pathname);
      return route.fulfill({ status: 200, headers, json: url.pathname.endsWith("/overview")
        ? { version: 1, brains: [{ id: "synthetic", name: "Synthetic Brain", kind: "organization", role: "admin", folders: [] }] }
        : payload });
    });
    for (let attempt = 0; attempt < 2; attempt++) {
      await page.goto(`${base}/dashboard/machines/runtime_web_design/brain`);
      await page.getByRole("button", { name: "Synthetic Brain", exact: true }).click();
      const roster = page.getByRole("region", { name: "Identities in Synthetic Brain", exact: true });
      await roster.getByText("Historical identity · Evidence 2026-10-01", { exact: true }).waitFor();
      await roster.getByText("Invitation key · Evidence 2026-10-02", { exact: true }).waitFor();
      await roster.getByText("Agent (retired)", { exact: true }).waitFor();
      assert.equal(await roster.getByRole("row").count(), 6);
      assert.equal(await roster.getByRole("columnheader").allTextContents().then(v => v.join(",")), "Identity,Owner,Role,Folders,Key");
      assert.equal(await roster.getByText("Unknown", { exact: true }).count(), 1);
      const ownerCells = await roster.locator("tbody tr td:nth-child(2)").allTextContents();
      assert.deepEqual(ownerCells, ["owner@example.test", "", "", "", ""]);
    }
    const artifacts = process.env.FC_BROWSER_ARTIFACT_DIR;
    if (artifacts) {
      await mkdir(artifacts, { recursive: true });
      await page.screenshot({ path: path.join(artifacts, "brain-scoped-roster-desktop.png"), fullPage: true });
    }
    await page.setViewportSize({ width: 390, height: 844 });
    await page.waitForFunction(() => {
      const sidebar = document.querySelector(".finite-agent-shell__sidebar");
      return (!sidebar || sidebar.getBoundingClientRect().right <= 0)
        && document.documentElement.scrollWidth <= innerWidth;
    });
    if (artifacts) await page.screenshot({ path: path.join(artifacts, "brain-scoped-roster-mobile.png"), fullPage: true });
    allowed = false;
    await page.getByRole("button", { name: "Refresh brains", exact: true }).click();
    await page.getByRole("alert").filter({ hasText: "unavailable for this account" }).waitFor();
    assert.equal(await page.getByText("Previous example Agent", { exact: true }).count(), 0);
    assert.equal(await page.getByText("private-denial", { exact: true }).count(), 0);
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    server.kill("SIGTERM");
    if (server.exitCode === null) await once(server, "exit");
  }
});
