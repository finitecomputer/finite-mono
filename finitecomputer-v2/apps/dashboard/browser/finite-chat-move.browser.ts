// FIN-463: the real dashboard sends moved people on to finite.chat. Phase 1
// redirects a person once Core says every agent of theirs has moved; phase 2's
// switch redirects every page visit. A local page stands in for finite.chat.
import assert from "node:assert/strict";
import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdir } from "node:fs/promises";
import http from "node:http";
import { test } from "node:test";
import { chromium, type Browser, type Page } from "playwright";

import { chromiumLaunchOptions } from "../scripts/playwright-browser";

const FINITE_CHAT = "finite.chat stands in here";

async function listen(server: http.Server) {
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return (server.address() as { port: number }).port;
}

async function freePort() {
  const reserve = http.createServer();
  const port = await listen(reserve);
  await new Promise<void>((resolve) => reserve.close(() => resolve()));
  return port;
}

async function startDashboard(corePort: number, finiteChat: string, extra: Record<string, string>) {
  const port = await freePort();
  const origin = `http://127.0.0.1:${port}`;
  const dashboard = spawn(process.execPath, ["node_modules/next/dist/bin/next", "dev", "--hostname", "127.0.0.1", "--port", String(port)], {
    env: {
      NODE_ENV: "development", PATH: process.env.PATH, HOME: process.env.HOME, TMPDIR: process.env.TMPDIR,
      FC_CORE_BASE_URL: `http://127.0.0.1:${corePort}`,
      FC_CORE_API_TOKEN: "move-fixture-token",
      FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
      FC_DASHBOARD_DEV_EMAIL: "move@example.test",
      FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_move",
      FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "move-fixture-access-token",
      FC_DASHBOARD_RUNTIME_MODE: "customer", FC_WORKOS_AUTH_ENABLED: "0",
      WORKOS_COOKIE_PASSWORD: "move-fixture-cookie-password-at-least-32-characters",
      FC_FINITE_CHAT_URL: finiteChat,
      NEXT_DIST_DIR: ".next-browser-test",
      ...extra,
    },
    stdio: "pipe",
  });
  let output = "";
  dashboard.stdout?.on("data", (chunk) => { output += String(chunk); });
  dashboard.stderr?.on("data", (chunk) => { output += String(chunk); });
  for (let attempt = 0; ; attempt++) {
    if (await fetch(`${origin}/health`).then(() => true).catch(() => false)) break;
    if (attempt >= 240 || dashboard.exitCode !== null) throw new Error(output);
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  return { origin, dashboard, output: () => output };
}

async function stop(dashboard: ChildProcess) {
  if (dashboard.exitCode !== null) return;
  dashboard.kill("SIGTERM");
  await once(dashboard, "exit");
}

test("moved people land on finite.chat, from the real dashboard", { timeout: 300_000 }, async (t) => {
  const state = { moved: false };
  const me = () => ({
    email: "move@example.test", workos_user_id: "user_move", claimable_candidates: [], agent_creation_requests: [],
    moved_to_finite_chat: state.moved,
    projects: state.moved ? [] : [{
      project: { id: "project_move", display_name: "Moving Agent", created_at: "2026-07-01T00:00:00Z", updated_at: "2026-07-01T00:00:00Z" },
      runtime: { id: "runtime_move", project_id: "project_move", contact_endpoint: "http://127.0.0.1:1", runtime_status: "online", hermes_available: true, created_at: "2026-07-01T00:00:00Z", updated_at: "2026-07-01T00:00:00Z" },
    }],
  });
  const billing = () => ({
    customer_org: { id: "org_move", owner_user_id: "user_move", name: "Move fixture", billing_class: "standard" },
    billing_account: null, agent_creation_entitlement: null, can_create_agent: false, requires_billing: false, trial_access: null,
  });
  const core = http.createServer((request, response) => {
    const data = request.url === "/api/core/v1/me" ? me()
      : request.url === "/api/core/v1/me/billing" ? billing()
      : request.url === "/api/core/v1/me/dashboard-summary" ? { me: me(), billing: billing(), finite_private_usage: null }
      : null;
    response.writeHead(data ? 200 : 404, { "content-type": "application/json" });
    response.end(JSON.stringify(data ?? { error: "Unknown fixture route" }));
  });
  const chat = http.createServer((_request, response) => {
    response.writeHead(200, { "content-type": "text/html" });
    response.end(`<!doctype html><title>finite.chat</title><h1>${FINITE_CHAT}</h1>`);
  });
  const corePort = await listen(core);
  const finiteChat = `http://127.0.0.1:${await listen(chat)}/`;
  const browser: Browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
  const screenshots = process.env.FINITE_CHAT_MOVE_SCREENSHOTS;
  if (screenshots) await mkdir(screenshots, { recursive: true });
  async function withPage(run: (page: Page) => Promise<void>, name: string) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
    const page = await context.newPage();
    page.setDefaultTimeout(60_000);
    try {
      await run(page);
      if (screenshots) await page.screenshot({ path: `${screenshots}/${name}.png`, fullPage: true });
    } finally {
      await context.close();
    }
  }
  const dashboards: ChildProcess[] = [];
  try {
    const phaseOne = await startDashboard(corePort, finiteChat, {});
    dashboards.push(phaseOne.dashboard);

    await t.test("a person with an agent still here keeps the dashboard", () => withPage(async (page) => {
      state.moved = false;
      await page.goto(`${phaseOne.origin}/dashboard`);
      await page.getByText("Moving Agent").first().waitFor();
      assert.ok(page.url().startsWith(`${phaseOne.origin}/dashboard`), page.url());
    }, "phase-1-not-moved"));

    // The layout doesn't re-render on client navigation, so an open tab
    // must recheck on its own (the review's repro: an in-app link).
    await t.test("an open dashboard follows the move on its next navigation", () => withPage(async (page) => {
      state.moved = false;
      // The page checks once on load too. Mark the move only after that check
      // has answered, or it can carry the page off before the click lands.
      const loadCheck = page.waitForResponse((response) => new URL(response.url()).pathname === "/api/finite-chat-move");
      await page.goto(`${phaseOne.origin}/dashboard/skills`);
      const choose = page.getByRole("link", { name: "Choose an agent" });
      await choose.waitFor();
      assert.deepEqual(await (await loadCheck).json(), { redirect: null });
      state.moved = true;
      await choose.click();
      await page.waitForURL(finiteChat, { timeout: 20_000 });
      await page.getByRole("heading", { name: FINITE_CHAT }).waitFor();
    }, "phase-1-moved-while-open"));

    await t.test("a person whose agents have all moved is sent to finite.chat", () => withPage(async (page) => {
      state.moved = true;
      // The dashboard reads `/me` through a short cache; the first visit
      // after the mark may still see the agent.
      for (let attempt = 0; ; attempt++) {
        await page.goto(`${phaseOne.origin}/dashboard`);
        if (page.url() === finiteChat) break;
        if (attempt >= 20) assert.fail(`still at ${page.url()}`);
        await page.waitForTimeout(1_000);
      }
      await page.getByRole("heading", { name: FINITE_CHAT }).waitFor();
      // Any dashboard page, not only its home.
      await page.goto(`${phaseOne.origin}/dashboard/skills`);
      assert.equal(page.url(), finiteChat);
    }, "phase-1-moved"));

    await t.test("without the switch, the front page and sign-in stay", async () => {
      for (const path of ["/", "/login"]) {
        const response = await fetch(`${phaseOne.origin}${path}`, { redirect: "manual" });
        assert.notEqual(response.headers.get("location"), finiteChat, path);
      }
    });
    await stop(phaseOne.dashboard);

    // Phase 2, signed in as an operator whose own agents have moved.
    const phaseTwo = await startDashboard(corePort, finiteChat, {
      FC_REDIRECT_ALL_TO_FINITE_CHAT: "1",
      FC_DASHBOARD_DEV_ADMIN_EMAILS: "move@example.test",
    });
    dashboards.push(phaseTwo.dashboard);
    await t.test("phase 2 sends every page visit to finite.chat", () => withPage(async (page) => {
      state.moved = false;
      for (const path of ["/", "/signup", "/code", "/dashboard"]) {
        await page.goto(`${phaseTwo.origin}${path}`);
        assert.equal(page.url(), finiteChat, path);
      }
      await page.getByRole("heading", { name: FINITE_CHAT }).waitFor();
    }, "phase-2"));

    await t.test("operators keep sign-in and the admin pages", () => withPage(async (page) => {
      state.moved = true;
      const login = await fetch(`${phaseTwo.origin}/login`, { redirect: "manual" });
      assert.notEqual(login.headers.get("location"), finiteChat);
      // Moved, but an operator: neither phase sends them away from admin.
      for (let attempt = 0; attempt < 10; attempt++) {
        await page.goto(`${phaseTwo.origin}/dashboard/admin`);
        await page.waitForTimeout(1_000);
      }
      assert.ok(page.url().startsWith(`${phaseTwo.origin}/dashboard/admin`), page.url());
      assert.equal(await page.getByRole("heading", { name: FINITE_CHAT }).count(), 0);
    }, "phase-2-operator"));

    await t.test("phase 2 keeps health and machine routes answering", async () => {
      const health = await fetch(`${phaseTwo.origin}/health`, { redirect: "manual" });
      assert.equal(health.status, 200);
      for (const path of ["/api/stripe/webhook", "/api/billing/trial-access"]) {
        const response = await fetch(`${phaseTwo.origin}${path}`, { redirect: "manual" });
        assert.notEqual(response.headers.get("location"), finiteChat, path);
      }
      const post = await fetch(`${phaseTwo.origin}/dashboard`, { method: "POST", redirect: "manual" });
      assert.notEqual(post.headers.get("location"), finiteChat);
    });
  } finally {
    await browser.close();
    for (const dashboard of dashboards) await stop(dashboard);
    core.close();
    chat.close();
  }
});
