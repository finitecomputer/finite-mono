import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir } from "node:fs/promises";
import http from "node:http";
import { test } from "node:test";
import { chromium, type Page } from "playwright";

import { chromiumLaunchOptions } from "../scripts/playwright-browser";
import { stripeDashboardOnboardingReturnPath } from "../src/lib/stripe-billing";

const PAUSED = "Your trial has ended. Agent access is paused";
const WAITING = "Waiting for your agent to be ready. This page updates automatically.";

test("trial access changes refresh the real dashboard", { timeout: 180_000 }, async (t) => {
  const state = {
    blocked: true,
    recovery: null as "restart_pending" | null,
    online: false,
    writes: 0,
    summaryBlocked: null as boolean | null,
  };
  const billing = () => ({
    customer_org: { id: "org_trial", owner_user_id: "user_trial", name: "Trial fixture", billing_class: "standard" },
    billing_account: null,
    agent_creation_entitlement: null,
    can_create_agent: false,
    requires_billing: state.blocked,
    trial_access: { blocked: state.blocked, eventName: "Browser trial", subscriptionStatus: state.blocked ? "past_due" : "active", periodEnd: null },
  });
  const me = () => ({
    email: "trial@example.test", workos_user_id: "user_trial", claimable_candidates: [], agent_creation_requests: [],
    projects: [{
      project: { id: "project_trial", display_name: "Trial Agent", created_at: "2026-07-01T00:00:00Z", updated_at: "2026-07-01T00:00:00Z" },
      runtime_recovery: state.recovery,
      runtime: { id: "runtime_trial", project_id: "project_trial", contact_endpoint: "http://127.0.0.1:1", runtime_status: state.online ? "online" : "offline", hermes_available: state.online, created_at: "2026-07-01T00:00:00Z", updated_at: "2026-07-01T00:00:00Z" },
    }],
  });
  const core = http.createServer((request, response) => {
    if (request.method !== "GET") state.writes++;
    const data = request.url === "/api/core/v1/me/billing" ? billing()
      : request.url === "/api/core/v1/me" ? me()
      : request.url === "/api/core/v1/me/dashboard-summary" ? { me: me(), billing: { ...billing(), trial_access: { ...billing().trial_access, blocked: state.summaryBlocked ?? state.blocked } }, finite_private_usage: null }
      : null;
    response.writeHead(data ? 200 : 404, { "content-type": "application/json" });
    response.end(JSON.stringify(data ?? { error: "Unknown fixture route" }));
  });
  core.listen(0, "127.0.0.1");
  await once(core, "listening");
  const reserve = http.createServer();
  reserve.listen(0, "127.0.0.1");
  await once(reserve, "listening");
  const port = (reserve.address() as { port: number }).port;
  await new Promise<void>((resolve) => reserve.close(() => resolve()));
  const origin = `http://127.0.0.1:${port}`;
  const dashboard = spawn(process.execPath, ["node_modules/next/dist/bin/next", "dev", "--hostname", "127.0.0.1", "--port", String(port)], {
    env: {
      NODE_ENV: "development", PATH: process.env.PATH, HOME: process.env.HOME, TMPDIR: process.env.TMPDIR,
      FC_CORE_BASE_URL: `http://127.0.0.1:${(core.address() as { port: number }).port}`,
      FC_CORE_API_TOKEN: "trial-fixture-token",
      FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
      FC_DASHBOARD_DEV_EMAIL: "trial@example.test",
      FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_trial",
      FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "trial-fixture-access-token",
      FC_DASHBOARD_RUNTIME_MODE: "customer", FC_WORKOS_AUTH_ENABLED: "0",
      WORKOS_COOKIE_PASSWORD: "trial-fixture-cookie-password-at-least-32-characters",
      NEXT_DIST_DIR: ".next-browser-test",
    },
    stdio: "pipe",
  });
  let output = "";
  dashboard.stdout.on("data", (chunk) => { output += String(chunk); });
  dashboard.stderr.on("data", (chunk) => { output += String(chunk); });
  const browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
  const screenshots = process.env.TRIAL_ACCESS_SCREENSHOTS;
  if (screenshots) await mkdir(screenshots, { recursive: true });
  async function screenshot(page: Page, name: string) {
    if (screenshots) await page.screenshot({ path: `${screenshots}/${name}.png`, fullPage: true });
  }
  async function withPage(run: (page: Page) => Promise<void>) {
    state.blocked = true; state.recovery = null; state.online = false; state.summaryBlocked = null;
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
    const page = await context.newPage();
    page.setDefaultTimeout(10_000);
    await page.clock.install();
    try { await run(page); }
    catch (error) { await screenshot(page, "failure"); throw error; }
    finally { await context.close(); }
  }
  async function openPaused(page: Page) {
    const poll = page.waitForResponse((response) => response.url().endsWith("/api/billing/trial-access"));
    await page.goto(`${origin}/dashboard`);
    await page.getByRole("heading", { name: PAUSED }).waitFor();
    await poll;
    // Let the response body and hydration effects settle before advancing time.
    await page.waitForLoadState("networkidle");
  }
  async function tick(page: Page, ms = 30_000) {
    const poll = page.waitForResponse((response) => response.url().endsWith("/api/billing/trial-access"));
    await page.clock.fastForward(ms);
    await poll;
    await page.waitForTimeout(100);
  }
  try {
    for (let attempt = 0; ; attempt++) {
      if (await fetch(origin).then(() => true).catch(() => false)) break;
      if (attempt >= 120 || dashboard.exitCode !== null) throw new Error(output);
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    await t.test("an already-open paused tab recovers without reload and waits for runtime readiness", async () => withPage(async (page) => {
      await openPaused(page);
      await screenshot(page, "paused");
      state.blocked = false; state.recovery = "restart_pending";
      await tick(page, 45_000);
      await screenshot(page, "after-payment-poll");
      await page.getByText(WAITING, { exact: true }).waitFor();
      assert.equal(await page.getByRole("heading", { name: PAUSED }).count(), 0);
      assert.equal(await page.getByText("Your agent is online.", { exact: true }).count(), 0);
      await screenshot(page, "recovered-waiting");
      state.online = true; state.recovery = null;
      await page.clock.fastForward(30_000);
      await page.getByText("Your agent is online.", { exact: true }).waitFor();
      await screenshot(page, "runtime-ready");
      state.blocked = true;
      await tick(page);
      await page.getByRole("heading", { name: PAUSED }).waitFor();
      await screenshot(page, "blocked-again");
    }));
    await t.test("unchanged or cancelled payment stays paused without route refresh loops", async () => withPage(async (page) => {
      await openPaused(page);
      let refreshes = 0;
      page.on("request", (request) => { if (request.headers().rsc === "1") refreshes++; });
      await tick(page); await tick(page); await tick(page);
      assert.equal(refreshes, 0);
      await page.getByRole("heading", { name: PAUSED }).waitFor();
    }));
    await t.test("portal return before a delayed webhook recovers on the next poll", async () => withPage(async (page) => {
      // Local stand-in for the external portal. The return destination is the
      // same helper used by billingPortalDestination; no Stripe requests occur.
      await page.route("https://portal.example.test/**", (route) => route.fulfill({ contentType: "text/html", body: `<a href="${origin}${stripeDashboardOnboardingReturnPath()}">Return to Finite</a>` }));
      await page.goto("https://portal.example.test/session");
      const poll = page.waitForResponse((response) => response.url().endsWith("/api/billing/trial-access"));
      await page.getByRole("link", { name: "Return to Finite" }).click();
      await page.getByRole("heading", { name: PAUSED }).waitFor();
      await poll;
      await page.waitForLoadState("networkidle");
      state.blocked = false; // Owner-stopped runtime: no automatic recovery marker.
      await tick(page);
      await page.getByRole("heading", { name: PAUSED }).waitFor({ state: "hidden" });
      assert.equal(await page.getByText(WAITING, { exact: true }).count(), 0);
      assert.equal(await page.getByText("Your agent is online.", { exact: true }).count(), 0);
      await screenshot(page, "portal-owner-stopped");
    }));
    await t.test("portal return after the webhook renders recovered access immediately", async () => withPage(async (page) => {
      state.blocked = false; state.recovery = "restart_pending";
      await page.goto(`${origin}${stripeDashboardOnboardingReturnPath()}`);
      await page.getByText(WAITING, { exact: true }).waitFor();
      assert.equal(await page.getByRole("heading", { name: PAUSED }).count(), 0);
    }));
    await t.test("the rendered page snapshot wins over a newer layout read, with bounded stale-render retries", async () => withPage(async (page) => {
      state.blocked = false; state.summaryBlocked = true;
      let refreshes = 0;
      page.on("request", (request) => { if (request.headers().rsc === "1") refreshes++; });
      await openPaused(page);
      assert.equal(refreshes, 1, "poll reconciles the stale page even though the layout read is already unblocked");
      await tick(page); await tick(page); await tick(page); await tick(page);
      assert.equal(refreshes, 3, "persistent stale RSC data cannot create an infinite refresh loop");
      // Renewed blocking rearms the observer. The next recovery can converge.
      state.blocked = true;
      await tick(page);
      state.blocked = false; state.summaryBlocked = null;
      await tick(page);
      await page.getByRole("heading", { name: PAUSED }).waitFor({ state: "hidden" });
    }));
    await t.test("a stale recovery render retries and stops once server props acknowledge recovery", async () => withPage(async (page) => {
      await openPaused(page);
      state.summaryBlocked = true; state.blocked = false;
      await tick(page);
      await page.getByRole("heading", { name: PAUSED }).waitFor();
      state.summaryBlocked = null;
      await tick(page);
      await page.getByRole("heading", { name: PAUSED }).waitFor({ state: "hidden" });
      await page.waitForLoadState("networkidle");
      let refreshes = 0;
      page.on("request", (request) => { if (request.headers().rsc === "1") refreshes++; });
      await tick(page); await tick(page);
      assert.equal(refreshes, 0, "acknowledged recovery does not keep refreshing");
    }));
    assert.equal(state.writes, 0, "the access monitor never mutates Core or runtime lifecycle");
  } finally {
    await browser.close();
    dashboard.kill("SIGTERM");
    await once(dashboard, "exit");
    core.closeAllConnections();
    await new Promise<void>((resolve) => core.close(() => resolve()));
  }
});
