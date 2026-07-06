import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync } from "node:fs";
import http, { type IncomingMessage, type ServerResponse } from "node:http";
import { once } from "node:events";
import { test } from "node:test";

import { chromium, type Browser, type Page } from "playwright";

const CORE_TOKEN = "browser-core-token";

type AgentCreationRequest = {
  id: string;
  project_id: string;
  display_name: string;
  status: "requested" | "launching" | "running" | "failed" | "cancelled";
  agent_runtime_id: string | null;
  failure_message: string | null;
  created_at: string;
  updated_at: string;
};

type VisibleProject = {
  project: {
    id: string;
    customer_org_id: string;
    owner_user_id: string;
    display_name: string;
    import_candidate_id: string | null;
    created_at: string;
    updated_at: string;
  };
  runtime: null | {
    id: string;
    project_id: string;
      source_host_id: string;
      source_machine_id: string;
      source_import_key: string;
      runtime_artifact_id: string | null;
      state_schema_version: string | null;
      host_facts: {
        display_name: string;
        hostname: string;
      runtime_host: string;
      runtime_status: "online" | "offline" | "stale" | "unknown";
      hermes_available: boolean;
      published_app_urls: string[];
    };
    created_at: string;
    updated_at: string;
  };
};

type CoreState = {
  projects: VisibleProject[];
  requests: AgentCreationRequest[];
  creationPosts: unknown[];
  cancelPosts: string[];
  createDelayMs: number;
};

test("dashboard agent creation browser states", async () => {
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

    core.reset();
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await page.getByLabel("Agent name").fill("Oslo Bot");
      await page.getByLabel("Launch code").fill("off2026");
      await page.getByRole("button", { name: "Create agent" }).click();
      await waitFor(
        () => core.state.creationPosts.length === 1,
        5_000,
        async () => `agent creation POST was not sent\n${await pageText(page)}`
      );
      const post = core.state.creationPosts[0] as Record<string, unknown>;
      assert.equal(post.displayName, "Oslo Bot");
      assert.equal(post.launchCode, "off2026");
      assert.match(String(post.idempotencyKey), /.+/);
      await expectVisibleText(page, "Creating your agent");
    });

    core.reset({ createDelayMs: 500 });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await page.getByLabel("Agent name").fill("Double Submit Bot");
      await page.getByLabel("Launch code").fill("off2026");
      await page.getByRole("button", { name: "Create agent" }).dblclick();
      await waitFor(() => core.state.creationPosts.length === 1);
      await new Promise((resolve) => setTimeout(resolve, 700));
      assert.equal(core.state.creationPosts.length, 1);
    });

    core.reset({
      requests: [
        agentCreationRequest({
          id: "agent_request_waiting",
          projectId: "project_waiting",
          displayName: "Queued Oslo Bot",
          status: "requested",
          createdAt: "2026-05-28T12:00:00Z",
        }),
      ],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await expectVisibleText(page, "Waiting for runner capacity");
      await expectVisibleText(
        page,
        "All runner hosts are busy. Your agent is still queued and will start automatically when capacity opens."
      );
    });

    core.reset({
      requests: [
        agentCreationRequest({
          id: "agent_request_failed",
          projectId: "project_failed",
          displayName: "Failed Oslo Bot",
          status: "failed",
          failureMessage: "Runner capacity exhausted",
        }),
      ],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await expectVisibleText(page, "Agent creation needs a retry");
      await expectVisibleText(page, "Runner capacity exhausted");
      await page.getByRole("button", { name: "Start over" }).click();
      await waitFor(() => core.state.cancelPosts.includes("agent_request_failed"));
    });

    core.reset({
      projects: [visibleProject("project_running", "Completed Oslo Bot")],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await page.waitForURL(/\/dashboard\/machines\/completed-oslo-bot$/u);
      await expectVisibleText(page, "Completed Oslo Bot");
      const main = page.getByRole("main");
      await expectVisibleText(page, "Finite Chat");
      await expectVisibleText(page, "Runtime recovery");
      await expectVisibleText(page, "Runtime facts");
      await main.getByRole("button", { name: "Restart agent" }).waitFor({ state: "visible" });
      await main.getByRole("button", { name: "Recover chat" }).waitFor({ state: "visible" });
      await main.getByRole("button", { name: "Stop" }).waitFor({ state: "visible" });
      await main.getByRole("button", { name: "Destroy" }).waitFor({ state: "visible" });
      assert.equal(await page.getByText("Connections", { exact: true }).count(), 0);
      assert.equal(await page.getByText("OpenRouter", { exact: true }).count(), 0);
    });
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
    "npm",
    ["run", "dev", "--", "--hostname", "127.0.0.1", "--port", String(port)],
    {
      cwd: process.cwd(),
      env: {
        ...process.env,
        FC_CORE_API_TOKEN: CORE_TOKEN,
        FC_CORE_BASE_URL: coreUrl,
        FC_DASHBOARD_DEV_EMAIL: "browser@finite.vip",
        FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_browser",
        FC_WORKOS_AUTH_ENABLED: "0",
      },
      stdio: "pipe",
    }
  );
}

async function startFakeCore() {
  let state = emptyCoreState();
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
    get state() {
      return state;
    },
    reset(next: Partial<CoreState> = {}) {
      state = { ...emptyCoreState(), ...next };
    },
  };
}

async function handleCoreRequest(
  request: IncomingMessage,
  response: ServerResponse,
  state: CoreState
) {
  if (request.headers.authorization !== `Bearer ${CORE_TOKEN}`) {
    writeJson(response, 401, { error: "missing service token" });
    return;
  }

  if (request.method === "GET" && request.url === "/api/core/v1/me") {
    writeJson(response, 200, {
      email: "browser@finite.vip",
      workos_user_id: "user_browser",
      claimable_candidates: [],
      projects: state.projects,
      agent_creation_requests: state.requests,
    });
    return;
  }

  if (request.method === "POST" && request.url === "/api/core/v1/me/agent-creation-requests") {
    const body = await readJson(request);
    state.creationPosts.push(body);
    if (state.createDelayMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, state.createDelayMs));
    }
    const projectId = `project_${state.creationPosts.length}`;
    const requestRecord = agentCreationRequest({
      id: `agent_request_${state.creationPosts.length}`,
      projectId,
      displayName: String(body.displayName ?? "Oslo Bot"),
      status: "requested",
      createdAt: new Date().toISOString(),
    });
    state.requests = [requestRecord];
    writeJson(response, 200, {
      reused: false,
      project: {
        id: projectId,
        customer_org_id: "org_browser",
        owner_user_id: "user_browser",
        display_name: requestRecord.display_name,
        import_candidate_id: null,
        created_at: requestRecord.created_at,
        updated_at: requestRecord.updated_at,
      },
      request: {
        id: requestRecord.id,
        customer_org_id: "org_browser",
        owner_user_id: "user_browser",
        project_id: requestRecord.project_id,
        idempotency_key: String(body.idempotencyKey ?? ""),
        display_name: requestRecord.display_name,
        status: requestRecord.status,
        requested_launch_code: String(body.launchCode ?? ""),
        agent_runtime_id: null,
        created_at: requestRecord.created_at,
        updated_at: requestRecord.updated_at,
      },
    });
    return;
  }

  const cancelMatch = request.url?.match(/^\/api\/core\/v1\/agent-creation-requests\/([^/]+)\/cancel$/u);
  if (request.method === "POST" && cancelMatch?.[1]) {
    const requestId = decodeURIComponent(cancelMatch[1]);
    state.cancelPosts.push(requestId);
    state.requests = state.requests.filter((candidate) => candidate.id !== requestId);
    writeJson(response, 200, agentCreationRequest({ id: requestId, projectId: "cancelled", status: "cancelled" }));
    return;
  }

  writeJson(response, 404, { error: "not found" });
}

async function withSignedInPage(
  browser: Browser,
  dashboardPort: number,
  fn: (page: Page) => Promise<void>
) {
  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    await fn(page);
  } finally {
    await context.close();
  }
}

function emptyCoreState(): CoreState {
  return {
    projects: [],
    requests: [],
    creationPosts: [],
    cancelPosts: [],
    createDelayMs: 0,
  };
}

function agentCreationRequest({
  id,
  projectId,
  displayName = "Oslo Bot",
  status,
  failureMessage = null,
  createdAt = new Date().toISOString(),
}: {
  id: string;
  projectId: string;
  displayName?: string;
  status: AgentCreationRequest["status"];
  failureMessage?: string | null;
  createdAt?: string;
}): AgentCreationRequest {
  return {
    id,
    project_id: projectId,
    display_name: displayName,
    status,
    agent_runtime_id: null,
    failure_message: failureMessage,
    created_at: createdAt,
    updated_at: createdAt,
  };
}

function visibleProject(projectId: string, displayName: string): VisibleProject {
  return {
    project: {
      id: projectId,
      customer_org_id: "org_browser",
      owner_user_id: "user_browser",
      display_name: displayName,
      import_candidate_id: null,
      created_at: "2026-05-28T12:00:00Z",
      updated_at: "2026-05-28T12:01:00Z",
    },
    runtime: {
      id: "runtime_running",
      project_id: projectId,
      source_host_id: "oslo",
      source_machine_id: "completed-oslo-bot",
      source_import_key: "oslo:completed-oslo-bot",
      runtime_artifact_id: "runtime_artifact_1",
      state_schema_version: "runtime-state-v1",
      host_facts: {
        display_name: "Completed Oslo Bot",
        hostname: "completed-oslo-bot.finite.computer",
        runtime_host: "oslo",
        runtime_status: "online",
        hermes_available: true,
        published_app_urls: [],
      },
      created_at: "2026-05-28T12:00:00Z",
      updated_at: "2026-05-28T12:01:00Z",
    },
  };
}

async function readJson(request: IncomingMessage) {
  const chunks: Buffer[] = [];
  for await (const chunk of request) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  }
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
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

async function expectVisibleText(page: Page, text: string) {
  await page.getByText(text, { exact: true }).waitFor({ state: "visible", timeout: 15_000 });
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

async function pageText(page: Page) {
  return (await page.locator("body").innerText({ timeout: 1_000 }).catch((error) => String(error))).slice(0, 4_000);
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
