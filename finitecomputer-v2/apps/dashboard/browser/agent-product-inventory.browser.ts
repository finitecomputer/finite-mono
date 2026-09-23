/** Browser contract proof; owner/native responses are synthetic, never production credentials. */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { test } from "node:test";
import { chromium, type Browser } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

test("Brain and Sites use agent-scoped reads, preserve stale results, clear lost access, and ignore late old-agent responses", { timeout: 180_000 }, async () => {
  const reservation = http.createServer().listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const address = reservation.address();
  assert(address && typeof address !== "string");
  const base = `http://127.0.0.1:${address.port}`;
  await new Promise<void>(resolve => reservation.close(() => resolve()));
  const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
    env: { ...process.env, FC_WEB_DESIGN_PORT: String(address.port), FC_WEB_DESIGN_SECOND_AGENT: "1", FC_WEB_DESIGN_PREVIEWS: "0" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  server.stdout.on("data", chunk => { output += chunk; });
  server.stderr.on("data", chunk => { output += chunk; });
  let browser: Browser | undefined;
  let releaseHeld: (() => void) | undefined;
  try {
    await waitFor(async () => {
      assert.equal(server.exitCode, null, output);
      return fetch(`${base}/healthz`).then(response => response.ok).catch(() => false);
    });
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce" });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    let ownerStatus = 200;
    let nativeStatus = 200;
    let empty = false;
    let malformed = false;
    let reads = 0;
    let holdNext = false;
    let held = false;
    const methods: string[] = [];
    await page.route("**/api/chat/**", route => route.fulfill({ status: 503, json: {} }));
    await page.route("**/api/agents/*/hermes-access", route => {
      methods.push(route.request().method());
      const runtime = route.request().url().split("/agents/")[1].split("/")[0];
      return route.fulfill({ status: ownerStatus, json: ownerStatus === 200
        ? { baseUrl: `https://products.fixture.test/${runtime}/`, accessToken: "synthetic-test", expiresAt: 100 }
        : { error: "never-display-private-diagnostics" } });
    });
    await page.route("https://products.fixture.test/**", async route => {
      const headers = { "access-control-allow-origin": base, "access-control-allow-headers": "authorization" };
      if (route.request().method() === "OPTIONS") { await route.fulfill({ status: 200, headers }); return; }
      reads++;
      assert.equal(route.request().method(), "GET");
      assert.equal(route.request().headers().authorization, "Bearer synthetic-test");
      assert.equal(route.request().headers().cookie, undefined);
      const url = route.request().url();
      const brain = url.endsWith("/api/plugins/finite-brain/overview");
      assert(brain || url.endsWith("/api/plugins/finite-sites/overview"));
      const second = url.includes("runtime_web_design_second");
      if (malformed) { await route.fulfill({ status: 200, body: "not JSON", headers }); return; }
      const payload = brain ? { version: 1, brains: empty ? [] : [
        { id: "org", name: second ? "Fern brain" : "Moss brain", kind: "organization", role: "guest", folders: [{ id: "shared", name: "Visible folder" }] },
        { id: "pending", name: "Pending invitation", kind: "organization", role: "invited", folders: null },
        { id: "unknown", name: "No folder details", kind: "personal", role: "personal_agent", folders: null },
      ] } : { version: 1, sourceOnlyProjects: 1, sites: empty ? [] : [
        { id: "project", name: second ? "Fern site" : "Moss site", url: "https://example.finite.site", visibility: "private", status: "published", published: true, canEdit: true, repositoryUrl: "https://finite.site/project.git" },
        { id: "draft", name: "Unpublished site", url: "https://draft.finite.site", visibility: "shared", status: "claimed_unpublished", published: false, canEdit: false, repositoryUrl: "https://finite.site/draft.git" },
      ] };
      if (holdNext) {
        holdNext = false; held = true;
        await new Promise<void>(resolve => { releaseHeld = resolve; });
      }
      await route.fulfill({ status: nativeStatus, json: nativeStatus === 200 ? payload : { error: "never-display-private-diagnostics" }, headers }).catch(() => {});
    });
    const root = `${base}/dashboard/machines/runtime_web_design`;
    for (const product of ["brain", "sites"]) {
      await page.goto(`${root}/${product}`);
      const marker = product === "brain" ? page.getByRole("rowheader", { name: "Moss brain", exact: true }) : page.getByRole("link", { name: /Moss site/ });
      await marker.waitFor();
      if (product === "brain") {
        await page.getByText("2 brains accessible to Moss. 1 pending.", { exact: true }).waitFor();
        const folders = page.getByLabel("1 folders in Moss brain", { exact: true });
        await folders.focus(); await page.keyboard.press("Enter");
        await page.getByText("Visible folder", { exact: true }).waitFor();
        await page.getByText("Folder details unavailable", { exact: true }).waitFor();
      } else {
        await page.getByText("1 repository has no website yet.", { exact: true }).waitFor();
        assert(await page.getByRole("button", { name: "Copy link to Unpublished site", exact: true }).isDisabled());
        assert(await page.getByRole("button", { name: "Edit Unpublished site in chat", exact: true }).isDisabled());
        await page.getByRole("button", { name: "Share Moss site", exact: true }).click();
        await page.getByRole("dialog").waitFor();
        await page.keyboard.press("Escape");
      }
      const artifacts = process.env.FC_BROWSER_ARTIFACT_DIR;
      if (artifacts) {
        await mkdir(artifacts, { recursive: true });
        await page.screenshot({ path: path.join(artifacts, `${product}-live-desktop.png`), fullPage: true });
      }
      await page.setViewportSize({ width: 390, height: 844 });
      await waitFor(async () => page.evaluate(() => {
        const sidebar = document.querySelector(".finite-agent-shell__sidebar");
        return (!sidebar || sidebar.getBoundingClientRect().right <= 0) && document.documentElement.scrollWidth <= innerWidth;
      }));
      if (artifacts) await page.screenshot({ path: path.join(artifacts, `${product}-live-mobile.png`), fullPage: true });
      await page.setViewportSize({ width: 1440, height: 1000 });
      const refresh = page.getByRole("button", { name: `Refresh ${product === "brain" ? "brains" : "sites"}`, exact: true });
      const before = reads;
      nativeStatus = 503;
      await refresh.click();
      await page.getByRole("alert").filter({ hasText: "last successful list" }).waitFor();
      assert.equal(await marker.count(), 1);
      assert.equal(reads, before + 1);
      ownerStatus = 403;
      await refresh.click();
      await page.getByRole("alert").filter({ hasText: "unavailable for this account" }).waitFor();
      assert.equal(await marker.count(), 0);
      assert.equal(await page.locator("main time").count(), 0);
      assert.equal(reads, before + 1);
      ownerStatus = 200; nativeStatus = 200;
      await refresh.click(); await marker.waitFor();
      malformed = true;
      await refresh.click();
      await page.getByRole("alert").filter({ hasText: "incompatible response" }).waitFor();
      assert.equal(await marker.count(), 0);
      assert.equal(await page.locator("main time").count(), 0);
      malformed = false;
      ownerStatus = 200; nativeStatus = 404;
      await refresh.click();
      await page.getByRole("alert").filter({ hasText: "does not support" }).waitFor();
      nativeStatus = 200; empty = true;
      await refresh.click();
      await page.getByRole("heading", { name: product === "brain" ? "Your brains will show up here" : "Your sites will show up here", exact: true }).waitFor();
      empty = false;
      holdNext = true; held = false;
      await refresh.click(); await waitFor(async () => held);
      // Client-side agent switch while the old agent's request is outstanding.
      await page.getByRole("button", { name: /^Moss,/ }).click();
      await page.getByRole("menuitem", { name: "Fern", exact: true }).click();
      await page.waitForURL(`${base}/dashboard/machines/runtime_web_design_second`);
      await page.getByRole("navigation", { name: "Agent navigation", exact: true }).getByRole("link", { name: product === "brain" ? "Brain" : "Sites", exact: true }).click();
      await page.getByText(product === "brain" ? "Fern brain" : "Fern site", { exact: true }).waitFor();
      releaseHeld?.();
      assert.equal(await page.getByText(product === "brain" ? "Moss brain" : "Moss site", { exact: true }).count(), 0);
    }
    assert(methods.every(method => method === "POST"), "reads never toggle access");
    assert.equal(await page.getByText("never-display-private-diagnostics", { exact: false }).count(), 0);
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
  throw new Error("Timed out waiting for product inventory");
}
