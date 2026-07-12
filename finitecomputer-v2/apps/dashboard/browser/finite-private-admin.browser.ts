import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync } from "node:fs";
import http, { type IncomingMessage, type ServerResponse } from "node:http";
import { once } from "node:events";
import { test } from "node:test";

import { chromium, type Browser } from "playwright";

const CORE_TOKEN = "browser-core-token";
const OPERATOR_ORG_ID = "workos_org_browser_operator";
const ADMIN_ACCESS_TOKEN = [
  "browser-header",
  Buffer.from(JSON.stringify({ org_id: OPERATOR_ORG_ID })).toString("base64url"),
  "browser-signature",
].join(".");

test("admins issue Standard or Confidential Launch Codes", { timeout: 120_000 }, async () => {
  const core = await startFakeCore();
  const dashboardPort = await freePort();
  const dashboard = startDashboard(dashboardPort, core.url);
  const dashboardOutput = collectOutput(dashboard);
  let browser: Browser | null = null;

  try {
    await waitForDashboard(dashboardPort, dashboardOutput);
    browser = await chromium.launch({
      headless: true,
      ...chromeExecutable(),
    });
    const context = await browser.newContext();
    const page = await context.newPage();

    await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard?new=1`);
    await page
      .getByRole("link", { name: "Agent", exact: true })
      .waitFor({ state: "visible" });
    await page.getByLabel("Agent name").waitFor({ state: "visible" });
    const machineSwitcher = page.locator(".ocean-machine-switcher__button");
    await machineSwitcher.waitFor({ state: "visible" });
    assert.equal(await machineSwitcher.evaluate((element) => element.tagName), "A");
    assert.equal((await machineSwitcher.textContent())?.trim(), "New agent");
    const newAgentUrl = new URL(
      (await machineSwitcher.getAttribute("href")) ?? "",
      page.url()
    );
    assert.equal(newAgentUrl.pathname, "/dashboard");
    assert.equal(newAgentUrl.searchParams.get("new"), "1");
    assert.equal(await page.getByText("Legacy Agent", { exact: true }).count(), 0);
    assert.equal(await page.getByRole("heading", { name: "Finite Private" }).count(), 0);

    await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard/admin`);
    await page.getByRole("heading", { name: "Finite Private" }).waitFor({ state: "visible" });
    await page.getByText("fp_grant_1", { exact: true }).waitFor({ state: "visible" });
    await page.getByText("fp_key_1", { exact: true }).waitFor({ state: "visible" });
    await page.getByText("Legacy Standard batch", { exact: true }).waitFor({ state: "visible" });
    await page.getByText("Confidential batch", { exact: true }).waitFor({ state: "visible" });
    await page.getByText("Standard batch details", { exact: true }).waitFor({ state: "visible" });
    await page
      .getByText("Confidential batch details", { exact: true })
      .waitFor({ state: "visible" });

    await page.getByLabel("Batch name").fill("Browser default batch");
    await page.getByLabel("Exact code count").fill("1");
    await page.getByLabel("Expiry (hours)").fill("24");
    assert.equal(await page.getByLabel("Hosting tier").textContent(), "Standard");
    await page.getByRole("button", { name: "Issue codes" }).click();
    await page.getByLabel("Issued Launch Codes").waitFor({ state: "visible" });
    await waitFor(() => core.state.issuancePosts.length === 1);
    assert.equal(core.state.issuancePosts[0]?.hostingTier, "standard");
    await page.getByText(/Browser default batch · Standard · expires/u).waitFor({ state: "visible" });

    await page.getByLabel("Batch name").fill("Browser confidential batch");
    await page.getByLabel("Hosting tier").click();
    await page.getByRole("option", { name: "Confidential" }).click();
    await page.getByRole("button", { name: "Issue codes" }).click();
    await waitFor(() => core.state.issuancePosts.length === 2);
    assert.equal(core.state.issuancePosts[1]?.hostingTier, "confidential");
    await page.getByText(/Browser confidential batch · Confidential · expires/u).waitFor({
      state: "visible",
    });

    await context.close();
  } finally {
    await browser?.close().catch(() => {});
    dashboard.kill("SIGTERM");
    core.server.close();
    await Promise.race([
      once(dashboard, "exit"),
      new Promise((resolve) => setTimeout(resolve, 2_000)),
    ]);
  }
});

function startDashboard(port: number, coreUrl: string) {
  return spawn(
    process.execPath,
    ["node_modules/next/dist/bin/next", "dev", "--hostname", "127.0.0.1", "--port", String(port)],
    {
      cwd: process.cwd(),
      env: {
        ...process.env,
        FC_CORE_API_TOKEN: CORE_TOKEN,
        FC_CORE_BASE_URL: coreUrl,
        FC_DASHBOARD_DEV_ADMIN_EMAILS: "admin@finite.vip",
        FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
        FC_DASHBOARD_DEV_EMAIL: "admin@finite.vip",
        FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_admin",
        FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: ADMIN_ACCESS_TOKEN,
        FC_WORKOS_OPERATOR_ORG_ID: OPERATOR_ORG_ID,
        FC_WORKOS_AUTH_ENABLED: "0",
        NEXT_DIST_DIR: ".next-browser-test",
      },
      stdio: "pipe",
    }
  );
}

async function startFakeCore() {
  const state = {
    issuancePosts: [] as Array<Record<string, unknown>>,
  };
  const server = http.createServer(async (request, response) => {
    try {
      await handleCoreRequest(request, response, state);
    } catch (error) {
      response.writeHead(500, { "content-type": "application/json" });
      response.end(JSON.stringify({ error: String(error) }));
    }
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");

  return {
    server,
    url: `http://127.0.0.1:${address.port}`,
    state,
  };
}

async function handleCoreRequest(
  request: IncomingMessage,
  response: ServerResponse,
  state: { issuancePosts: Array<Record<string, unknown>> }
) {
  if (
    request.headers.authorization !== `Bearer ${CORE_TOKEN}` &&
    request.headers.authorization !== `Bearer ${ADMIN_ACCESS_TOKEN}`
  ) {
    writeJson(response, 401, { error: "missing service token" });
    return;
  }

  if (request.method === "GET" && request.url === "/api/core/v1/finite-private/admin-state") {
    writeJson(response, 200, finitePrivateAdminState());
    return;
  }

  if (request.method === "GET" && request.url === "/api/core/v1/admin/runtimes") {
    writeJson(response, 200, []);
    return;
  }

  if (request.method === "GET" && request.url === "/api/core/v1/admin/launch-code-batches") {
    writeJson(response, 200, launchCodeBatches());
    return;
  }

  if (request.method === "POST" && request.url === "/api/core/v1/admin/launch-code-batches") {
    const body = JSON.parse(await readBody(request)) as Record<string, unknown>;
    state.issuancePosts.push(body);
    const index = state.issuancePosts.length;
    const codeCount = Number(body.codeCount);
    writeJson(response, 200, {
      batch: {
        id: `launch_batch_browser_${index}`,
        name: String(body.name),
        hosting_tier: String(body.hostingTier),
        code_count: codeCount,
        expires_at: "2026-07-18T12:00:00Z",
        revoked_at: null,
        revoked_by_workos_user_id: null,
        created_by_workos_user_id: "user_admin",
        created_at: "2026-07-11T12:00:00Z",
      },
      codes: Array.from({ length: codeCount }, (_, codeIndex) => ({
        id: `launch_code_browser_${index}_${codeIndex}`,
        code: `finite_browser_${index}_${codeIndex}`,
      })),
    });
    return;
  }

  if (request.method === "GET" && request.url === "/api/core/v1/me") {
    writeJson(response, 200, {
      email: "admin@finite.vip",
      workos_user_id: "user_admin",
      claimable_candidates: [],
      projects: [legacyProject()],
      agent_creation_requests: [],
    });
    return;
  }

  if (request.method === "GET" && request.url === "/api/core/v1/me/billing") {
    writeJson(response, 200, {
      customer_org: {
        id: "org_admin",
        owner_user_id: "user_admin",
        name: "Admin",
        billing_class: "sponsored",
        created_at: "2026-05-28T12:00:00Z",
        updated_at: "2026-05-28T12:01:00Z",
      },
      billing_account: null,
      agent_creation_entitlement: null,
      can_create_agent: false,
      requires_billing: true,
    });
    return;
  }

  writeJson(response, 404, { error: "not found" });
}

function launchCodeBatches() {
  return [
    {
      batch: {
        id: "launch_batch_legacy",
        name: "Legacy Standard batch",
        code_count: 1,
        expires_at: "2026-07-18T12:00:00Z",
        revoked_at: null,
        created_by_workos_user_id: "user_admin",
        created_at: "2026-07-11T12:00:00Z",
      },
      codes: [{ id: "launch_code_legacy", redeemed_at: null }],
    },
    {
      batch: {
        id: "launch_batch_confidential",
        name: "Confidential batch",
        hosting_tier: "confidential",
        code_count: 2,
        expires_at: "2026-07-18T12:00:00Z",
        revoked_at: null,
        created_by_workos_user_id: "user_admin",
        created_at: "2026-07-11T12:00:00Z",
      },
      codes: [
        { id: "launch_code_confidential_1", redeemed_at: null },
        { id: "launch_code_confidential_2", redeemed_at: "2026-07-11T13:00:00Z" },
      ],
    },
  ];
}

async function readBody(request: IncomingMessage) {
  let body = "";
  for await (const chunk of request) {
    body += chunk.toString();
  }
  return body;
}

function legacyProject() {
  return {
    project: {
      id: "project_legacy",
      display_name: "Legacy Agent",
      created_at: "2026-05-28T12:00:00Z",
      updated_at: "2026-05-28T12:01:00Z",
    },
    runtime: {
      id: "runtime_legacy",
      project_id: "project_legacy",
      contact_endpoint: null,
      runtime_status: "online",
      hermes_available: true,
      created_at: "2026-05-28T12:00:00Z",
      updated_at: "2026-05-28T12:01:00Z",
    },
  };
}

function finitePrivateAdminState() {
  return {
    grants: [
      {
        id: "fp_grant_1",
        user_id: "user_private",
        limit_profile_id: "finite-private-generous",
        status: "active",
        current_window_started_at: "2026-05-28T12:00:00Z",
        current_window_used_units: 84,
        created_at: "2026-05-28T12:00:00Z",
        updated_at: "2026-05-28T12:01:00Z",
      },
    ],
    apiKeys: [
      {
        id: "fp_key_1",
        grant_id: "fp_grant_1",
        project_id: "project_private",
        agent_runtime_id: "runtime_private",
        key_hash: "hash-only",
        status: "active",
        created_at: "2026-05-28T12:00:00Z",
        updated_at: "2026-05-28T12:01:00Z",
      },
    ],
    adminAuditEvents: [
      {
        id: "fp_event_1",
        action: "finite_private.grant.approve",
        target_type: "grant",
        target_id: "fp_grant_1",
        grant_id: "fp_grant_1",
        api_key_id: null,
        actor: "core-service",
        metadata: {},
        created_at: "2026-05-28T12:00:00Z",
      },
    ],
  };
}

function writeJson(response: ServerResponse, status: number, body: unknown) {
  response.writeHead(status, { "content-type": "application/json" });
  response.end(JSON.stringify(body));
}

async function waitForDashboard(port: number, output: () => string) {
  await waitFor(async () => {
    const response = await fetch(`http://127.0.0.1:${port}/`, { redirect: "manual" }).catch(() => null);
    return Boolean(response && response.status < 500);
  }, 30_000, () => `dashboard did not become ready\n${output()}`);
}

async function waitFor(
  condition: () => boolean | Promise<boolean>,
  timeoutMs = 5_000,
  timeoutMessage: () => string | Promise<string> = () => "timed out waiting for condition"
) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    if (await condition()) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(await timeoutMessage());
}

function collectOutput(process: ChildProcessWithoutNullStreams) {
  let output = "";
  process.stdout.on("data", (chunk) => {
    output = `${output}${chunk.toString()}`.slice(-8_000);
  });
  process.stderr.on("data", (chunk) => {
    output = `${output}${chunk.toString()}`.slice(-8_000);
  });
  return () => output;
}

async function freePort() {
  const server = http.createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");
  const { port } = address;
  server.close();
  await once(server, "close");
  return port;
}

function chromeExecutable() {
  for (const executablePath of [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
  ]) {
    if (existsSync(executablePath)) {
      return { executablePath };
    }
  }
  return { channel: "chrome" as const };
}
