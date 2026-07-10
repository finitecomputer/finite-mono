import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync } from "node:fs";
import http, { type IncomingMessage, type ServerResponse } from "node:http";
import { once } from "node:events";
import { test } from "node:test";

import { chromium, type Browser } from "playwright";

const CORE_TOKEN = "browser-core-token";

test("admins keep the SaaS dashboard and separate Finite Private controls", { timeout: 120_000 }, async () => {
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
    assert.equal(await machineSwitcher.evaluate((element) => element.tagName), "BUTTON");
    assert.equal((await machineSwitcher.textContent())?.trim(), "New agent");
    await machineSwitcher.click();
    await page
      .getByRole("menuitem", { name: "Legacy Agent", exact: true })
      .waitFor({ state: "visible" });
    await page
      .getByRole("menuitem", { name: "New agent", exact: true })
      .waitFor({ state: "visible" });
    assert.equal(await page.getByRole("heading", { name: "Finite Private" }).count(), 0);

    await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard/admin`);
    await page.getByRole("heading", { name: "Finite Private" }).waitFor({ state: "visible" });
    await page.getByText("fp_grant_1", { exact: true }).waitFor({ state: "visible" });
    await page.getByText("fp_key_1", { exact: true }).waitFor({ state: "visible" });

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
        FC_WORKOS_AUTH_ENABLED: "0",
        NEXT_DIST_DIR: ".next-browser-test",
      },
      stdio: "pipe",
    }
  );
}

async function startFakeCore() {
  const server = http.createServer(async (request, response) => {
    try {
      await handleCoreRequest(request, response);
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
  };
}

async function handleCoreRequest(
  request: IncomingMessage,
  response: ServerResponse
) {
  if (request.headers.authorization !== `Bearer ${CORE_TOKEN}`) {
    writeJson(response, 401, { error: "missing service token" });
    return;
  }

  if (request.method === "GET" && request.url === "/api/core/v1/finite-private/admin-state") {
    writeJson(response, 200, finitePrivateAdminState());
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
        billing_class: "off2026",
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

function legacyProject() {
  return {
    project: {
      id: "project_legacy",
      customer_org_id: "org_admin",
      owner_user_id: "user_admin",
      display_name: "Legacy Agent",
      import_candidate_id: "import_legacy",
      created_at: "2026-05-28T12:00:00Z",
      updated_at: "2026-05-28T12:01:00Z",
    },
    runtime: {
      id: "runtime_legacy",
      project_id: "project_legacy",
      source_host_id: "box1",
      source_machine_id: "legacy-agent",
      source_import_key: "box1:legacy-agent",
      runtime_artifact_id: "runtime_artifact_legacy",
      state_schema_version: "runtime-state-v1",
      host_facts: {
        display_name: "Legacy Agent",
        hostname: "legacy-agent.finite.computer",
        runtime_host: "box1",
        runtime_status: "online",
        hermes_available: true,
        published_app_urls: [],
      },
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
