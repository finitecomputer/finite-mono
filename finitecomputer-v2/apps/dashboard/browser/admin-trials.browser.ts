import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { once } from "node:events";
import { copyFile, mkdir, rm } from "node:fs/promises";
import http from "node:http";
import test from "node:test";
import { chromium } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";
import type { TrialCampaign } from "../src/lib/trial-types";

const operatorOrg = "workos_org_trial_fixture";
const token = `fixture.${Buffer.from(JSON.stringify({ org_id: operatorOrg })).toString("base64url")}.signature`;

// Synthetic fixtures only. No Stripe client, credentials, or production endpoints.
test("trial admin dashboard creates campaigns, increases total capacity, and reports unknowns", { timeout: 180_000 }, async (t) => {
  const state = { campaigns: fixtures(), posts: [] as Array<{ path: string; body: Record<string, unknown> }>, fail: false, reject: false };
  const core = http.createServer(async (request, response) => {
    response.setHeader("content-type", "application/json");
    const reply = (data: unknown, status = 200) => { response.statusCode = status; response.end(JSON.stringify(data)); };
    const path = request.url ?? "";
    if (path.includes("trial-campaigns")) {
      assert.equal(request.headers.authorization, `Bearer ${token}`);
      if (state.fail) return reply({ error: "Fixture unavailable" }, 503);
      if (state.reject) return reply({ error: "Operator access revoked" }, 403);
      if (request.method === "GET") return reply(state.campaigns);
      let raw = ""; for await (const chunk of request) raw += chunk;
      const body = JSON.parse(raw); state.posts.push({ path, body });
      if (path.endsWith("/capacity")) {
        const campaign = state.campaigns.find(c => path.includes(c.id))!;
        if (campaign.seatLimit !== body.expectedSeatLimit) return reply({ error: "The signup limit changed. Refresh before increasing it." }, 409);
        campaign.seatsRemaining += Number(body.seatLimit) - campaign.seatLimit;
        campaign.seatLimit = Number(body.seatLimit);
        response.statusCode = 204; return response.end();
      }
      state.campaigns.unshift({ id: "campaign_created", ...body, active: true, reservedSeats: 0, redeemedSeats: 0, seatsRemaining: body.seatLimit, redemptions: [] } as TrialCampaign);
      return reply({ id: "campaign_created", code: "TEST-2345-6789-ABCD" });
    }
    if (path === "/api/core/v1/me") return reply({ email: "admin@example.test", workos_user_id: "user_admin", projects: [], claimable_candidates: [], agent_creation_requests: [] });
    if (path === "/api/core/v1/me/billing") return reply({ customer_org: null, billing_account: null, agent_creation_entitlement: null, can_create_agent: false, requires_billing: false });
    if (path === "/api/core/v1/admin/runtimes" || path === "/api/core/v1/admin/launch-code-batches") return reply([]);
    if (path === "/api/core/v1/finite-private/admin-state") return reply({ profiles: [], grants: [], apiKeys: [], accounts: [] });
    return reply({ error: "fixture route unavailable" }, 404);
  });
  core.listen(0, "127.0.0.1"); await once(core, "listening");
  t.after(() => { core.closeAllConnections(); core.close(); });
  const address = core.address(); assert(address && typeof address !== "string");
  const coreUrl = `http://127.0.0.1:${address.port}`;
  const port = await freePort();
  const tsconfig = `.trial-admin-tsconfig-${process.pid}.json`;
  await copyFile("tsconfig.json", tsconfig);
  t.after(() => rm(tsconfig, { force: true }));
  const dashboard = startDashboard(port, coreUrl, true, tsconfig);
  const output = collectOutput(dashboard);
  t.after(() => stop(dashboard));
  await waitForDashboard(port, output);
  const browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 1440, height: 1400 } });
  const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", route => {
    const url = new URL(route.request().url());
    return ["127.0.0.1", "localhost"].includes(url.hostname) ? route.continue() : route.abort();
  });
  const open = async () => {
    await page.goto(`http://127.0.0.1:${port}/dashboard/admin`);
    await page.getByRole("tab", { name: "Free trials", exact: true }).click();
    await page.getByRole("heading", { name: "Free trials", exact: true }).waitFor();
  };
  await open();
  await page.getByText("9 / 18", { exact: true }).waitFor();
  const workshop = page.getByRole("article", { name: "October workshop" });
  await workshop.getByText("Account attribution and trial states (7)").click();
  await page.getByText("Active trial · access allowed", { exact: true }).waitFor();
  await page.getByText("Trial ended · access blocked", { exact: true }).waitFor();
  assert.equal(await page.getByText("Checkout expired", { exact: true }).count(), 1);
  await page.getByText("Physical runtime capacity", { exact: false }).waitFor();
  const screenshots = process.env.TRIAL_SCREENSHOT_DIR || "/tmp/trial-admin-screenshots";
  await mkdir(screenshots, { recursive: true });
  await page.screenshot({ path: `${screenshots}/desktop.png` });
  await page.getByRole("heading", { name: "Readiness checks" }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${screenshots}/desktop-readiness.png` });
  await page.getByRole("heading", { name: "Free trials", exact: true }).scrollIntoViewIfNeeded();
  await page.setViewportSize({ width: 390, height: 844 });
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), "mobile page must not overflow horizontally");
  await page.screenshot({ path: `${screenshots}/mobile.png` });
  await workshop.getByText("Increase signup limit", { exact: true }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${screenshots}/mobile-capacity.png` });
  await page.getByRole("heading", { name: "Readiness checks" }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${screenshots}/mobile-readiness.png` });
  await page.setViewportSize({ width: 1440, height: 1400 });
  await workshop.getByText("Increase signup limit", { exact: true }).click();
  await workshop.getByLabel("New total signup limit").fill("15");
  await workshop.getByText("10 → 15 total seats · adds 5 seats to this code.").waitFor();
  await workshop.getByRole("button", { name: "Increase total limit" }).click();
  await workshop.getByText("6 / 15", { exact: true }).waitFor();
  await workshop.getByRole("status").filter({ hasText: "Signup limit increased to 15 total seats (+5)." }).waitFor();
  assert.deepEqual(state.posts[0].body, { seatLimit: 15, expectedSeatLimit: 10 });
  assert.equal(state.campaigns.find(c => c.id === "campaign_workshop")!.trialDays, 7);
  await page.getByText("Create new free trial campaign", { exact: true }).click();
  await page.getByLabel("Campaign name", { exact: true }).fill("Extra workshop");
  await page.getByLabel("Total signup limit", { exact: true }).fill("15");
  await page.getByRole("button", { name: "Create campaign and code" }).click();
  await page.getByText("TEST-2345-6789-ABCD", { exact: true }).waitFor();
  await page.getByText("Extra workshop · 15 total seats · 7-day trial", { exact: true }).waitFor();
  await page.getByRole("article", { name: "Extra workshop" }).waitFor();
  assert.deepEqual(state.posts[1].body, { name: "Extra workshop", seatLimit: 15, trialDays: 7 });
  await page.getByText("Create new free trial campaign", { exact: true }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${screenshots}/created.png` });
  // Revoked Core authorization blocks a forged/stale admin form too.
  state.reject = true;
  await page.getByLabel("Campaign name", { exact: true }).fill("Must not create");
  await page.getByRole("button", { name: "Create campaign and code" }).click();
  await page.getByRole("alert").filter({ hasText: "Operator access revoked" }).waitFor();
  assert.equal(state.posts.length, 2);
  state.reject = false;
  await open();
  assert.equal(await page.getByText("TEST-2345-6789-ABCD", { exact: true }).count(), 0, "one-time code must not reappear after navigation");
  // A stale total cannot overwrite another operator's increase.
  await workshop.getByText("Increase signup limit", { exact: true }).click();
  state.campaigns.find(c => c.id === "campaign_workshop")!.seatLimit = 20;
  state.campaigns.find(c => c.id === "campaign_workshop")!.seatsRemaining += 5;
  await workshop.getByLabel("New total signup limit").fill("25");
  await workshop.getByRole("button", { name: "Increase total limit" }).click();
  await workshop.getByRole("alert").filter({ hasText: "signup limit changed" }).waitFor();
  await page.getByRole("button", { name: "Refresh counts" }).click();
  await workshop.getByText("6 / 20", { exact: true }).waitFor();
  state.campaigns = [];
  await open(); await page.getByText("No free trial campaigns yet.").waitFor();
  state.fail = true;
  await open(); await page.getByRole("alert").filter({ hasText: "Trial campaigns are unavailable" }).waitFor();
  assert.equal(await page.getByText("No free trial campaigns yet.").count(), 0, "failure must not look like an empty database");
  assert.equal(await page.getByText("Create new free trial campaign", { exact: true }).count(), 0);
  assert.deepEqual(errors, []);
  assert.equal(await page.locator("[data-nextjs-dialog]").count(), 0);

  // A second real dashboard process has no operator organization or dev admin grant.
  const memberPort = await freePort();
  const member = startDashboard(memberPort, coreUrl, false, tsconfig);
  t.after(() => stop(member));
  await waitForDashboard(memberPort, collectOutput(member));
  const response = await page.goto(`http://127.0.0.1:${memberPort}/dashboard/admin`);
  assert.equal(response?.status(), 404);
  assert.equal(await page.getByText("Create new free trial campaign", { exact: true }).count(), 0);
});

function fixtures(): TrialCampaign[] {
  const seat = (id: string, state: string, status: string | null = null, blocked = false) => ({
    customerOrgId: `org_${id}`, ownerWorkosUserId: `user_${id}`, state,
    redeemedAt: state === "redeemed" ? "2026-10-01T12:00:00Z" : null,
    trialAccess: state === "redeemed" ? { eventName: "October workshop", blocked, subscriptionStatus: status, periodEnd: status === "active" ? "2026-11-01T12:00:00Z" : "2026-10-08T12:00:00Z" } : null,
  });
  return [
    { id: "campaign_workshop", name: "October workshop", seatLimit: 10, trialDays: 7, active: true, reservedSeats: 2, redeemedSeats: 4, seatsRemaining: 4, redemptions: [seat("trial", "redeemed", "trialing"), seat("paid", "redeemed", "active"), seat("ended", "redeemed", "trialing", true), seat("unpaid", "redeemed", "past_due", true), seat("checkout", "reserved"), seat("retry", "reserved"), seat("retry", "expired")] },
    { id: "campaign_full", name: "Design partners", seatLimit: 3, trialDays: 14, active: true, reservedSeats: 3, redeemedSeats: 0, seatsRemaining: 0, redemptions: [seat("d1", "reserved"), seat("d2", "reserved"), seat("d3", "reserved")] },
    { id: "campaign_inactive", name: "Previous event", seatLimit: 5, trialDays: 7, active: false, reservedSeats: 0, redeemedSeats: 0, seatsRemaining: 5, redemptions: [] },
  ];
}

function startDashboard(port: number, coreUrl: string, admin: boolean, tsconfig: string) {
  return spawn(process.execPath, ["node_modules/next/dist/bin/next", "dev", "--hostname", "127.0.0.1", "--port", String(port)], {
    env: { ...process.env, NEXT_TSCONFIG_PATH: tsconfig, FC_CORE_BASE_URL: coreUrl, FC_CORE_API_TOKEN: "fixture_service", FC_WORKOS_AUTH_ENABLED: "0", FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1", FC_DASHBOARD_DEV_EMAIL: admin ? "admin@example.test" : "member@example.test", FC_DASHBOARD_DEV_ADMIN_EMAILS: admin ? "admin@example.test" : "", FC_DASHBOARD_DEV_WORKOS_USER_ID: admin ? "user_admin" : "user_member", FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: admin ? token : "fixture_member", FC_WORKOS_OPERATOR_ORG_ID: operatorOrg, FC_DASHBOARD_TRIALS_ENABLED: "true", NEXT_DIST_DIR: admin ? ".next-browser-test" : ".next-browser-trial-member" }, stdio: "pipe",
  });
}
function collectOutput(child: ChildProcessWithoutNullStreams) {
  let output = ""; for (const pipe of [child.stdout, child.stderr]) pipe.on("data", chunk => { output = (output + chunk).slice(-8000); });
  return () => output;
}
async function waitForDashboard(port: number, output: () => string) {
  const start = Date.now();
  while (Date.now() - start < 90000) {
    try { if ((await fetch(`http://127.0.0.1:${port}/dashboard`)).status < 500) return; } catch {}
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  throw new Error(output());
}
async function stop(child: ChildProcessWithoutNullStreams) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const done = once(child, "exit"); child.kill("SIGTERM");
  const timeout = setTimeout(() => child.kill("SIGKILL"), 3000);
  await done; clearTimeout(timeout);
}
async function freePort() {
  const server = http.createServer(); server.listen(0, "127.0.0.1"); await once(server, "listening");
  const address = server.address(); assert(address && typeof address !== "string");
  server.close(); await once(server, "close"); return address.port;
}
