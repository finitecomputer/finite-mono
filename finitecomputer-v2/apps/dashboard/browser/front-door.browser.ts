import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import http from "node:http";
import { test } from "node:test";
import { chromium, type Page } from "playwright";

import { chromiumLaunchOptions } from "../scripts/playwright-browser";
import { AGENT_DRAFT_COOKIE, sealAgentOnboardingDraft } from "../src/lib/agent-onboarding";

const BANNER = "Try the new Finite.";
const MOVED = "New agents live in the new Finite";
const COOKIE_PASSWORD = "front-door-fixture-cookie-password-32-chars";

type Scenario = {
  meStatus: number;
  projects: unknown[];
  requests: unknown[];
  migratedAt: string | null;
  /** Legacy Stripe subscription status; null means no subscription. */
  subscription: string | null;
  /** Core still requires payment before a new agent (e.g. a cancelled checkout). */
  requiresBilling: boolean;
};

function legacyProject() {
  return {
    project: { id: "project_legacy", display_name: "Legacy Agent", created_at: "2026-10-01T00:00:00Z", updated_at: "2026-10-01T00:00:00Z" },
    runtime: { id: "runtime_legacy", project_id: "project_legacy", contact_endpoint: "http://127.0.0.1:1", runtime_status: "online", hermes_available: true, created_at: "2026-10-01T00:00:00Z", updated_at: "2026-10-01T00:00:00Z" },
  };
}

async function listen(server: http.Server) {
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return (server.address() as { port: number }).port;
}

test("the front door routes accounts between the legacy dashboard and the new Finite", { timeout: 300_000 }, async (t) => {
  const state: Scenario & { writes: string[]; meReads: number } = { meStatus: 200, projects: [], requests: [], migratedAt: null, subscription: null, requiresBilling: false, writes: [], meReads: 0 };
  const me = () => ({
    email: "owner@example.test", workos_user_id: "user_front_door", projects: state.projects,
    agent_creation_requests: state.requests, migrated_at: state.migratedAt,
  });
  const billing = () => ({
    customer_org: { id: "org_front_door", owner_user_id: "user_front_door", name: "Front door fixture", billing_class: "standard", created_at: "2026-10-01T00:00:00Z", updated_at: "2026-10-01T00:00:00Z" },
    billing_account: state.subscription
      ? { customer_org_id: "org_front_door", stripe_customer_id: "cus_front_door", stripe_subscription_id: "sub_front_door", subscription_status: state.subscription, cancel_at_period_end: false, created_at: "2026-10-01T00:00:00Z", updated_at: "2026-10-01T00:00:00Z" }
      : null,
    agent_creation_entitlement: null, can_create_agent: !state.requiresBilling, requires_billing: state.requiresBilling,
  });
  const core = http.createServer(async (request, response) => {
    if (request.method !== "GET") state.writes.push(`${request.method} ${request.url}`);
    if (request.method === "PUT" && request.url === "/api/core/v1/me/front-door-preference") {
      // The caller's own preference, as Core records it on their row.
      let body = "";
      for await (const chunk of request) body += chunk;
      const { useNewFinite } = JSON.parse(body) as { useNewFinite: boolean };
      state.migratedAt = useNewFinite ? (state.migratedAt ?? new Date().toISOString()) : null;
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify({ use_new_finite: useNewFinite, migrated_at: state.migratedAt }));
      return;
    }
    if (request.url === "/api/core/v1/me") state.meReads++;
    const unavailable = state.meStatus !== 200;
    let data: unknown = request.url === "/api/core/v1/me" ? (unavailable ? null : me())
      : request.url === "/api/core/v1/me/billing" ? billing()
      : request.url === "/api/core/v1/me/dashboard-summary" ? (unavailable ? null : { me: me(), billing: billing(), finite_private_usage: null })
      : null;
    if (request.method === "POST" && request.url === "/api/core/v1/me/agent-creation-requests") {
      // The paid draft's exact creation request, as Core would record it.
      const project = { id: "project_paid", display_name: "Paid Agent", created_at: "2026-10-19T09:00:00Z", updated_at: "2026-10-19T09:00:00Z" };
      const created = { id: "request_paid", customer_org_id: "org_front_door", owner_user_id: "user_front_door", project_id: project.id, idempotency_key: "front-door-paid-key", display_name: project.display_name, runner_class: "kata", status: "launching", agent_runtime_id: null, created_at: project.created_at, updated_at: project.updated_at };
      state.projects = [{ project, runtime: null }];
      state.requests = [created];
      data = { project, request: created, reused: false };
    }
    const status = data ? 200 : unavailable && request.url?.startsWith("/api/core/v1/me") ? state.meStatus : 404;
    response.writeHead(status, { "content-type": "application/json" });
    response.end(JSON.stringify(data ?? { error: status === 404 ? "Unknown fixture route" : "Core is unavailable" }));
  });
  const corePort = await listen(core);
  // Hosted Web Device stand-in: accepts the paid launch's chat bootstrap only.
  const hostedDevice = http.createServer((request, response) => {
    const bootstrap = request.method === "POST" && request.url === "/v1/app/agent-bindings/authorize-bootstrap";
    response.writeHead(bootstrap ? 200 : 404, { "content-type": "application/json" });
    response.end(JSON.stringify(bootstrap ? {} : { error: "Unknown fixture route" }));
  });
  const hostedDevicePort = await listen(hostedDevice);
  // Local stand-in for https://finite.chat; no external request is made.
  const newFinite = http.createServer((request, response) => {
    const signup = request.url?.startsWith("/auth/login");
    response.writeHead(200, { "content-type": "text/html" });
    response.end(`<h1>${signup ? "New Finite sign up" : "New Finite home"}</h1>`);
  });
  const newFinitePort = await listen(newFinite);
  const newFiniteOrigin = `http://127.0.0.1:${newFinitePort}`;
  const reserve = http.createServer();
  const port = await listen(reserve);
  await new Promise<void>((resolve) => reserve.close(() => resolve()));
  const origin = `http://127.0.0.1:${port}`;
  const dashboard = spawn(process.execPath, ["node_modules/next/dist/bin/next", "dev", "--hostname", "127.0.0.1", "--port", String(port)], {
    env: {
      NODE_ENV: "development", PATH: process.env.PATH, HOME: process.env.HOME, TMPDIR: process.env.TMPDIR,
      FC_CORE_BASE_URL: `http://127.0.0.1:${corePort}`,
      FC_CORE_API_TOKEN: "front-door-fixture-token",
      FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
      FC_DASHBOARD_DEV_EMAIL: "owner@example.test",
      FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_front_door",
      FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "front-door-fixture-access-token",
      FC_DASHBOARD_RUNTIME_MODE: "customer", FC_WORKOS_AUTH_ENABLED: "0",
      // Redirect origins come from here, as in production (finite.computer).
      NEXT_PUBLIC_WORKOS_REDIRECT_URI: `${origin}/callback`,
      WORKOS_COOKIE_PASSWORD: COOKIE_PASSWORD,
      FC_HOSTED_WEB_DEVICE_URL: `http://127.0.0.1:${hostedDevicePort}`,
      FINITECHAT_HOSTED_API_TOKEN: "front-door-fixture-hosted-token",
      FC_FRONT_DOOR_ENABLED: "true",
      FC_FRONT_DOOR_TARGET_URL: `${newFiniteOrigin}/`,
      FC_FRONT_DOOR_SIGNUP_URL: `${newFiniteOrigin}/auth/login`,
      NEXT_DIST_DIR: ".next-browser-test",
    },
    stdio: "pipe",
  });
  let output = "";
  dashboard.stdout.on("data", (chunk) => { output += String(chunk); });
  dashboard.stderr.on("data", (chunk) => { output += String(chunk); });
  const browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
  // The layout reads `/me` through the per-session server SWR cache (5s
  // fresh, 30s stale). Let the previous scenario's entry go stale, then make
  // one request so the cache revalidates against the new fixture state.
  async function settleLayoutRead() {
    await sleep(5_200);
    const reads = state.meReads;
    await fetch(`${origin}/dashboard`, { redirect: "manual" });
    for (let attempt = 0; state.meReads === reads && attempt < 100; attempt++) await sleep(100);
    await sleep(300);
  }
  async function withPage(scenario: Partial<Scenario>, run: (page: Page) => Promise<void>, settle = true) {
    Object.assign(state, { meStatus: 200, projects: [], requests: [], migratedAt: null, subscription: null, requiresBilling: false }, scenario);
    if (settle) await settleLayoutRead();
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
    const page = await context.newPage();
    page.setDefaultTimeout(60_000);
    try { await run(page); }
    finally { await context.close(); }
  }
  async function expectNewFinite(page: Page, path: string, heading = "New Finite home") {
    await page.goto(`${origin}${path}`);
    await page.waitForURL((url) => url.origin === newFiniteOrigin);
    // The stand-in page has no web fonts, so assert its text, not visibility.
    assert.equal(await page.locator("h1").textContent(), heading, `${path} landed on ${page.url()}`);
  }
  try {
    for (let attempt = 0; ; attempt++) {
      if (await fetch(origin).then(() => true).catch(() => false)) break;
      if (attempt >= 240 || dashboard.exitCode !== null) throw new Error(output);
      await new Promise((resolve) => setTimeout(resolve, 250));
    }

    // First, before this session has any cached account read: a sign-in
    // while Core is unavailable must never redirect blindly.
    await t.test("Core unavailable at sign-in keeps the account on legacy", async () => withPage({ meStatus: 503 }, async (page) => {
      await page.goto(`${origin}/dashboard`);
      await page.getByRole("heading", { name: "Could not load your account" }).waitFor();
      assert.equal(new URL(page.url()).origin, origin);
      assert.ok(state.meReads > 0, "the layout asked Core and got no answer");
    }, false));

    await t.test("an account without a legacy agent lands on the new Finite, deep links included", async () => withPage({}, async (page) => {
      await expectNewFinite(page, "/dashboard");
      await expectNewFinite(page, "/dashboard/machines/runtime_legacy/chat");
    }));

    await t.test("sign-up goes to the new product's sign-up screen", async () => withPage({}, async (page) => {
      await expectNewFinite(page, "/signup?returnTo=/dashboard", "New Finite sign up");
    }));

    await t.test("routes outside the dashboard pages are never redirected", async () => withPage({}, async (page) => {
      for (const path of ["/healthz", "/api/pwa/manifest"]) {
        const response = await page.request.get(`${origin}${path}`, { maxRedirects: 0 });
        assert.equal(response.status(), 200, `${path} answered ${response.status()}`);
      }
    }));

    await t.test("a legacy agent owner stays, with a session-dismissible banner and no new-agent entry point", async () => withPage({ projects: [legacyProject()] }, async (page) => {
      await page.goto(`${origin}/dashboard`);
      await page.getByRole("heading", { name: "Your agents" }).waitFor();
      assert.equal(new URL(page.url()).origin, origin);
      const banner = page.getByRole("complementary", { name: "The new Finite" });
      await banner.getByText(BANNER).waitFor();
      assert.equal(await banner.getByRole("link", { name: "Open" }).getAttribute("href"), `${newFiniteOrigin}/`);
      assert.equal(await page.getByRole("link", { name: "New agent" }).count(), 0);

      await banner.getByRole("button", { name: "Dismiss for this session" }).click();
      await banner.waitFor({ state: "detached" });
      await page.reload();
      await page.getByRole("heading", { name: "Your agents" }).waitFor();
      await page.waitForLoadState("networkidle");
      assert.equal(await page.getByText(BANNER).count(), 0, "dismissal survives a reload in the same session");
    }));

    await t.test("a fresh session shows the banner again", async () => withPage({ projects: [legacyProject()] }, async (page) => {
      await page.goto(`${origin}/dashboard`);
      await page.getByText(BANNER).waitFor();
    }));

    await t.test("creation entry points explain the move and the POST refuses without touching Core", async () => withPage({ projects: [legacyProject()] }, async (page) => {
      await page.goto(`${origin}/dashboard?new=1`);
      await page.getByRole("heading", { name: MOVED }).waitFor();
      assert.equal(await page.getByLabel("Agent name").count(), 0, "no creation form is offered");
      state.writes = [];
      const response = await page.request.post(`${origin}/dashboard/agent-creation-requests`, {
        form: { displayName: "Second Agent", hostingTier: "standard", idempotencyKey: "front-door-browser-key" },
        maxRedirects: 0,
      });
      assert.equal(response.status(), 303);
      const location = new URL(response.headers().location ?? "", origin);
      assert.match(location.searchParams.get("agentCreationError") ?? "", /new Finite/u);
      assert.deepEqual(state.writes, [], "no Core write happens");
      await page.goto(location.toString());
      await page.getByRole("heading", { name: MOVED }).waitFor();
    }));

    await t.test("a migrated owner goes to the new Finite even with a legacy agent", async () => withPage({ projects: [legacyProject()], migratedAt: "2026-10-19T09:00:00Z" }, async (page) => {
      await expectNewFinite(page, "/dashboard");
    }));

    await t.test("an in-flight creation keeps its owner on legacy", async () => withPage({
      projects: [{ project: { id: "project_new", display_name: "New Agent", created_at: "2026-10-01T00:00:00Z", updated_at: "2026-10-01T00:00:00Z" }, runtime: null }],
      requests: [{ id: "request_launching", project_id: "project_new", display_name: "New Agent", status: "launching", agent_runtime_id: null, created_at: "2026-10-01T00:00:00Z", updated_at: "2026-10-01T00:00:00Z" }],
    }, async (page) => {
      await page.goto(`${origin}/dashboard?creation=request_launching`);
      await page.waitForLoadState("domcontentloaded");
      assert.equal(new URL(page.url()).origin, origin);
      assert.equal(new URL(page.url()).pathname, "/dashboard");
    }));

    await t.test("a legacy owner can opt into the new Finite and come back through the stable way back", async () => withPage({ projects: [legacyProject()] }, async (page) => {
      state.writes = [];
      await page.goto(`${origin}/dashboard`);
      const setting = page.getByRole("region", { name: "Use the new Finite by default" });
      await setting.getByText("Off: signing in to finite.computer opens this dashboard.", { exact: false }).waitFor();
      assert.match((await setting.textContent()) ?? "", /\/dashboard\/front-door/u, "the way back is shown");

      // Enable → the next /dashboard load sends this owner to the new Finite.
      await setting.getByRole("button", { name: "Use the new Finite by default" }).click();
      await page.waitForURL((url) => url.origin === newFiniteOrigin);
      await expectNewFinite(page, "/dashboard");

      // The stable way back opens legacy despite the preference, on the setting.
      await page.goto(`${origin}/dashboard/front-door`);
      await page.waitForURL((url) => url.port === String(port) && url.pathname === "/dashboard");
      const back = page.getByRole("region", { name: "Use the new Finite by default" });
      await back.getByText("On: signing in to finite.computer opens the new Finite.", { exact: false }).waitFor();

      // Disable → stays on legacy, now and on later loads.
      await back.getByRole("button", { name: "Keep using this dashboard" }).click();
      await page.waitForURL((url) => url.port === String(port) && url.pathname === "/dashboard");
      await page.getByRole("region", { name: "Use the new Finite by default" })
        .getByText("Off: signing in to finite.computer opens this dashboard.", { exact: false }).waitFor();
      await page.goto(`${origin}/dashboard`);
      await page.getByRole("heading", { name: "Your agents" }).waitFor();
      assert.equal(new URL(page.url()).port, String(port));
      assert.equal(state.migratedAt, null);
      assert.deepEqual(state.writes, [
        "PUT /api/core/v1/me/front-door-preference",
        "PUT /api/core/v1/me/front-door-preference",
      ], "only the user's own preference changed");
      state.writes = [];
    }));

    await t.test("an agentless account still billed by a legacy subscription stays and can manage billing", async () => withPage({ subscription: "active" }, async (page) => {
      await page.goto(`${origin}/dashboard`);
      await page.getByRole("button", { name: "Manage billing" }).waitFor();
      assert.equal(new URL(page.url()).origin, origin);
      await page.getByRole("heading", { name: MOVED }).waitFor();
      await page.getByText(BANNER).waitFor();
      assert.equal(await page.getByLabel("Agent name").count(), 0, "no creation form is offered");
    }));

    await t.test("a migrated account stays while its legacy subscription can still bill", async () => withPage({ subscription: "past_due", migratedAt: "2026-10-19T09:00:00Z" }, async (page) => {
      await page.goto(`${origin}/dashboard`);
      await page.getByRole("button", { name: "Manage billing" }).waitFor();
      assert.equal(new URL(page.url()).origin, origin);
    }));

    await t.test("once the legacy subscription is canceled, that agentless account goes to the new Finite", async () => withPage({ subscription: "canceled" }, async (page) => {
      await expectNewFinite(page, "/dashboard");
    }));

    async function addSavedCheckout(page: Page) {
      const now = Date.now();
      const sealed = await sealAgentOnboardingDraft({
        version: 1, workosUserId: "user_front_door", displayName: "Paid Agent", profilePictureUrl: null,
        hostingTier: "standard", idempotencyKey: "front-door-paid-key", issuedAtMs: now - 60_000,
        returnMachineId: null, stripeCheckoutStartedAtMs: now - 30_000,
      }, { WORKOS_COOKIE_PASSWORD: COOKIE_PASSWORD });
      await page.context().addCookies([{ name: AGENT_DRAFT_COOKIE, value: sealed, domain: "127.0.0.1", path: "/dashboard" }]);
    }

    await t.test("a cancelled saved checkout explains the move instead of a retry that cannot succeed", async () => withPage({ requiresBilling: true }, async (page) => {
      await addSavedCheckout(page);
      await page.goto(`${origin}/dashboard?new=1&billing=cancelled`);
      await page.getByRole("heading", { name: "Checkout cancelled" }).waitFor();
      await page.getByText("No charge was made.", { exact: true }).waitFor();
      await page.getByRole("heading", { name: MOVED }).waitFor();
      assert.equal(await page.getByRole("link", { name: "Retry agent setup" }).count(), 0);
      assert.equal(await page.getByLabel("Agent name").count(), 0, "no creation form is offered");
    }));

    await t.test("a checkout paid before the flip completes into its own launch progress", async () => withPage({ subscription: "active" }, async (page) => {
      await addSavedCheckout(page);
      state.writes = [];
      // The Stripe success return: the dashboard sends the saved setup
      // through /complete, which creates the exact request and lands on its
      // progress. Nothing on the way may send this new owner away.
      await page.goto(`${origin}/dashboard?new=1&billing=success`);
      await page.waitForURL((url) => url.pathname === "/dashboard" && url.searchParams.get("creation") === "request_paid");
      await page.getByRole("heading", { name: "Starting your agent" }).waitFor();
      // /complete builds its redirect from the request URL when no WorkOS base
      // URL is configured; `next dev` reports that host as localhost.
      assert.equal(new URL(page.url()).port, String(port), `still on the dashboard: ${page.url()}`);
      assert.equal(await page.getByRole("heading", { name: MOVED }).count(), 0, "no 'moved' panel above the paid launch");
      assert.deepEqual(state.writes, ["POST /api/core/v1/me/agent-creation-requests"], "exactly one creation, through /complete");
      state.writes = [];
    }));

    assert.deepEqual(state.writes, [], "routing never mutates Core");
  } catch (error) {
    throw new Error(`${error instanceof Error ? error.stack : String(error)}\n--- dashboard output ---\n${output.slice(-4000)}`);
  } finally {
    await browser.close();
    dashboard.kill("SIGTERM");
    await once(dashboard, "exit");
    for (const server of [core, newFinite, hostedDevice]) {
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
    }
  }
});
