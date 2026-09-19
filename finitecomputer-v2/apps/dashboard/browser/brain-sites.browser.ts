/** Product UI and draft-chat proof against local services; no live Brain/Sites reads. */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { test } from "node:test";
import { chromium, type Browser } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

for (const preview of [false, true]) {
  test(`Brain and Sites ${preview ? "local samples stay on the fixture agent" : "show honest product states and open reviewable chat drafts"}`, { timeout: 180_000 }, async () => {
    const reservation = http.createServer().listen(0, "127.0.0.1");
    await once(reservation, "listening");
    const address = reservation.address();
    assert(address && typeof address !== "string");
    const base = `http://127.0.0.1:${address.port}`;
    await new Promise<void>(resolve => reservation.close(() => resolve()));
    const reset = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "reset"]);
    await once(reset, "exit");
    assert.equal(reset.exitCode, 0);
    const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
      env: { ...process.env, FC_WEB_DESIGN_PORT: String(address.port), FC_WEB_DESIGN_SECOND_AGENT: "1", FC_WEB_DESIGN_PREVIEWS: preview ? "1" : "0" },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let output = "";
    server.stdout.on("data", chunk => { output += chunk; });
    server.stderr.on("data", chunk => { output += chunk; });
    let browser: Browser | undefined;
    try {
      const deadline = Date.now() + 60_000;
      while (true) {
        assert.equal(server.exitCode, null, output);
        if (await fetch(`${base}/healthz`).then(r => r.ok).catch(() => false)) break;
        assert(Date.now() < deadline, output);
        await new Promise(resolve => setTimeout(resolve, 100));
      }
      browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
      const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce" });
      // These cases prove unavailable/preview states; the live contract has its
      // own test. Do not wait for an unimplemented fixture owner grant.
      await page.route("**/api/agents/*/hermes-access", route => route.fulfill({ status: 403, json: {} }));
      const errors: string[] = [];
      const actions: Record<string, unknown>[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("request", request => {
        if (request.url().includes("/hosted-device/actions") && request.method() === "POST") actions.push(request.postDataJSON());
      });
      const root = `${base}/dashboard/machines/runtime_web_design`;
      await page.goto(`${root}/brain?preview=1`);
      const nav = page.getByRole("navigation", { name: "Agent navigation", exact: true });
      assert.equal(await nav.getByRole("link", { name: "Brain", exact: true }).getAttribute("aria-current"), "page");
      assert.equal(await nav.getByRole("link", { name: "Sites", exact: true }).count(), 1);
      if (preview) {
        await page.getByRole("rowheader", { name: "Personal brain", exact: true }).waitFor();
        await page.getByText("Design preview · Sample Brain memberships", { exact: true }).waitFor();
        const folders = page.getByLabel("6 folders in Finite", { exact: true });
        await folders.focus();
        await page.keyboard.press("Enter");
        await page.getByText("Runbooks", { exact: true }).waitFor();
        await page.getByText("Preview states", { exact: true }).click();
        await page.getByLabel("Sample state").selectOption("unauthorized");
        await page.getByRole("alert").filter({ hasText: "no longer available" }).waitFor();
        assert.equal(await page.getByRole("table").count(), 0);
        await nav.getByRole("link", { name: "Sites", exact: true }).click();
        await page.getByRole("link", { name: /Azimuth/ }).waitFor();
        await page.getByText("Design preview · Sample Sites records", { exact: true }).waitFor();
        // Even with every preview switch enabled, another selected agent never
        // inherits Moss's invented memberships or site records.
        for (const surface of ["brain", "sites"]) {
          await page.goto(`${base}/dashboard/machines/runtime_web_design_second/${surface}?preview=1`);
          await page.getByRole("heading", { name: surface === "brain" ? "Brain overview isn’t available yet" : "Sites are unavailable right now", exact: true }).waitFor();
          assert.equal(await page.getByText(/Design preview · Sample/).count(), 0);
        }
      } else {
        const artifacts = process.env.FC_BROWSER_ARTIFACT_DIR;
        for (const surface of ["brain", "sites"]) {
          await page.goto(`${root}/${surface}?preview=1&state=populated`);
          await page.getByRole("heading", { name: surface === "brain" ? "Brain overview isn’t available yet" : "Sites are unavailable right now", exact: true }).waitFor();
          assert.equal(await page.getByText(/Design preview · Sample|Preview states|Personal brain|Azimuth/).count(), 0);
          assert.equal(await page.getByRole("table").count(), 0);
          if (artifacts) {
            await mkdir(artifacts, { recursive: true });
            await page.screenshot({ path: path.join(artifacts, `${surface}-desktop.png`), fullPage: true });
          }
          await page.setViewportSize({ width: 390, height: 844 });
          // Wait for the responsive sidebar to finish leaving the viewport.
          await page.waitForFunction(() => {
            const sidebar = document.querySelector(".finite-agent-shell__sidebar");
            return !sidebar || sidebar.getBoundingClientRect().right <= 0;
          });
          assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
          const heading = await page.getByRole("heading", { level: 1, exact: true, name: surface === "brain" ? "Brain" : "Sites" }).boundingBox();
          const navigation = await page.getByRole("button", { name: "Open agent navigation", exact: true }).boundingBox();
          assert(heading && navigation && heading.y >= navigation.y + navigation.height + 12, "page heading clears the mobile navigation button");
          if (artifacts) await page.screenshot({ path: path.join(artifacts, `${surface}-mobile.png`), fullPage: true });
          await page.setViewportSize({ width: 1440, height: 1000 });
          await page.getByRole("button", { name: "New chat in Home", exact: true }).waitFor();
          const count = actions.length;
          await page.getByRole("button", { name: surface === "brain" ? "Open chat" : "Create a site", exact: true }).click();
          const prompt = surface === "brain"
            ? "Help me with Brain. Show me which brains you can access, or help me create one."
            : "Help me build a new website. Start by asking me what kind of site I want to create.";
          await page.waitForFunction(expected => document.querySelector("textarea")?.value === expected, prompt);
          assert(page.url().startsWith(`${root}/chat`));
          const launched = actions.slice(count).filter(action => "StartTopicChatIntent" in action);
          assert.equal(launched.length, 1);
          assert.equal(actions.some(action => Object.keys(action).some(key => key.startsWith("Send"))), false, "opening a draft never sends a message");
        }
        // The page remains useful while Chat itself is unavailable, and a
        // missing binding cannot dispatch a guessed new-chat action.
        await page.route("**/api/chat/**", route => route.fulfill({ status: 503, json: { error: "Chat unavailable" } }));
        for (const surface of ["brain", "sites"]) {
          await page.goto(`${root}/${surface}`);
          const count = actions.length;
          await page.getByRole("button", { name: surface === "brain" ? "Open chat" : "Create a site", exact: true }).click();
          await page.getByText("Chat is still connecting. Please try again in a moment.", { exact: true }).waitFor();
          assert.equal(actions.length, count);
        }
        await page.unroute("**/api/chat/**");
        // A deep link is still governed by existing selected-agent access.
        await page.goto(`${base}/dashboard/machines/unknown-agent/brain`);
        await page.waitForURL(`${base}/dashboard`);
        const devPreview = await page.goto(`${base}/dev/sites?preview=1`);
        assert.equal(devPreview?.status(), 404);
      }
      assert.deepEqual(errors, []);
    } finally {
      await browser?.close();
      server.kill("SIGTERM");
      if (server.exitCode === null) await once(server, "exit");
    }
  });
}
